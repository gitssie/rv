//! Locally managed OAuth mailboxes used for explicit verification-code reads.
use super::WorkspaceStore;
use crate::StoreError;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct MailboxAccount {
    pub id: String,
    pub email: String,
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
            .prepare("SELECT id,email,password,client_id,refresh_token FROM mailbox_accounts")?;
        let rows = query.query_map([], mailbox_from_row)?;
        let mut accounts: Vec<MailboxAccount> = rows.collect::<Result<_, _>>()?;
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
        let existing_id = tx
            .query_row(
                "SELECT id FROM mailbox_accounts WHERE email=?1 COLLATE NOCASE LIMIT 1",
                [email],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let account = MailboxAccount {
            id: existing_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            email: email.to_owned(),
            password: password.to_owned(),
            client_id: client_id.to_owned(),
            refresh_token: refresh_token.to_owned(),
        };
        tx.execute(
            "INSERT INTO mailbox_accounts(id,email,password,client_id,refresh_token) \
             VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET \
             email=excluded.email,password=excluded.password,client_id=excluded.client_id, \
             refresh_token=excluded.refresh_token",
            params![
                account.id,
                account.email,
                account.password,
                account.client_id,
                account.refresh_token
            ],
        )?;
        tx.commit()?;
        Ok(account)
    }

    pub fn mailbox_account(&self, id: &str) -> Result<Option<MailboxAccount>, StoreError> {
        self.db
            .query_row(
                "SELECT id,email,password,client_id,refresh_token FROM mailbox_accounts WHERE id=?1",
                [id],
                mailbox_from_row,
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn delete_mailbox_account(&mut self, id: &str) -> Result<bool, StoreError> {
        Ok(self
            .db
            .execute("DELETE FROM mailbox_accounts WHERE id=?1", [id])?
            > 0)
    }
}

fn mailbox_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MailboxAccount> {
    Ok(MailboxAccount {
        id: row.get(0)?,
        email: row.get(1)?,
        password: row.get(2)?,
        client_id: row.get(3)?,
        refresh_token: row.get(4)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailbox_credentials_are_plaintext_and_resaves_replace_the_same_account() {
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

        let stored: (String, String, String, String) = store
            .db
            .query_row(
                "SELECT email,password,client_id,refresh_token FROM mailbox_accounts WHERE id=?1",
                [&saved.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            stored,
            (
                "mail@example.test".into(),
                "test-password".into(),
                "test-client".into(),
                "test-refresh-token".into()
            )
        );

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
}
