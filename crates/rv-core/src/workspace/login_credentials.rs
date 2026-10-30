//! Account login credentials are independent of OAuth tokens and phone backups.
use super::{AppId, WorkspaceStore};
use crate::StoreError;
use rusqlite::{OptionalExtension, params};

#[derive(Clone, PartialEq, Eq)]
pub struct AppLoginCredentials {
    pub username: String,
    /// None means unknown; an unverified generated password must not be saved here.
    pub password: Option<String>,
}
impl std::fmt::Debug for AppLoginCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppLoginCredentials")
            .field("username", &self.username)
            .field("password", &"[redacted]")
            .finish()
    }
}
fn validate_username(username: &str) -> Result<(), StoreError> {
    if username.is_empty()
        || username.len() > 256
        || username.chars().any(char::is_control)
        || username.trim() != username
    {
        return Err(StoreError::Invalid("登录用户名无效".into()));
    }
    Ok(())
}
pub(super) fn write_login_credentials(db: &rusqlite::Connection, id: &AppId, credentials: &AppLoginCredentials) -> Result<(), StoreError> {
    validate_username(&credentials.username)?;
    if credentials.password.as_ref().is_some_and(|password| password.is_empty() || password.len() > 4096) {
        return Err(StoreError::Invalid("登录密码为空或过长".into()));
    }
    db.execute("INSERT INTO app_login_credentials(app_id,login_username,login_password) VALUES(?1,?2,?3) ON CONFLICT(app_id) DO UPDATE SET login_username=excluded.login_username,login_password=excluded.login_password",
        params![id.as_str(),credentials.username,credentials.password])?;
    Ok(())
}

impl WorkspaceStore {
    pub fn login_credentials(&self, id: &AppId) -> Result<Option<AppLoginCredentials>, StoreError> {
        self.db
            .query_row(
                "SELECT login_username,login_password FROM app_login_credentials WHERE app_id=?1",
                [id.as_str()],
                |row| {
                    Ok(AppLoginCredentials {
                        username: row.get(0)?,
                        password: row.get(1)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }
    /// Save only known credentials. This never sends a password-change request.
    pub fn save_login_credentials(
        &mut self,
        id: &AppId,
        credentials: &AppLoginCredentials,
    ) -> Result<(), StoreError> {
        self.business_transaction(|db| write_login_credentials(db, id, credentials))
    }

    pub(super) fn sync_login_username(&self, id: &AppId, username: &str) -> Result<(), StoreError> {
        validate_username(username)?;
        self.db.execute("INSERT INTO app_login_credentials(app_id,login_username) VALUES(?1,?2) ON CONFLICT(app_id) DO UPDATE SET login_username=excluded.login_username WHERE app_login_credentials.login_username<>excluded.login_username",
            params![id.as_str(),username])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
