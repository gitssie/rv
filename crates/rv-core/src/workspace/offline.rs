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
/// Display-only expiry metadata. JWT claims are not a signature/validity check.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TokenExpirations {
    pub access: Option<i64>,
    pub refresh: Option<i64>,
}
fn jwt_expiry(token: &str) -> Option<i64> {
    use base64::Engine as _;
    if token.len() > 65536 {
        return None;
    }
    let mut parts = token.split('.');
    parts.next()?;
    let payload = parts.next()?;
    parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    serde_json::from_slice::<Value>(&bytes)
        .ok()?
        .get("exp")?
        .as_i64()
        .filter(|value| *value > 0)
}
impl AppAuth {
    pub fn access_token_rejected(&self)->bool {self.extra.get("access_token_rejected").and_then(serde_json::Value::as_bool)==Some(true)}

    pub fn token_expirations(&self) -> TokenExpirations {
        let refresh = if self
            .refresh_token
            .as_deref()
            .is_some_and(|token| !token.is_empty())
        {
            self.extra
                .get("refresh_token_expires_at")
                .and_then(Value::as_i64)
                .filter(|v| *v > 0)
                .or_else(|| self.refresh_token.as_deref().and_then(jwt_expiry))
                .or_else(|| {
                    self.extra
                        .get("credentials_received_at")
                        .and_then(Value::as_i64)
                        .filter(|v| *v > 0)
                        .zip(
                            self.extra
                                .get("refresh_token_expires_in")
                                .and_then(Value::as_i64)
                                .filter(|v| *v >= 0),
                        )
                        .and_then(|(start, duration)| start.checked_add(duration))
                })
        } else {
            None
        };
        TokenExpirations {
            access: if self.access_token.is_empty() {
                None
            } else {
                self.expires_at
                    .filter(|v| *v > 0)
                    .or_else(|| jwt_expiry(&self.access_token))
            },
            refresh,
        }
    }
}
impl std::fmt::Debug for AppAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AppAuth([redacted])")
    }
}
fn access_token_key(token:&str)->Vec<u8> {ring::digest::digest(&ring::digest::SHA256,token.as_bytes()).as_ref().to_vec()}
fn read_session(db:&rusqlite::Connection,id:&AppId,token:&str)->Result<Option<String>,StoreError> {
    db.query_row("SELECT session_id FROM app_request_sessions WHERE app_id=?1 AND access_token_sha256=?2",params![id.as_str(),access_token_key(token)],|r|r.get(0)).optional().map_err(Into::into)
}
fn ensure_session(db:&rusqlite::Connection,id:&AppId,token:&str)->Result<String,StoreError> {
    if let Some(session)=read_session(db,id,token)?{return Ok(session)}
    let session=uuid::Uuid::new_v4().to_string();
    db.execute("INSERT INTO app_request_sessions(app_id,access_token_sha256,session_id) VALUES(?1,?2,?3) ON CONFLICT(app_id,access_token_sha256) DO NOTHING",params![id.as_str(),access_token_key(token),session])?;
    read_session(db,id,token)?.ok_or_else(||StoreError::Invalid("Session ID 保存失败".into()))
}
fn attach_session(auth:&mut AppAuth,session:Option<String>) {
    auth.headers.retain(|key,_|!key.eq_ignore_ascii_case("X-Session-Id"));
    if let Some(session)=session {auth.headers.insert("X-Session-Id".into(),session);}
}
impl WorkspaceStore {
    /// Shared rejection state applies to this exact access token only.
    pub fn require_access_token(db: &rusqlite::Connection, app_id: &str) -> Result<(),StoreError> {
        use rusqlite::OptionalExtension;
        let row:Option<(Vec<u8>,String)>=db.query_row("SELECT c.encrypted,a.bundle_id FROM app_auth c JOIN app_instances a ON a.id=c.app_id WHERE c.app_id=?1",[app_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((data,bundle))=row {
            let auth:AppAuth=serde_json::from_slice(&crate::store::credentials::unseal(&format!("rv-app-auth|{app_id}|{bundle}"),data)?)?;
            if auth.access_token_rejected() {return Err(StoreError::Invalid("访问令牌已失效，等待更新凭据".into()));}
        }
        Ok(())
    }
    /// A late 401 must never invalidate a newer token saved by another request.
    pub fn reject_access_token(&mut self,id:&AppId,expected:&str)->Result<bool,StoreError> {
        self.business_transaction(|db| {
            let row:Option<(Vec<u8>,String)>=db.query_row("SELECT c.encrypted,a.bundle_id FROM app_auth c JOIN app_instances a ON a.id=c.app_id WHERE c.app_id=?1",[id.as_str()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            let Some((old,bundle))=row else{return Ok(false)};
            let scope=format!("rv-app-auth|{}|{bundle}",id.as_str());
            let mut auth:AppAuth=serde_json::from_slice(&crate::store::credentials::unseal(&scope,old.clone())?)?;
            if auth.access_token!=expected || auth.access_token_rejected(){return Ok(false)}
            auth.extra.insert("access_token_rejected".into(),serde_json::json!(true));
            let encrypted=crate::store::credentials::seal(&scope,&serde_json::to_vec(&auth)?)?;
            Ok(db.execute("UPDATE app_auth SET encrypted=?2 WHERE app_id=?1 AND encrypted=?3",rusqlite::params![id.as_str(),encrypted,old])?==1)
        })
    }
    pub fn save_auth(&mut self, id: &AppId, auth: &AppAuth) -> Result<(), StoreError> {
        self.save_auth_inner(id, auth, true)
    }
    pub(super) fn save_auth_inner(&mut self, id: &AppId, auth: &AppAuth, update_state: bool) -> Result<(), StoreError> {
        // Token-bound Session is plain metadata and does not count toward saved headers.
        let mut credentials=auth.clone();
        attach_session(&mut credentials,None);
        let auth=&credentials;
        if auth.access_token.is_empty()
            || auth.access_token.len() > 65536
            || auth.refresh_token.as_ref().is_some_and(|s| s.len() > 65536)
            || auth.headers.len() > 32
            || auth.headers.iter().any(|(k,v)|k.len()>128 || v.len()>4096 || v.contains(['\r','\n']))
        {return Err(StoreError::Invalid("App 凭据不完整或过长".into()));}
        if serde_json::to_vec(auth)?.len()>256*1024 {return Err(StoreError::Invalid("App 凭据超过大小限制".into()));}
        // Serialize reading/merging rejection state with every credential writer.
        // Task completion already owns an Immediate transaction and nests here.
        let own_transaction=self.db.is_autocommit();
        self.db.execute_batch(if own_transaction {"BEGIN IMMEDIATE"} else {"SAVEPOINT save_app_auth"})?;
        let checkpoint=self.changes.checkpoint();
        let outcome=(|| {
            let app=self.load(id)?;
            let mut auth=auth.clone();
            if let Some(current)=self.auth(id)? {
                if current.access_token==auth.access_token && current.access_token_rejected() {
                    auth.extra.insert("access_token_rejected".into(),serde_json::json!(true));
                } else if current.access_token!=auth.access_token {
                    auth.extra.remove("access_token_rejected");
                }
            } else {auth.extra.remove("access_token_rejected");}
            // Session IDs are intentionally plain SQLite metadata, not encrypted credentials.
            ensure_session(&self.db,id,&auth.access_token)?;
            if !auth.access_token_rejected() {
                self.db.execute("UPDATE tasks SET status='pending',next_run_at=?2,attempt_count=0,error_code=NULL,error_message=NULL,updated_at=?2 WHERE app_id=?1 AND enabled=1 AND error_code='access_token' AND status IN ('pending','failed')",params![id.as_str(),super::models::now()])?;
            }
            let scope=format!("rv-app-auth|{}|{}",id.as_str(),app.bundle_id);
            let encrypted=crate::store::credentials::seal(&scope,&serde_json::to_vec(&auth)?)?;
            if update_state { self.patch_saved_state_auth(id,&app.bundle_id,&auth)?; }
            self.db.execute("INSERT INTO app_auth VALUES(?1,?2) ON CONFLICT(app_id) DO UPDATE SET encrypted=excluded.encrypted",params![id.as_str(),encrypted])?;
            Ok(())
        })();
        match outcome {
            Ok(())=>{
                self.db.execute_batch(if own_transaction {"COMMIT"} else {"RELEASE save_app_auth"})?;
                self.publish_changes();Ok(())
            },
            Err(error)=>{
                self.db.execute_batch(if own_transaction {"ROLLBACK"} else {"ROLLBACK TO save_app_auth; RELEASE save_app_auth"})?;
                self.changes.restore(checkpoint);Err(error)
            }
        }
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
            let plain=crate::store::credentials::unseal(&format!("rv-app-auth|{}|{}",id.as_str(),app.bundle_id),bytes)?;
            let mut auth:AppAuth=serde_json::from_slice(&plain)?;
            let session=read_session(&self.db,id,&auth.access_token)?;
            attach_session(&mut auth,session);
            Ok(auth)
        }).transpose()
    }
    /// Read the current credentials with a persistent token-bound request session.
    /// Older records initialize this plain metadata once, without changing tokens/state.
    pub fn request_auth(&mut self,id:&AppId)->Result<Option<AppAuth>,StoreError> {
        let auth=self.auth(id)?;
        if auth.as_ref().is_none_or(|a|a.headers.contains_key("X-Session-Id")){return Ok(auth)}
        self.business_transaction(|db| {
            let row:Option<(Vec<u8>,String)>=db.query_row("SELECT c.encrypted,a.bundle_id FROM app_auth c JOIN app_instances a ON a.id=c.app_id WHERE c.app_id=?1",[id.as_str()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            let Some((data,bundle))=row else{return Ok(None)};
            let mut auth:AppAuth=serde_json::from_slice(&crate::store::credentials::unseal(&format!("rv-app-auth|{}|{bundle}",id.as_str()),data)?)?;
            let session=ensure_session(db,id,&auth.access_token)?;
            attach_session(&mut auth,Some(session));Ok(Some(auth))
        })
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
        let checkpoint = self.changes.checkpoint();
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
                self.publish_changes();
                Ok(())
            }
            Err(error) => {
                self.db
                    .execute_batch("ROLLBACK TO import_profile; RELEASE import_profile")?;
                self.changes.restore(checkpoint);
                Err(error)
            }
        }
    }
    /// Persist an interactive business read without creating or modifying tasks.
    pub fn commit_business_data(
        &mut self,id:&AppId,projection:&Value,
        persist:impl FnOnce(&rusqlite::Connection)->Result<(),StoreError>,
    )->Result<(),StoreError> {
        self.db.execute_batch("BEGIN IMMEDIATE")?;
        let outcome=(|| {persist(&self.db)?;self.apply_business_projection(id,projection)})();
        match outcome {
            Ok(())=>{self.db.execute_batch("COMMIT")?;self.publish_changes();Ok(())},
            Err(error)=>{self.db.execute_batch("ROLLBACK")?;Err(error)}
        }
    }
    fn apply_business_projection(&mut self,id:&AppId,patch:&Value)->Result<(),StoreError> {
        let fields=patch.as_object().ok_or_else(||StoreError::Invalid("业务投影格式错误".into()))?;
        if fields.is_empty(){return Ok(())}
        let mut app=self.load(id)?;
        let mut data:Value=serde_json::from_str(&app.metadata)?;
        let object=data.as_object_mut().ok_or_else(||StoreError::Invalid("业务元数据格式错误".into()))?;
        object.extend(fields.iter().map(|(key,value)|(key.clone(),value.clone())));
        app.metadata=data.to_string();self.save(&app).map(|_|())
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
        self.complete_task_with_policy(
            id,
            task,
            status,
            result,
            auth,
            projection,
            TaskFinishPolicy::default(),
            persist,
        )
    }
    /// A refresh response may arrive after the user changes its owner or imports
    /// newer phone credentials. Check generation inside the credential commit.
    pub fn complete_refresh_task(
        &mut self, id: &AppId, task: &str, result: &Value,
        expected: &AppAuth, updated: &AppAuth, projection: &Value,
    ) -> Result<bool, StoreError> {
        self.db.execute_batch("BEGIN IMMEDIATE")?;
        let checkpoint=self.changes.checkpoint();
        let outcome=(|| {
            let belongs:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1 AND app_id=?2 AND status='running')",params![task,id.as_str()],|r|r.get(0))?;
            if !belongs { return Err(StoreError::Conflict); }
            let current=self.auth(id)?;
            let adopt=self.token_source(id)?==TokenSource::Rv && current.as_ref().is_some_and(|current|
                current.access_token==expected.access_token && current.refresh_token==expected.refresh_token);
            if adopt {
                self.save_auth(id,updated)?;
                self.apply_business_projection(id,projection)?;
                self.finish_task_with_policy(task,"completed",result,TaskFinishPolicy::default())?;
            } else {
                self.finish_task_with_policy(task,"failed",&serde_json::json!({"credentials_updated":false,"error":"Token 来源或凭据已改变，本次刷新结果未采用"}),TaskFinishPolicy::default())?;
            }
            Ok(adopt)
        })();
        match outcome {
            Ok(adopt)=>{self.db.execute_batch("COMMIT")?;self.publish_changes();Ok(adopt)}
            Err(error)=>{self.db.execute_batch("ROLLBACK")?;self.changes.restore(checkpoint);Err(error)}
        }
    }
    pub fn complete_task_with_policy(
        &mut self,
        id: &AppId,
        task: &str,
        status: &str,
        result: &Value,
        auth: Option<&AppAuth>,
        projection: Option<&Value>,
        policy: TaskFinishPolicy,
        persist: impl FnOnce(&rusqlite::Connection) -> Result<(), StoreError>,
    ) -> Result<(), StoreError> {
        self.db.execute_batch("BEGIN IMMEDIATE")?;
        let outcome = (|| {
            let belongs: bool = self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1 AND app_id=?2 AND status='running')",
                params![task, id.as_str()],
                |row| row.get(0),
            )?;
            if !belongs {
                return Err(StoreError::Conflict);
            }
            persist(&self.db)?;
            if let Some(auth) = auth {
                self.save_auth(id, auth)?;
            }
            if let Some(patch)=projection {self.apply_business_projection(id,patch)?;}
            self.finish_task_with_policy(task, status, result, policy)
        })();
        match outcome {
            Ok(()) => {
                self.db.execute_batch("COMMIT")?;
                self.publish_changes();
                Ok(())
            }
            Err(error) => {
                self.db.execute_batch("ROLLBACK")?;
                Err(error)
            }
        }
    }
}
