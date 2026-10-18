//! Locally managed OAuth mailboxes used for explicit verification-code reads.
use super::WorkspaceStore;
use crate::{StoreError, store::credentials};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct MailboxAccount {
    pub id: String,
    pub email: String,
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
            || client_id.is_empty()
            || client_id.len() > 512
            || refresh_token.is_empty()
            || refresh_token.len() > 65536
        {
            return Err(StoreError::Invalid(
                "邮箱 OAuth 资料不完整或格式无效".into(),
            ));
        }
        let account = MailboxAccount {
            id: uuid::Uuid::new_v4().to_string(),
            email: email.to_owned(),
            client_id: client_id.to_owned(),
            refresh_token: refresh_token.to_owned(),
        };
        let encrypted = credentials::seal(
            &format!("rv-mailbox|{}", account.id),
            &serde_json::to_vec(&account)?,
        )?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let duplicate = {
            let mut query = tx.prepare("SELECT id,encrypted FROM mailbox_accounts")?;
            let rows = query.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })?;
            rows.map(|row| {
                let (id, encrypted) = row?;
                let plain = credentials::unseal(&format!("rv-mailbox|{id}"), encrypted)?;
                let saved: MailboxAccount = serde_json::from_slice(&plain)?;
                Ok(saved.email.eq_ignore_ascii_case(email))
            })
            .collect::<Result<Vec<_>, StoreError>>()?
            .into_iter()
            .any(|is_duplicate| is_duplicate)
        };
        if duplicate {
            return Err(StoreError::Invalid("该邮箱已在列表中".into()));
        }
        tx.execute(
            "INSERT INTO mailbox_accounts(id,encrypted) VALUES(?1,?2)",
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
