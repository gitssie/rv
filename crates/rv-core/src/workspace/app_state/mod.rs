//! Complete Shadow state plus a small encrypted, directly readable AppAuth view.
mod archive;
mod context;
use super::{AppAuth, AppId, WorkspaceStore};
use crate::{ConnectionId, StoreError};
use archive::*;
pub use archive::{MAX_MANIFEST_BYTES, MAX_STATE_BYTES, state_sha256};
pub use context::{AppDeviceInfo, AppRequestContext};
use plist::Value as Plist;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

fn invalid(message: &str) -> StoreError {
    StoreError::Invalid(message.into())
}
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[derive(Clone)]
pub struct ShadowAppState {
    manifest: Vec<u8>,
    archive: Vec<u8>,
    payloads: [Vec<u8>; 3],
    device: AppDeviceInfo,
}
impl std::fmt::Debug for ShadowAppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ShadowAppState([redacted])")
    }
}
impl ShadowAppState {
    pub fn decode(
        id: &AppId,
        bundle: &str,
        manifest: Vec<u8>,
        archive: Vec<u8>,
        device: AppDeviceInfo,
    ) -> Result<Self, StoreError> {
        let payloads = unpack(&manifest, &archive, id, bundle)?;
        Ok(Self {
            manifest,
            archive,
            payloads,
            device,
        })
    }
    pub fn manifest(&self) -> &[u8] {
        &self.manifest
    }
    pub fn archive(&self) -> &[u8] {
        &self.archive
    }
    pub fn payload(&self, name: &str) -> Option<&[u8]> {
        NAMES
            .iter()
            .position(|n| *n == name)
            .map(|i| self.payloads[i].as_slice())
    }
    pub fn request_auth(&self) -> Result<AppAuth, StoreError> {
        context::credentials(self)
    }
}
#[derive(Clone, Debug)]
pub struct AppStateInfo {
    pub source_device_id: Option<ConnectionId>,
    pub archive_sha256: String,
    pub archive_size: i64,
    pub imported_at: i64,
    pub updated_at: i64,
}
fn scope(id: &AppId, bundle: &str, field: &str) -> String {
    format!("rv-app-state|{}|{bundle}|{field}", id.as_str())
}
fn seal(id: &AppId, bundle: &str, field: &str, data: &[u8]) -> Result<Vec<u8>, StoreError> {
    crate::store::credentials::seal(&scope(id, bundle, field), data)
}
fn unseal(id: &AppId, bundle: &str, field: &str, data: Vec<u8>) -> Result<Vec<u8>, StoreError> {
    crate::store::credentials::unseal(&scope(id, bundle, field), data)
}

impl AppAuth {
    pub fn request_context(&self) -> Option<AppRequestContext> {
        serde_json::from_value(self.extra.get("request_context")?.clone()).ok()
    }
}
impl WorkspaceStore {
    pub fn import_app_state(
        &mut self,
        id: &AppId,
        source: Option<ConnectionId>,
        state: &ShadowAppState,
    ) -> Result<(), StoreError> {
        super::execution::require_rv_execution(&self.db, id.as_str())?;
        self.import_app_state_inner(id, source, state, true)
    }
    pub(super) fn import_app_state_inner(
        &mut self,
        id: &AppId,
        source: Option<ConnectionId>,
        state: &ShadowAppState,
        preserve_host_credentials: bool,
    ) -> Result<(), StoreError> {
        let mut app = self.load(id)?;
        let manifest = parse(&state.manifest)?;
        if text(&manifest, "bundleID") != Some(app.bundle_id.as_str())
            || text(&manifest, "idfv") != Some(id.as_str())
        {
            return Err(invalid("state 与所选 App 不匹配"));
        }
        let mut state = state.clone();
        let mut auth = state.request_auth()?;
        self.db.execute_batch("SAVEPOINT import_app_state")?;
        let checkpoint = self.changes.checkpoint();
        let outcome = (|| {
            let existing_auth = self.auth(id)?;
            // A token first imported from a phone belongs to that phone. Later
            // backup imports preserve the user's chosen source and live token.
            if existing_auth.is_none() {
                if let Some(device) = source {
                    self.db.execute("INSERT OR IGNORE INTO app_token_sources(app_id,device_id) VALUES(?1,?2)",params![id.as_str(),device.to_string()])?;
                }
            }
            if let Some(existing) = existing_auth.filter(|_| preserve_host_credentials) {
                // Downloading a historical snapshot cannot revert host-rotated credentials.
                if existing.access_token != auth.access_token
                    || existing.refresh_token != auth.refresh_token
                {
                    auth.access_token = existing.access_token;
                    auth.refresh_token = existing.refresh_token;
                    auth.expires_at = existing.expires_at;
                    for (key, value) in existing.extra {
                        if key != "request_context" {
                            auth.extra.insert(key, value);
                        }
                    }
                    context::patch_credentials(&mut state, &auth)?;
                }
            }
            let bundle = &app.bundle_id;
            let timestamp = now();
            self.db.execute("INSERT INTO app_states(app_id,source_device_id,manifest_blob,archive_blob,keychain_blob,identity_blob,preferences_blob,device_context_blob,archive_sha256,archive_size,imported_at,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11,?11) ON CONFLICT(app_id) DO UPDATE SET source_device_id=excluded.source_device_id,manifest_blob=excluded.manifest_blob,archive_blob=excluded.archive_blob,keychain_blob=excluded.keychain_blob,identity_blob=excluded.identity_blob,preferences_blob=excluded.preferences_blob,device_context_blob=excluded.device_context_blob,archive_sha256=excluded.archive_sha256,archive_size=excluded.archive_size,imported_at=excluded.imported_at,updated_at=excluded.updated_at",params![id.as_str(),source.map(|s|s.to_string()),seal(id,bundle,"manifest",&state.manifest)?,seal(id,bundle,"archive",&state.archive)?,seal(id,bundle,"keychain",&state.payloads[0])?,seal(id,bundle,"identity",&state.payloads[1])?,seal(id,bundle,"preferences",&state.payloads[2])?,seal(id,bundle,"device",&serde_json::to_vec(&state.device)?)?,state_sha256(&state.archive),state.archive.len() as i64,timestamp])?;
            self.save_auth(id, &auth)?;
            let mut metadata: Value = serde_json::from_str(&app.metadata)?;
            metadata["profile"] = context::profile_metadata(&auth);
            app.metadata = metadata.to_string();
            self.save(&app)?;
            Ok(())
        })();
        match outcome {
            Ok(()) => {
                self.db.execute_batch("RELEASE import_app_state")?;
                self.publish_changes();
            }
            Err(error) => {
                self.db
                    .execute_batch("ROLLBACK TO import_app_state; RELEASE import_app_state")?;
                self.changes.restore(checkpoint);
                return Err(error);
            }
        }
        Ok(())
    }
    pub fn app_state_info(&self, id: &AppId) -> Result<Option<AppStateInfo>, StoreError> {
        let row=self.db.query_row("SELECT source_device_id,archive_sha256,archive_size,imported_at,updated_at FROM app_states WHERE app_id=?1",[id.as_str()],|r|Ok((r.get::<_,Option<String>>(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        row.map(|(source, sha, size, imported, updated)| {
            Ok(AppStateInfo {
                source_device_id: source
                    .map(|s| {
                        uuid::Uuid::parse_str(&s)
                            .map(ConnectionId)
                            .map_err(|_| invalid("来源设备 ID 无效"))
                    })
                    .transpose()?,
                archive_sha256: sha,
                archive_size: size,
                imported_at: imported,
                updated_at: updated,
            })
        })
        .transpose()
    }
    pub fn app_state(&self, id: &AppId) -> Result<Option<ShadowAppState>, StoreError> {
        let app = self.load(id)?;
        let row=self.db.query_row("SELECT manifest_blob,archive_blob,keychain_blob,identity_blob,preferences_blob,device_context_blob FROM app_states WHERE app_id=?1",[id.as_str()],|r|Ok((r.get::<_,Vec<u8>>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,Vec<u8>>(2)?,r.get::<_,Vec<u8>>(3)?,r.get::<_,Vec<u8>>(4)?,r.get::<_,Vec<u8>>(5)?))).optional()?;
        row.map(
            |(manifest, archive, keychain, identity, preferences, device)| {
                Ok(ShadowAppState {
                    manifest: unseal(id, &app.bundle_id, "manifest", manifest)?,
                    archive: unseal(id, &app.bundle_id, "archive", archive)?,
                    payloads: [
                        unseal(id, &app.bundle_id, "keychain", keychain)?,
                        unseal(id, &app.bundle_id, "identity", identity)?,
                        unseal(id, &app.bundle_id, "preferences", preferences)?,
                    ],
                    device: serde_json::from_slice(&unseal(id, &app.bundle_id, "device", device)?)?,
                })
            },
        )
        .transpose()
    }
    /// Materialize current plists into a matching archive/manifest only when
    /// an export is requested. Normal API/token reads never call this method.
    pub fn prepare_state_upload(&mut self, id: &AppId) -> Result<ShadowAppState, StoreError> {
        let app = self.load(id)?;
        let mut state = self
            .app_state(id)?
            .ok_or_else(|| invalid("尚未下载此 App 的 state"))?;
        let manifest = parse(&state.manifest)?;
        let entries = at(at(&manifest, "stateArchiveInfo")?, "entries")?
            .as_array()
            .ok_or_else(|| invalid("state 清单无效"))?;
        let changed = NAMES.iter().enumerate().any(|(i, name)| {
            entries
                .iter()
                .find(|e| text(e, "path") == Some(*name))
                .and_then(|e| text(e, "sha256"))
                != Some(state_sha256(&state.payloads[i]).as_str())
        });
        if changed {
            (state.manifest, state.archive) = repack(&state.manifest, &state.payloads)?;
            self.db.execute("UPDATE app_states SET manifest_blob=?2,archive_blob=?3,archive_sha256=?4,archive_size=?5 WHERE app_id=?1",params![id.as_str(),seal(id,&app.bundle_id,"manifest",&state.manifest)?,seal(id,&app.bundle_id,"archive",&state.archive)?,state_sha256(&state.archive),state.archive.len() as i64])?;
        }
        self.publish_changes();
        Ok(state)
    }
    pub(super) fn patch_saved_state_auth(
        &mut self,
        id: &AppId,
        bundle: &str,
        auth: &AppAuth,
    ) -> Result<(), StoreError> {
        let row=self.db.query_row("SELECT manifest_blob,keychain_blob,preferences_blob FROM app_states WHERE app_id=?1",[id.as_str()],|r|Ok((r.get::<_,Vec<u8>>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,Vec<u8>>(2)?))).optional()?;
        let Some((manifest, keychain, preferences)) = row else {
            return Ok(());
        };
        let mut state = ShadowAppState {
            manifest: unseal(id, bundle, "manifest", manifest)?,
            archive: vec![],
            payloads: [
                unseal(id, bundle, "keychain", keychain)?,
                vec![],
                unseal(id, bundle, "preferences", preferences)?,
            ],
            device: Default::default(),
        };
        let old_keychain = state.payloads[0].clone();
        let old_preferences = state.payloads[2].clone();
        context::patch_credentials(&mut state, auth)?;
        if old_keychain != state.payloads[0] || old_preferences != state.payloads[2] {
            self.db.execute("UPDATE app_states SET keychain_blob=?2,preferences_blob=?3,updated_at=?4 WHERE app_id=?1",params![id.as_str(),seal(id,bundle,"keychain",&state.payloads[0])?,seal(id,bundle,"preferences",&state.payloads[2])?,now()])?;
        }
        Ok(())
    }
    pub fn update_app_tokens(
        &mut self,
        id: &AppId,
        access: String,
        refresh: Option<String>,
        expires_at: Option<i64>,
    ) -> Result<(), StoreError> {
        let mut auth = self.auth(id)?.ok_or_else(|| invalid("尚未读取账号凭据"))?;
        auth.access_token = access;
        auth.refresh_token = refresh;
        auth.expires_at = expires_at;
        self.save_auth(id, &auth)
    }
}

#[cfg(test)]
mod tests;
