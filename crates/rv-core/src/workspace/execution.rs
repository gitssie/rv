//! Durable execution ownership, independent from task execution history.
use super::*;
use crate::{ConnectionId, StoreError};
use rusqlite::{OptionalExtension, params};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExecutionMode {
    #[default]
    Rv,
    ToPhone,
    Phone,
}
#[derive(Clone, Debug)]
pub struct AppExecution {
    pub mode: ExecutionMode,
    pub device_id: Option<ConnectionId>,
    pub operation_id: String,
    pub state_sha256: Option<String>,
}
impl Default for AppExecution {
    fn default() -> Self {
        Self {
            mode: ExecutionMode::Rv,
            device_id: None,
            operation_id: String::new(),
            state_sha256: None,
        }
    }
}

// State handoff keeps its archive ownership rule; HTTP/tasks do not use it.
pub(crate) fn require_rv_execution(db: &rusqlite::Connection, app_id: &str) -> Result<(), StoreError> {
    let blocked: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM app_execution WHERE app_id=?1 AND mode<>'rv')", [app_id], |r|r.get(0))?;
    if blocked { return Err(StoreError::Invalid("账号正在交给手机或已由手机接管".into())); }
    Ok(())
}
impl WorkspaceStore {
    pub fn execution(&self, id: &AppId) -> Result<AppExecution, StoreError> {
        self.load(id)?;
        let row: Option<(String, Option<String>, String, Option<String>)> = self.db.query_row(
            "SELECT mode,device_id,operation_id,state_sha256 FROM app_execution WHERE app_id=?1", [id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).optional()?;
        let Some((mode, device, operation_id, state_sha256)) = row else {
            return Ok(AppExecution::default());
        };
        Ok(AppExecution {
            mode: match mode.as_str() {
                "rv" => ExecutionMode::Rv,
                "to_phone" => ExecutionMode::ToPhone,
                "phone" => ExecutionMode::Phone,
                _ => return Err(StoreError::Invalid("账号执行归属无效".into())),
            },
            device_id: device
                .map(|value| {
                    uuid::Uuid::parse_str(&value)
                        .map(ConnectionId)
                        .map_err(|_| StoreError::Invalid("接管设备无效".into()))
                })
                .transpose()?,
            operation_id,
            state_sha256,
        })
    }
    pub fn begin_phone_handoff(
        &mut self,
        id: &AppId,
        device: ConnectionId,
    ) -> Result<AppExecution, StoreError> {
        self.db.execute_batch("SAVEPOINT begin_phone_handoff")?;
        let checkpoint = self.changes.checkpoint();
        let result = (|| {
            let current = self.execution(id)?;
            if current.mode == ExecutionMode::Phone {
                return Err(StoreError::Invalid(
                    "账号已由手机接管，不能用 RV 的旧备份覆盖手机".into(),
                ));
            }
            if current.mode == ExecutionMode::ToPhone && current.device_id != Some(device) {
                return Err(StoreError::Invalid("请先完成原目标手机的交接".into()));
            }
            if !self.backups(id)?.iter().any(|link| {
                link.device_id == device && link.backup_id == format!("snapshot:{}", id.as_str())
            }) {
                return Err(StoreError::Invalid("目标手机没有对应 IDFV 的备份".into()));
            }
            if self.app_state_info(id)?.is_none() || self.auth(id)?.is_none() {
                return Err(StoreError::Invalid("尚未读取完整账号备份".into()));
            }
            let timestamp = models::now();
            if current.mode == ExecutionMode::Rv {
                self.db.execute("INSERT INTO app_execution(app_id,mode,device_id,operation_id,updated_at) VALUES(?1,'to_phone',?2,?3,?4) ON CONFLICT(app_id) DO UPDATE SET mode='to_phone',device_id=excluded.device_id,operation_id=excluded.operation_id,state_sha256=NULL,updated_at=excluded.updated_at",params![id.as_str(),device.to_string(),uuid::Uuid::new_v4().to_string(),timestamp])?;
            }
            self.db.execute("UPDATE tasks SET enabled=0,status=CASE WHEN status='pending' THEN 'cancelled' ELSE status END,next_run_at=NULL,updated_at=?2 WHERE app_id=?1 AND (enabled<>0 OR next_run_at IS NOT NULL)",params![id.as_str(),timestamp])?;
            self.execution(id)
        })();
        match result {
            Ok(value) => {
                self.db.execute_batch("RELEASE begin_phone_handoff")?;
                self.publish_changes();
                Ok(value)
            }
            Err(error) => {
                self.db.execute_batch(
                    "ROLLBACK TO begin_phone_handoff; RELEASE begin_phone_handoff",
                )?;
                self.changes.restore(checkpoint);
                Err(error)
            }
        }
    }
    /// Called only after the persistent fence. Running work commits its latest
    /// tokens before becoming terminal; uncertain writes require reconciliation.
    pub fn prepare_phone_handoff(
        &mut self,
        id: &AppId,
        operation: &str,
    ) -> Result<ShadowAppState, StoreError> {
        let execution = self.execution(id)?;
        if execution.mode != ExecutionMode::ToPhone || execution.operation_id != operation {
            return Err(StoreError::Invalid("账号交接已改变".into()));
        }
        if self.has_running_task(id)? {
            return Err(StoreError::Conflict);
        }
        let unknown: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE app_id=?1 AND status='unknown' AND kind IN ('refresh','chat_send','chat_read'))",[id.as_str()],|row|row.get(0))?;
        if unknown {
            return Err(StoreError::Invalid(
                "账号存在结果待确认的写操作，请先核对，暂不发送备份".into(),
            ));
        }
        let state = self.prepare_state_upload(id)?;
        let sha = state_sha256(state.archive());
        if execution
            .state_sha256
            .as_ref()
            .is_some_and(|saved| *saved != sha)
        {
            return Err(StoreError::Invalid(
                "交接中的本地 state 已改变，不能覆盖原交接版本".into(),
            ));
        }
        self.db.execute("UPDATE app_execution SET state_sha256=?2,updated_at=?3 WHERE app_id=?1 AND mode='to_phone' AND operation_id=?4",params![id.as_str(),sha,models::now(),operation])?;
        self.publish_changes();
        Ok(state)
    }
    pub fn validate_rv_takeover(
        &self,
        id: &AppId,
        device: ConnectionId,
        operation: &str,
    ) -> Result<(), StoreError> {
        let execution = self.execution(id)?;
        if execution.operation_id != operation || execution.mode == ExecutionMode::ToPhone {
            return Err(StoreError::Invalid("账号交接已改变".into()));
        }
        match self.token_source(id)? {
            TokenSource::Phone(Some(source)) if source == device => {}
            TokenSource::Rv if execution.mode == ExecutionMode::Phone && execution.device_id == Some(device) => {}
            _ => return Err(StoreError::Invalid("请选择提供账号凭据的手机备份".into())),
        }
        if !self.backups(id)?.iter().any(|link| {
            link.device_id == device && link.backup_id == format!("snapshot:{}", id.as_str())
        }) {
            return Err(StoreError::Invalid("手机备份与账号身份不匹配".into()));
        }
        Ok(())
    }
    /// Explicitly adopt the phone's credentials. Ordinary imports retain the
    /// separate protection against historical backups overwriting RV rotations.
    pub fn complete_rv_takeover(
        &mut self,
        id: &AppId,
        device: ConnectionId,
        operation: &str,
        state: &ShadowAppState,
    ) -> Result<(), StoreError> {
        let own_transaction = self.db.is_autocommit();
        self.db.execute_batch(if own_transaction { "BEGIN IMMEDIATE" } else { "SAVEPOINT rv_takeover" })?;
        let checkpoint = self.changes.checkpoint();
        let result = (|| {
            self.validate_rv_takeover(id, device, operation)?;
            if state.request_auth()?.refresh_token.as_deref().is_none_or(str::is_empty) {
                return Err(StoreError::Invalid("手机备份缺少 refresh token，未交接给 RV".into()));
            }
            self.import_app_state_inner(id, Some(device), state, false)?;
            let timestamp = models::now();
            // Explicit handover adopts this phone backup, unlike catalogue refresh
            // or ordinary state synchronisation. Publish source and credentials together.
            self.db.execute("DELETE FROM app_token_sources WHERE app_id=?1", [id.as_str()])?;
            self.db.execute("INSERT INTO app_execution(app_id,mode,device_id,operation_id,state_sha256,updated_at) VALUES(?1,'rv',?2,?3,?4,?5) ON CONFLICT(app_id) DO UPDATE SET mode='rv',device_id=excluded.device_id,operation_id=excluded.operation_id,state_sha256=excluded.state_sha256,updated_at=excluded.updated_at",params![id.as_str(),device.to_string(),uuid::Uuid::new_v4().to_string(),state_sha256(state.archive()),timestamp])?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.db.execute_batch(if own_transaction { "COMMIT" } else { "RELEASE rv_takeover" })?;
                self.publish_changes();
                Ok(())
            }
            Err(error) => {
                self.db
                    .execute_batch(if own_transaction { "ROLLBACK" } else { "ROLLBACK TO rv_takeover; RELEASE rv_takeover" })?;
                self.changes.restore(checkpoint);
                Err(error)
            }
        }
    }
    pub fn complete_phone_handoff(
        &mut self,
        id: &AppId,
        operation: &str,
        device: ConnectionId,
        sha: &str,
    ) -> Result<(), StoreError> {
        self.business_transaction(|db| {
            if db.execute("UPDATE app_execution SET mode='phone',updated_at=?5 WHERE app_id=?1 AND mode='to_phone' AND operation_id=?2 AND device_id=?3 AND state_sha256=?4 AND NOT EXISTS(SELECT 1 FROM tasks WHERE app_id=?1 AND status='running')",params![id.as_str(),operation,device.to_string(),sha,models::now()])? != 1 {
                return Err(StoreError::Conflict);
            }
            // A verified backup handoff also transfers refresh responsibility.
            db.execute("INSERT INTO app_token_sources(app_id,device_id) VALUES(?1,?2) ON CONFLICT(app_id) DO UPDATE SET device_id=excluded.device_id",params![id.as_str(),device.to_string()])?;
            Ok(())
        })
    }
}
