use super::*;
use crate::{ConnectionId, StoreError, events::PhoneTokenRequested};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TokenSource {
    #[default]
    Rv,
    Phone(Option<ConnectionId>),
}
/// Deliberately excludes refresh tokens. Debug must never expose token/header values.
#[derive(Clone, Deserialize, Serialize)]
pub struct PhoneAccessToken {
    pub bundle_id: String,
    pub idfv: String,
    pub access_token: String,
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub headers: std::collections::BTreeMap<String, String>,
}
impl std::fmt::Debug for PhoneAccessToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PhoneAccessToken([redacted])")
    }
}
impl WorkspaceStore {
    pub fn wait_for_access_token(&mut self, id: &AppId) -> Result<(), StoreError> {
        self.business_transaction(|db| {
            db.execute("UPDATE tasks SET status='pending',next_run_at=?2,error_code='access_token',error_message='等待 Token',updated_at=?2 WHERE app_id=?1 AND enabled=1 AND kind<>'refresh' AND status IN ('pending','failed') AND error_code IS NOT 'access_token'", params![id.as_str(), super::models::now()])?;
            Ok(())
        })
    }
    pub fn token_source(&self, id: &AppId) -> Result<TokenSource, StoreError> {
        Self::token_source_in(&self.db, id.as_str())
    }
    pub fn token_source_in(db: &rusqlite::Connection, id: &str) -> Result<TokenSource, StoreError> {
        let value: Option<Option<String>> = db
            .query_row(
                "SELECT device_id FROM app_token_sources WHERE app_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        match value {
            None => Ok(TokenSource::Rv),
            Some(device) => Ok(TokenSource::Phone(
                device
                    .map(|s| {
                        uuid::Uuid::parse_str(&s)
                            .map(ConnectionId)
                            .map_err(|_| StoreError::Invalid("Token 来源设备无效".into()))
                    })
                    .transpose()?,
            )),
        }
    }
    pub fn set_token_source(&mut self, id: &AppId, source: TokenSource) -> Result<(), StoreError> {
        self.load(id)?;
        self.business_transaction(|db| {
            match source {
                TokenSource::Rv => { db.execute("DELETE FROM app_token_sources WHERE app_id=?1", [id.as_str()])?; }
                TokenSource::Phone(Some(device)) => {
                    db.execute("INSERT INTO app_token_sources VALUES(?1,?2) ON CONFLICT(app_id) DO UPDATE SET device_id=excluded.device_id", params![id.as_str(), device.to_string()])?;
                }
                TokenSource::Phone(None) => return Err(StoreError::Invalid("请选择提供 Token 的手机".into())),
            }
            Ok(())
        })?;
        if matches!(source, TokenSource::Phone(_)) {
            self.request_phone_token(id, false)?;
        }
        Ok(())
    }
    /// Called once for a newly rejected token. Phone reads are independent of
    /// Tasks; RV refresh may wake only the user's already-enabled keepalive.
    pub fn request_access_token(&mut self, id: &AppId) -> Result<(), StoreError> {
        if matches!(self.token_source(id)?, TokenSource::Phone(_)) {
            return self.request_phone_token(id, false);
        }
        self.business_transaction(|db| {
            db.execute("UPDATE tasks SET status='pending',next_run_at=?2,updated_at=?2 WHERE app_id=?1 AND kind='refresh' AND task_key='keepalive' AND enabled=1 AND status IN ('pending','failed','completed')",params![id.as_str(),super::models::now()])?;
            Ok(())
        })
    }
    pub fn request_phone_token(&self, id: &AppId, manual: bool) -> Result<(), StoreError> {
        if matches!(self.token_source(id)?, TokenSource::Phone(_)) {
            self.events.post(PhoneTokenRequested {
                app_id: id.as_str().into(),
                manual,
            });
        }
        Ok(())
    }
    pub fn phone_token_apps(&self, device: ConnectionId) -> Result<Vec<AppInstance>, StoreError> {
        let ids = self
            .db
            .prepare("SELECT app_id FROM app_token_sources WHERE device_id=?1")?
            .query_map([device.to_string()], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|s| self.load(&AppId::from_idfv(&s)?))
            .collect()
    }
    /// The source check and credential update are one transaction. A late phone
    /// read cannot overwrite credentials after the user changes their source.
    pub fn accept_phone_token(
        &mut self,
        device: ConnectionId,
        token: &PhoneAccessToken,
    ) -> Result<bool, StoreError> {
        let id = AppId::from_idfv(&token.idfv)?;
        self.db.execute_batch("BEGIN IMMEDIATE")?;
        let checkpoint = self.changes.checkpoint();
        let result = (|| {
            if self.token_source(&id)? != TokenSource::Phone(Some(device)) {
                return Err(StoreError::Invalid("Token 来源已改变".into()));
            }
            let app = self.load(&id)?;
            if app.bundle_id != token.bundle_id {
                return Err(StoreError::Invalid("手机 Token 与店铺不匹配".into()));
            }
            let mut auth = self.auth(&id)?.unwrap_or_default();
            let changed = auth.access_token != token.access_token;
            if !changed && auth.access_token_rejected() {
                return Ok(false);
            }
            auth.access_token = token.access_token.clone();
            auth.expires_at = token.expires_at;
            // Keep RV's refresh credential untouched; phone-sourced tokens can
            // never authorize RV to consume it.
            auth.extra.insert("scope".into(), serde_json::json!("user"));
            for (key, value) in &token.headers {
                if key.eq_ignore_ascii_case("authorization") {
                    continue;
                }
                auth.headers.retain(|old, _| !old.eq_ignore_ascii_case(key));
                auth.headers.insert(key.clone(), value.clone());
            }
            // First use of a phone token adopts its observed Session; subsequent
            // uses keep the existing per-token mapping across all tasks.
            if let Some(session) = token
                .headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("X-Session-Id"))
                .and_then(|(_, s)| uuid::Uuid::parse_str(s).ok())
            {
                self.db.execute(
                    "INSERT OR IGNORE INTO app_request_sessions VALUES(?1,?2,?3)",
                    params![
                        id.as_str(),
                        ring::digest::digest(&ring::digest::SHA256, token.access_token.as_bytes())
                            .as_ref(),
                        session.to_string()
                    ],
                )?;
            }
            self.save_auth_inner(&id, &auth, false)?;
            Ok(changed)
        })();
        match result {
            Ok(changed) => {
                self.db.execute_batch("COMMIT")?;
                self.publish_changes();
                Ok(changed)
            }
            Err(error) => {
                self.db.execute_batch("ROLLBACK")?;
                self.changes.restore(checkpoint);
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AddressBook, Connection, StorePaths};
    fn fixture() -> (tempfile::TempDir, WorkspaceStore, AppId, ConnectionId) {
        let dir = tempfile::tempdir().unwrap();
        let paths = StorePaths::in_dir(dir.path().into());
        let mut book = AddressBook::load(paths.clone()).unwrap();
        let phone = Connection::new("phone", "fixture", 5900);
        book.upsert(phone.clone());
        book.save().unwrap();
        let mut store = WorkspaceStore::open(&paths).unwrap();
        let id = AppId::from_idfv("C148FA97-37B0-4BA1-9DBB-AF619062F420").unwrap();
        store.sync_backups(phone.id,"lt.manodrabuziai.fr",&serde_json::json!({"bundle_id":"lt.manodrabuziai.fr","backups":[{"kind":"snapshot","backup_id":"snapshot:fixture","idfv":id.as_str()}]})).unwrap();
        store
            .save_auth(
                &id,
                &AppAuth {
                    access_token: "old-access".into(),
                    refresh_token: Some("rv-refresh-kept".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        (dir, store, id, phone.id)
    }
    fn token(id: &AppId, access: &str) -> PhoneAccessToken {
        PhoneAccessToken {
            bundle_id: "lt.manodrabuziai.fr".into(),
            idfv: id.as_str().into(),
            access_token: access.into(),
            expires_at: Some(2000000000),
            headers: std::collections::BTreeMap::from([(
                "X-Session-Id".into(),
                "a148fa97-37b0-4ba1-9dbb-af619062f420".into(),
            )]),
        }
    }
    #[test]
    fn new_phone_access_wakes_only_token_waiters_and_preserves_refresh_and_session() {
        let (_dir, mut store, id, phone) = fixture();
        let spec = TaskSpec::new("products", "0 */15 * * * *");
        let waiting = store.enqueue_task(&id, &spec).unwrap();
        let stopped = store
            .enqueue_task(&id, &TaskSpec::new("orders", "0 */15 * * * *"))
            .unwrap();
        store.set_task_enabled(&stopped.id, false).unwrap();
        store
            .set_token_source(&id, TokenSource::Phone(Some(phone)))
            .unwrap();
        store.reject_access_token(&id, "old-access").unwrap();
        store.wait_for_access_token(&id).unwrap();
        assert_eq!(
            store.task(&waiting.id).unwrap().error_code.as_deref(),
            Some("access_token")
        );
        assert!(
            !store
                .accept_phone_token(phone, &token(&id, "old-access"))
                .unwrap()
        );
        assert_eq!(
            store.task(&waiting.id).unwrap().error_code.as_deref(),
            Some("access_token")
        );
        assert!(
            store
                .accept_phone_token(phone, &token(&id, "new-access"))
                .unwrap()
        );
        let ready = store.task(&waiting.id).unwrap();
        assert!(ready.enabled);
        assert!(ready.error_code.is_none());
        assert_eq!(ready.cron_expression, spec.cron_expression);
        assert!(!store.task(&stopped.id).unwrap().enabled);
        let auth = store.request_auth(&id).unwrap().unwrap();
        assert_eq!(auth.access_token, "new-access");
        assert_eq!(auth.refresh_token.as_deref(), Some("rv-refresh-kept"));
        assert_eq!(
            auth.headers["X-Session-Id"],
            "a148fa97-37b0-4ba1-9dbb-af619062f420"
        );
        assert!(!auth.access_token_rejected());
    }
    #[test]
    fn stale_phone_read_cannot_replace_changed_source_or_other_identity() {
        let (_dir, mut store, id, phone) = fixture();
        store
            .set_token_source(&id, TokenSource::Phone(Some(phone)))
            .unwrap();
        let mut wrong = token(&id, "wrong");
        wrong.bundle_id = "other.bundle".into();
        assert!(store.accept_phone_token(phone, &wrong).is_err());
        assert!(
            store
                .accept_phone_token(ConnectionId(uuid::Uuid::new_v4()), &token(&id, "wrong"))
                .is_err()
        );
        store.set_token_source(&id, TokenSource::Rv).unwrap();
        assert!(
            store
                .accept_phone_token(phone, &token(&id, "late"))
                .is_err()
        );
        assert_eq!(store.auth(&id).unwrap().unwrap().access_token, "old-access");
    }
    #[test]
    fn late_rv_refresh_cannot_overwrite_new_phone_or_rv_credentials() {
        for phone_source in [true, false] {
            let (_dir, mut store, id, phone) = fixture();
            let expected = store.auth(&id).unwrap().unwrap();
            let task = store.start_task(&id, "refresh").unwrap();
            if phone_source {
                store
                    .set_token_source(&id, TokenSource::Phone(Some(phone)))
                    .unwrap();
                store
                    .accept_phone_token(phone, &token(&id, "newer-access"))
                    .unwrap();
            } else {
                let mut newer = expected.clone();
                newer.access_token = "newer-access".into();
                store.save_auth(&id, &newer).unwrap();
            }
            let mut late = expected.clone();
            late.access_token = "late-rv-access".into();
            late.refresh_token = Some("late-rv-refresh".into());
            assert!(
                !store
                    .complete_refresh_task(
                        &id,
                        &task,
                        &serde_json::json!({"credentials_updated":true}),
                        &expected,
                        &late,
                        &serde_json::json!({})
                    )
                    .unwrap()
            );
            let current = store.auth(&id).unwrap().unwrap();
            assert_eq!(current.access_token, "newer-access");
            assert_eq!(current.refresh_token.as_deref(), Some("rv-refresh-kept"));
            assert_eq!(store.task(&task).unwrap().status, "failed");
        }
    }
    #[test]
    fn rv_token_rejection_wakes_only_an_already_enabled_keepalive() {
        let (_dir,mut store,id,_phone)=fixture();
        let now=super::super::models::now();
        let task=store.ensure_task(&id,"keepalive",&TaskSpec::new("refresh","0 0 * * * *"),None,now+3600).unwrap();
        store.request_access_token(&id).unwrap();
        assert!(store.task(&task.id).unwrap().next_run_at.unwrap()<=now+1);
        store.set_task_enabled(&task.id,false).unwrap();
        store.request_access_token(&id).unwrap();
        let stopped=store.task(&task.id).unwrap();assert!(!stopped.enabled);assert!(stopped.next_run_at.is_none());
    }
    #[test]
    fn phone_ownership_does_not_block_scheduling_with_available_credentials() {
        let (_dir, mut store, id, phone) = fixture();
        store.business_transaction(|db| { db.execute("INSERT INTO app_execution(app_id,mode,device_id,operation_id,updated_at) VALUES(?1,'phone',?2,'fixture',0)",params![id.as_str(),phone.to_string()])?;Ok(()) }).unwrap();
        let task = store
            .enqueue_task(&id, &TaskSpec::new("products", "* * * * * *"))
            .unwrap();
        assert!(store.run_task_now(&task.id).is_ok());
        assert!(
            store
                .due_tasks(super::super::models::now() + 2)
                .unwrap()
                .iter()
                .any(|t| t.id == task.id)
        );
    }
}
