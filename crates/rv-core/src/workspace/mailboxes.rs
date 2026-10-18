//! Locally managed OAuth mailboxes used for explicit verification-code reads.
use super::WorkspaceStore;
use crate::{StoreError, store::credentials};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct MailboxAccount {
    pub id: String,
    pub email: String,
    #[serde(default)]
    pub password: String,
    pub client_id: String,
    pub refresh_token: String,
}

impl std::fmt::Debug for MailboxAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MailboxAccount")
            .field("id", &self.id)
            .field("email", &self.email)
            .field("credentials", &"[redacted]")
            .finish()
    }
}

impl WorkspaceStore {
    pub fn mailbox_accounts(&self) -> Result<Vec<MailboxAccount>, StoreError> {
        let mut query = self
            .db
            .prepare("SELECT id,encrypted FROM mailbox_accounts ORDER BY id")?;
        let rows = query.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        let mut accounts: Vec<MailboxAccount> = rows
            .map(|row| {
                let (id, encrypted) = row?;
                let plain = credentials::unseal(&format!("rv-mailbox|{id}"), encrypted)?;
                Ok(serde_json::from_slice(&plain)?)
            })
            .collect::<Result<_, StoreError>>()?;
        accounts.sort_by_key(|account| account.email.to_ascii_lowercase());
        Ok(accounts)
    }

    pub fn save_mailbox_account(
        &mut self,
        email: &str,
        password: &str,
        client_id: &str,
        refresh_token: &str,
    ) -> Result<MailboxAccount, StoreError> {
        let email = email.trim();
        let client_id = client_id.trim();
        if !email.contains('@')
            || email.len() > 320
            || email
                .bytes()
                .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
            || password.len() > 4096
            || client_id.is_empty()
            || client_id.len() > 512
            || refresh_token.is_empty()
            || refresh_token.len() > 65536
        {
            return Err(StoreError::Invalid(
                "邮箱 OAuth 资料不完整或格式无效".into(),
            ));
        }
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let existing_id = {
            let mut query = tx.prepare("SELECT id,encrypted FROM mailbox_accounts")?;
            let rows = query.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })?;
            rows.map(|row| {
                let (id, encrypted) = row?;
                let plain = credentials::unseal(&format!("rv-mailbox|{id}"), encrypted)?;
                let saved: MailboxAccount = serde_json::from_slice(&plain)?;
                Ok(saved.email.eq_ignore_ascii_case(email).then_some(saved.id))
            })
            .collect::<Result<Vec<_>, StoreError>>()?
            .into_iter()
            .flatten()
            .next()
        };
        let account = MailboxAccount {
            id: existing_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            email: email.to_owned(),
            password: password.to_owned(),
            client_id: client_id.to_owned(),
            refresh_token: refresh_token.to_owned(),
        };
        let encrypted = credentials::seal(
            &format!("rv-mailbox|{}", account.id),
            &serde_json::to_vec(&account)?,
        )?;
        tx.execute(
            "INSERT INTO mailbox_accounts(id,encrypted) VALUES(?1,?2) \
             ON CONFLICT(id) DO UPDATE SET encrypted=excluded.encrypted",
            params![account.id, encrypted],
        )?;
        tx.commit()?;
        Ok(account)
    }

    pub fn mailbox_account(&self, id: &str) -> Result<Option<MailboxAccount>, StoreError> {
        let encrypted = self
            .db
            .query_row(
                "SELECT encrypted FROM mailbox_accounts WHERE id=?1",
                [id],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        encrypted
            .map(|encrypted| {
                let plain = credentials::unseal(&format!("rv-mailbox|{id}"), encrypted)?;
                Ok(serde_json::from_slice(&plain)?)
            })
            .transpose()
    }

    pub fn delete_mailbox_account(&mut self, id: &str) -> Result<bool, StoreError> {
        Ok(self
            .db
            .execute("DELETE FROM mailbox_accounts WHERE id=?1", [id])?
            > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailbox_credentials_are_encrypted_and_resaves_replace_the_same_account() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::StorePaths::in_dir(dir.path().into());
        let mut store = WorkspaceStore::open(&paths).unwrap();

        let saved = store
            .save_mailbox_account(
                "mail@example.test",
                "test-password",
                "test-client",
                "test-refresh-token",
            )
            .unwrap();
        assert_eq!(saved.password, "test-password");

        let encrypted: Vec<u8> = store
            .db
            .query_row(
                "SELECT encrypted FROM mailbox_accounts WHERE id=?1",
                [&saved.id],
                |row| row.get(0),
            )
            .unwrap();
        for secret in [b"test-password".as_slice(), b"test-refresh-token"] {
            assert!(
                !encrypted
                    .windows(secret.len())
                    .any(|window| window == secret)
            );
        }

        let updated = store
            .save_mailbox_account(
                "MAIL@example.test",
                "new-password",
                "new-client",
                "new-refresh-token",
            )
            .unwrap();
        assert_eq!(updated.id, saved.id);
        assert_eq!(store.mailbox_accounts().unwrap().len(), 1);
        assert_eq!(
            store.mailbox_account(&saved.id).unwrap().unwrap().password,
            "new-password"
        );
    }

    #[test]
    fn mailbox_account_without_password_deserializes_as_legacy_record() {
        let old = serde_json::json!({
            "id": "mailbox-id",
            "email": "mail@example.test",
            "client_id": "client",
            "refresh_token": "token"
        });
        let account: MailboxAccount = serde_json::from_value(old).unwrap();
        assert!(account.password.is_empty());
    }
}
