use super::*;
use crate::StoreError;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct AppAuth {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}
impl std::fmt::Debug for AppAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AppAuth([redacted])")
    }
}
#[derive(Clone, Debug)]
pub struct AppTask {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub result: Option<Value>,
}
impl WorkspaceStore {
    pub fn save_auth(&mut self, id: &AppId, auth: &AppAuth) -> Result<(), StoreError> {
        let app = self.load(id)?;
        if auth.access_token.is_empty()
            || auth.access_token.len() > 65536
            || auth.refresh_token.as_ref().is_some_and(|s| s.len() > 65536)
            || auth.headers.len() > 32
            || auth
                .headers
                .iter()
                .any(|(k, v)| k.len() > 128 || v.len() > 4096 || v.contains(['\r', '\n']))
        {
            return Err(StoreError::Invalid("App 凭据不完整或过长".into()));
        }
        if serde_json::to_vec(auth)?.len() > 256 * 1024 {
            return Err(StoreError::Invalid("App 凭据超过大小限制".into()));
        }
        let scope = format!("rv-app-auth|{}|{}", id.as_str(), app.bundle_id);
        let encrypted = crate::store::credentials::seal(&scope, &serde_json::to_vec(auth)?)?;
        self.db.execute("INSERT INTO app_auth VALUES(?1,?2) ON CONFLICT(app_id) DO UPDATE SET encrypted=excluded.encrypted",params![id.as_str(),encrypted])?;
        Ok(())
    }
    pub fn auth(&self, id: &AppId) -> Result<Option<AppAuth>, StoreError> {
        let app = self.load(id)?;
        let data: Option<Vec<u8>> = self
            .db
            .query_row(
                "SELECT encrypted FROM app_auth WHERE app_id=?1",
                [id.as_str()],
                |r| r.get(0),
            )
            .optional()?;
        data.map(|bytes| {
            let plain = crate::store::credentials::unseal(
                &format!("rv-app-auth|{}|{}", id.as_str(), app.bundle_id),
                bytes,
            )?;
            serde_json::from_slice(&plain).map_err(Into::into)
        })
        .transpose()
    }
    /// Import credentials for an existing real-IDFV card; never creates objects.
    pub fn import_profile(&mut self, id: &AppId, profile: &Value) -> Result<(), StoreError> {
        let mut app = self.load(id)?;
        if profile["bundle_id"].as_str() != Some(&app.bundle_id)
            || AppId::from_idfv(profile["idfv"].as_str().unwrap_or(""))? != *id
            || profile["supported"] != true
        {
            return Err(StoreError::Invalid("账号资料与所选备份不匹配".into()));
        }
        let mut auth: AppAuth = serde_json::from_value(profile["credentials"].clone())
            .map_err(|_| StoreError::Invalid("备份未提供可用 access token".into()))?;
        if auth.expires_at.is_none() {
            auth.expires_at = profile["metadata"]["exp"].as_i64().or_else(|| {
                profile["metadata"]["exp"]
                    .as_str()
                    .and_then(|value| value.parse().ok())
            });
        }
        let mut metadata: Value = serde_json::from_str(&app.metadata)?;
        if !metadata.is_object() || !profile["metadata"].is_object() {
            return Err(StoreError::Invalid("账号元数据格式错误".into()));
        }
        metadata["profile"] = profile["metadata"].clone();
        app.metadata = metadata.to_string();
        // No raw reply/credential material is written to searchable metadata.
        self.db.execute_batch("SAVEPOINT import_profile")?;
        // A historical phone backup must not replace a newer host-refreshed token pair.
        let result = (|| {
            match self.auth(id)? {
                None => self.save_auth(id, &auth)?,
                Some(mut existing)
                    if existing.access_token == auth.access_token
                        && existing.refresh_token == auth.refresh_token =>
                {
                    // Supplement missing fields only when the backup contains the exact same token pair.
                    // A historical profile cannot attach its expiry to a rotated host token.
                    if existing.expires_at.is_none() {
                        existing.expires_at = auth.expires_at;
                    }
                    for (key, value) in &auth.extra {
                        existing
                            .extra
                            .entry(key.clone())
                            .or_insert_with(|| value.clone());
                    }
                    for (key, value) in &auth.headers {
                        existing
                            .headers
                            .entry(key.clone())
                            .or_insert_with(|| value.clone());
                    }
                    self.save_auth(id, &existing)?;
                }
                Some(_) => {}
            }
            self.save(&app).map(|_| ())
        })();
        match result {
            Ok(()) => {
                self.db.execute_batch("RELEASE import_profile")?;
                Ok(())
            }
            Err(error) => {
                self.db
                    .execute_batch("ROLLBACK TO import_profile; RELEASE import_profile")?;
                Err(error)
            }
        }
    }
    pub fn start_task(&mut self, id: &AppId, kind: &str) -> Result<String, StoreError> {
        self.start_task_when(id, kind, |_| Ok(()))
    }
    /// Admission checks and task creation must share a transaction.
    pub fn start_task_when(
        &mut self,
        id: &AppId,
        kind: &str,
        admit: impl FnOnce(&rusqlite::Connection) -> Result<(), StoreError>,
    ) -> Result<String, StoreError> {
        self.load(id)?;
        let task = uuid::Uuid::new_v4().to_string();
        let timestamp = now();
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        admit(&tx)?;
        tx.execute("UPDATE app_tasks SET status='unknown',finished_at=?2 WHERE app_id=?1 AND status='running' AND deadline<?2",params![id.as_str(),timestamp])?;
        if kind == "refresh" {
            let unknown:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM app_tasks WHERE app_id=?1 AND kind='refresh' AND status='unknown')",[id.as_str()],|row|row.get(0))?;
            if unknown {
                tx.commit()?;
                return Err(StoreError::Invalid(
                    "上次刷新结果未知，请先核对凭据，不重复提交".into(),
                ));
            }
        }
        tx.execute("INSERT INTO app_tasks(id,app_id,kind,status,created_at,deadline) VALUES(?1,?2,?3,'running',?4,?5)",params![task,id.as_str(),kind,timestamp,timestamp+90])?;
        tx.commit()?;
        Ok(task)
    }
    pub fn finish_task(
        &mut self,
        task: &str,
        status: &str,
        result: &Value,
    ) -> Result<(), StoreError> {
        if !["completed", "failed", "unknown"].contains(&status)
            || result.to_string().len() > 1024 * 1024
        {
            return Err(StoreError::Invalid("任务结果无效".into()));
        }
        if self.db.execute("UPDATE app_tasks SET status=?2,result=?3,finished_at=?4 WHERE id=?1 AND status='running'",params![task,status,result.to_string(),now()])? != 1 { return Err(StoreError::Conflict); }
        Ok(())
    }
    pub fn tasks(&self, id: &AppId) -> Result<Vec<AppTask>, StoreError> {
        let mut query=self.db.prepare("SELECT id,kind,status,result FROM app_tasks WHERE app_id=?1 ORDER BY created_at DESC,rowid DESC LIMIT 50")?;
        let rows = query.query_map([id.as_str()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })?;
        rows.map(|r| {
            let (id, kind, status, result) = r?;
            Ok(AppTask {
                id,
                kind,
                status,
                result: result.map(|s| serde_json::from_str(&s)).transpose()?,
            })
        })
        .collect()
    }
    /// Commit the task outcome, optional credential rotation and card projection together.
    pub fn complete_task(
        &mut self,
        id: &AppId,
        task: &str,
        status: &str,
        result: &Value,
        auth: Option<&AppAuth>,
        projection: Option<&Value>,
    ) -> Result<(), StoreError> {
        self.complete_task_with(id, task, status, result, auth, projection, |_| Ok(()))
    }
    /// Plugin business rows share the same commit as the successful task result.
    pub fn complete_task_with(
        &mut self,
        id: &AppId,
        task: &str,
        status: &str,
        result: &Value,
        auth: Option<&AppAuth>,
        projection: Option<&Value>,
        persist: impl FnOnce(&rusqlite::Connection) -> Result<(), StoreError>,
    ) -> Result<(), StoreError> {
        self.db.execute_batch("BEGIN IMMEDIATE")?;
        let outcome = (|| {
            let belongs:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM app_tasks WHERE id=?1 AND app_id=?2 AND status='running')",params![task,id.as_str()],|row|row.get(0))?;
            if !belongs {
                return Err(StoreError::Conflict);
            }
            persist(&self.db)?;
            if let Some(auth) = auth {
                self.save_auth(id, auth)?;
            }
            if let Some(patch) = projection {
                let fields = patch
                    .as_object()
                    .ok_or_else(|| StoreError::Invalid("任务投影格式错误".into()))?;
                let mut app = self.load(id)?;
                let mut data: Value = serde_json::from_str(&app.metadata)?;
                let object = data
                    .as_object_mut()
                    .ok_or_else(|| StoreError::Invalid("业务元数据格式错误".into()))?;
                object.extend(
                    fields
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone())),
                );
                app.metadata = data.to_string();
                self.save(&app)?;
            }
            self.finish_task(task, status, result)
        })();
        match outcome {
            Ok(()) => {
                self.db.execute_batch("COMMIT")?;
                Ok(())
            }
            Err(error) => {
                self.db.execute_batch("ROLLBACK")?;
                Err(error)
            }
        }
    }
}
