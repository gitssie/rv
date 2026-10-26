//! Global Apple ID registry. Passwords and code URLs are plaintext by explicit user request.
//! Device IDs are optional hints, with no FK, uniqueness constraint or login restriction.
use super::WorkspaceStore;
use crate::StoreError;
use ring::rand::{SecureRandom, SystemRandom};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppleInitStatus {
    Pending,
    ChangingPassword,
    Ready,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppleAccountStatus {
    Unknown,
    Active,
    Locked,
    Disabled,
}
#[derive(Clone)]
pub struct AppleAccount {
    pub apple_id: String,
    pub original_password: String,
    pub password: String,
    pub pending_password: Option<String>,
    pub verification_code_url: Option<String>,
    pub device_id: Option<String>,
    pub params: Value,
    pub init_status: AppleInitStatus,
    pub account_status: AppleAccountStatus,
    pub created_at: i64,
    pub updated_at: i64,
    pub first_login_at: Option<i64>,
    pub last_login_at: Option<i64>,
    pub password_changed_at: Option<i64>,
    pub last_error_domain: Option<String>,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
    pub last_checked_at: Option<i64>,
    pub password_change_operation_id: Option<String>,
    pub note: String,
    pub revision: i64,
}
impl std::fmt::Debug for AppleAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppleAccount")
            .field("apple_id", &self.apple_id)
            .field("init_status", &self.init_status)
            .field("account_status", &self.account_status)
            .field("credentials", &"[redacted]")
            .finish()
    }
}
#[derive(Debug)]
pub struct AppleLoginPlan {
    pub account: AppleAccount,
    pub first_login: bool,
}
#[derive(Clone)]
pub struct AppleLoginFailure {
    pub domain: Option<String>,
    pub code: Option<String>,
    pub message: String,
    /// Only explicit server evidence may set Locked/Disabled. Transport failures leave this None.
    pub account_status: Option<AppleAccountStatus>,
}
const COLS: &str = "apple_id,original_password,password,pending_password,verification_code_url,device_id,params,init_status,account_status,created_at,updated_at,first_login_at,last_login_at,password_changed_at,last_error_domain,last_error_code,last_error_message,last_checked_at,password_change_operation_id,note,revision";
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
fn invalid() -> StoreError {
    StoreError::Invalid("Apple ID 资料或操作状态无效".into())
}
fn normalize(value: &str) -> Result<String, StoreError> {
    let value = value.trim().to_lowercase();
    if value.is_empty()
        || value.len() > 320
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(invalid());
    }
    Ok(value)
}
fn valid_password(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control)
}
fn status<T: serde::de::DeserializeOwned>(
    row: &rusqlite::Row<'_>,
    col: usize,
) -> rusqlite::Result<T> {
    let value: String = row.get(col)?;
    serde_json::from_value(Value::String(value)).map_err(|_| rusqlite::Error::InvalidQuery)
}
fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AppleAccount> {
    let p: String = row.get(6)?;
    let parameters = serde_json::from_str(&p).map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(AppleAccount {
        apple_id: row.get(0)?,
        original_password: row.get(1)?,
        password: row.get(2)?,
        pending_password: row.get(3)?,
        verification_code_url: row.get(4)?,
        device_id: row.get(5)?,
        params: parameters,
        init_status: status(row, 7)?,
        account_status: status(row, 8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        first_login_at: row.get(11)?,
        last_login_at: row.get(12)?,
        password_changed_at: row.get(13)?,
        last_error_domain: row.get(14)?,
        last_error_code: row.get(15)?,
        last_error_message: row.get(16)?,
        last_checked_at: row.get(17)?,
        password_change_operation_id: row.get(18)?,
        note: row.get(19)?,
        revision: row.get(20)?,
    })
}
fn status_text<T: Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}
/// Uniform samples from 62 ASCII characters, rejecting candidates missing any class.
/// The generated password is exactly eight bytes and always has upper/lower/digits.
pub fn generate_apple_password() -> Result<String, StoreError> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let random = SystemRandom::new();
    loop {
        let mut password = Vec::with_capacity(8);
        while password.len() < 8 {
            let mut bytes = [0u8; 16];
            random
                .fill(&mut bytes)
                .map_err(|_| StoreError::Invalid("密码随机数生成失败".into()))?;
            for byte in bytes {
                if byte < 248 {
                    password.push(ALPHABET[usize::from(byte) % 62]);
                    if password.len() == 8 {
                        break;
                    }
                }
            }
        }
        if password.iter().any(u8::is_ascii_uppercase)
            && password.iter().any(u8::is_ascii_lowercase)
            && password.iter().any(u8::is_ascii_digit)
        {
            return String::from_utf8(password).map_err(|_| invalid());
        }
    }
}
impl WorkspaceStore {
    pub fn apple_account(&self, apple_id: &str) -> Result<Option<AppleAccount>, StoreError> {
        self.db
            .query_row(
                &format!("SELECT {COLS} FROM apple_accounts WHERE apple_id=?1"),
                [normalize(apple_id)?],
                from_row,
            )
            .optional()
            .map_err(Into::into)
    }
    pub fn apple_account_for_password_operation(
        &self,
        id: &str,
    ) -> Result<Option<AppleAccount>, StoreError> {
        if id.is_empty() || id.len() > 128 {
            return Err(invalid());
        }
        self.db.query_row(&format!("SELECT {COLS} FROM apple_accounts WHERE password_change_operation_id=?1 AND init_status IN ('changing_password','unknown')"),[id],from_row).optional().map_err(Into::into)
    }
    pub fn apple_accounts(&self) -> Result<Vec<AppleAccount>, StoreError> {
        self.query_apple_accounts("")
    }
    /// Literal, case-insensitive search of account metadata, excluding credentials.
    pub fn query_apple_accounts(&self, query: &str) -> Result<Vec<AppleAccount>, StoreError> {
        let query = query.trim().to_lowercase();
        if query.len() > 1024 {
            return Err(invalid());
        }
        self.db
            .prepare(&format!(
                "SELECT {COLS} FROM apple_accounts WHERE ?1='' OR instr(lower(apple_id||' '||COALESCE(device_id,'')||' '||note||' '||params||' '||account_status||' '||init_status),?1)>0 ORDER BY created_at DESC,apple_id"
            ))?
            .query_map([query], from_row)?
            .collect::<Result<_, _>>()
            .map_err(Into::into)
    }
    /// Saves the first supplied password once; subsequent paste/login attempts use the current stored password.
    pub fn prepare_apple_login(
        &mut self,
        apple_id: &str,
        password: &str,
        code_url: Option<&str>,
        device_id: Option<&str>,
        parameters: &Value,
    ) -> Result<AppleLoginPlan, StoreError> {
        let apple_id = normalize(apple_id)?;
        if (!password.is_empty() && !valid_password(password))
            || !parameters.is_object()
            || parameters.to_string().len() > 32768
            || device_id.is_some_and(|v| v.is_empty() || v.len() > 128)
        {
            return Err(invalid());
        }
        if let Some(url) = code_url {
            let parsed = url::Url::parse(url).map_err(|_| invalid())?;
            if url.len() > 8192
                || !matches!(parsed.scheme(), "http" | "https")
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
            {
                return Err(invalid());
            }
        }
        // Run-specific credentials, OTPs and device unlock secrets are never copied into params.
        for key in [
            "password",
            "original_password",
            "pending_password",
            "new_password",
            "code",
            "device_passcode",
            "challenge_id",
            "_password_change_origin",
        ] {
            if parameters.get(key).is_some() {
                return Err(invalid());
            }
        }
        let at = now();
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if password.is_empty()
            && !tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM apple_accounts WHERE apple_id=?1)",
                [&apple_id],
                |row| row.get::<_, bool>(0),
            )?
        {
            return Err(invalid());
        }
        let inserted=tx.execute("INSERT INTO apple_accounts(apple_id,original_password,password,verification_code_url,device_id,params,created_at,updated_at) VALUES(?1,?2,?2,?3,?4,?5,?6,?6) ON CONFLICT(apple_id) DO NOTHING",params![apple_id,password,code_url,device_id,parameters.to_string(),at])?==1;
        if !inserted {
            tx.execute("UPDATE apple_accounts SET verification_code_url=COALESCE(?2,verification_code_url),device_id=COALESCE(?3,device_id),updated_at=?4 WHERE apple_id=?1",params![apple_id,code_url,device_id,at])?;
        }
        let account = tx.query_row(
            &format!("SELECT {COLS} FROM apple_accounts WHERE apple_id=?1"),
            [&apple_id],
            from_row,
        )?;
        tx.commit()?;
        Ok(AppleLoginPlan {
            account,
            first_login: inserted,
        })
    }
    pub fn apple_login_succeeded(
        &mut self,
        expected: &AppleAccount,
    ) -> Result<AppleAccount, StoreError> {
        let at = now();
        let n=self.db.execute("UPDATE apple_accounts SET account_status='active',first_login_at=COALESCE(first_login_at,?3),last_login_at=?3,last_checked_at=?3,updated_at=?3,last_error_domain=NULL,last_error_code=NULL,last_error_message=NULL,revision=revision+1 WHERE apple_id=?1 AND revision=?2",params![expected.apple_id,expected.revision,at])?;
        if n != 1 {
            return Err(StoreError::Conflict);
        }
        self.apple_account(&expected.apple_id)?.ok_or_else(invalid)
    }
    /// A pending candidate is durable before any request can change the remote password.
    pub fn begin_apple_password_change(
        &mut self,
        expected: &AppleAccount,
    ) -> Result<AppleAccount, StoreError> {
        self.stage_apple_password_change(expected, false)
    }
    pub fn begin_manual_apple_password_change(
        &mut self,
        expected: &AppleAccount,
    ) -> Result<AppleAccount, StoreError> {
        self.stage_apple_password_change(expected, true)
    }
    fn stage_apple_password_change(
        &mut self,
        expected: &AppleAccount,
        manual: bool,
    ) -> Result<AppleAccount, StoreError> {
        if !matches!(
            expected.init_status,
            AppleInitStatus::Pending | AppleInitStatus::Ready
        ) || (!manual && expected.account_status != AppleAccountStatus::Active)
            || matches!(
                expected.account_status,
                AppleAccountStatus::Locked | AppleAccountStatus::Disabled
            )
        {
            return Err(invalid());
        }
        let candidate = loop {
            let v = generate_apple_password()?;
            if v != expected.password && v != expected.original_password {
                break v;
            }
        };
        let n=self.db.execute("UPDATE apple_accounts SET pending_password=?3,password_change_operation_id=NULL,init_status='changing_password',params=json_set(params,'$._password_change_origin',?5),updated_at=?4,revision=revision+1 WHERE apple_id=?1 AND revision=?2 AND init_status IN ('pending','ready')",params![expected.apple_id,expected.revision,candidate,now(),if manual {"manual"} else {"first_login"}])?;
        if n != 1 {
            return Err(StoreError::Conflict);
        }
        self.apple_account(&expected.apple_id)?.ok_or_else(invalid)
    }
    pub fn record_apple_password_operation(
        &mut self,
        expected: &AppleAccount,
        operation_id: &str,
    ) -> Result<AppleAccount, StoreError> {
        if operation_id.is_empty() || operation_id.len() > 128 {
            return Err(invalid());
        }
        let n=self.db.execute("UPDATE apple_accounts SET password_change_operation_id=?3,updated_at=?4,revision=revision+1 WHERE apple_id=?1 AND revision=?2 AND init_status='changing_password'",params![expected.apple_id,expected.revision,operation_id,now()])?;
        if n != 1 {
            return Err(StoreError::Conflict);
        }
        self.apple_account(&expected.apple_id)?.ok_or_else(invalid)
    }
    pub fn apple_password_change_succeeded(
        &mut self,
        expected: &AppleAccount,
    ) -> Result<AppleAccount, StoreError> {
        let at = now();
        let n=self.db.execute("UPDATE apple_accounts SET password=pending_password,pending_password=NULL,init_status='ready',account_status='active',password_changed_at=?3,updated_at=?3,revision=revision+1 WHERE apple_id=?1 AND revision=?2 AND init_status IN ('changing_password','unknown') AND pending_password IS NOT NULL",params![expected.apple_id,expected.revision,at])?;
        if n != 1 {
            return Err(StoreError::Conflict);
        }
        self.apple_account(&expected.apple_id)?.ok_or_else(invalid)
    }
    pub fn apple_login_failed(
        &mut self,
        expected: &AppleAccount,
        failure: &AppleLoginFailure,
        uncertain: bool,
    ) -> Result<AppleAccount, StoreError> {
        if failure.message.len() > 1024
            || failure.domain.as_ref().is_some_and(|s| s.len() > 128)
            || failure.code.as_ref().is_some_and(|s| s.len() > 128)
        {
            return Err(invalid());
        }
        let at = now();
        let state = failure.account_status.map(status_text);
        let n=self.db.execute("UPDATE apple_accounts SET account_status=COALESCE(?3,account_status),last_error_domain=?4,last_error_code=?5,last_error_message=?6,last_checked_at=?7,updated_at=?7,init_status=CASE WHEN init_status IN ('changing_password','unknown') AND pending_password IS NOT NULL AND ?8 THEN 'unknown' WHEN init_status IN ('changing_password','unknown') AND pending_password IS NOT NULL THEN CASE WHEN password_changed_at IS NOT NULL THEN 'ready' ELSE 'pending' END ELSE init_status END,pending_password=CASE WHEN init_status IN ('changing_password','unknown') AND NOT ?8 THEN NULL ELSE pending_password END,password_change_operation_id=CASE WHEN init_status IN ('changing_password','unknown') AND NOT ?8 THEN NULL ELSE password_change_operation_id END,revision=revision+1 WHERE apple_id=?1 AND revision=?2",params![expected.apple_id,expected.revision,state,failure.domain,failure.code,failure.message,at,uncertain])?;
        if n != 1 {
            return Err(StoreError::Conflict);
        }
        self.apple_account(&expected.apple_id)?.ok_or_else(invalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StorePaths;
    fn setup() -> (tempfile::TempDir, WorkspaceStore) {
        let dir = tempfile::tempdir().unwrap();
        let s = WorkspaceStore::open(&StorePaths::in_dir(dir.path().into())).unwrap();
        (dir, s)
    }
    fn first(s: &mut WorkspaceStore) -> AppleAccount {
        s.prepare_apple_login(
            " Test@Example.com ",
            "Original123",
            Some("https://code.example.test/private"),
            Some("device-a"),
            &serde_json::json!({}),
        )
        .unwrap()
        .account
    }
    #[test]
    fn plaintext_original_password_and_url_survive_existing_account_and_device_changes() {
        let (_dir, mut s) = setup();
        let a = first(&mut s);
        assert_eq!(a.apple_id, "test@example.com");
        let b = s
            .prepare_apple_login(
                "TEST@example.com",
                "wrong-pasted-old-password",
                None,
                Some("device-b"),
                &serde_json::json!({}),
            )
            .unwrap();
        assert!(!b.first_login);
        assert_eq!(b.account.password, "Original123");
        assert_eq!(b.account.original_password, "Original123");
        assert_eq!(b.account.device_id.as_deref(), Some("device-b"));
        assert_eq!(a.created_at, b.account.created_at);
        assert!(b.account.updated_at >= a.updated_at);
        let raw: (String, String) =
            s.db.query_row(
                "SELECT password,verification_code_url FROM apple_accounts",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            raw,
            (
                "Original123".into(),
                "https://code.example.test/private".into()
            )
        );
        let n: i64 =
            s.db.query_row(
                "SELECT count(*) FROM pragma_foreign_key_list('apple_accounts')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0);
        s.prepare_apple_login(
            "other@test",
            "Other123",
            None,
            Some("device-b"),
            &serde_json::json!({}),
        )
        .unwrap();
        assert_eq!(s.apple_accounts().unwrap().len(), 2);
        assert!(!format!("{a:?}").contains("Original123"));
        assert!(!format!("{a:?}").contains("private"));
    }
    #[test]
    fn password_change_preserves_original_and_uncertain_candidate_then_commits_once() {
        let (_dir, mut s) = setup();
        let a = first(&mut s);
        let a = s.apple_login_succeeded(&a).unwrap();
        assert!(a.first_login_at.is_some() && a.last_login_at.is_some());
        let a = s.begin_apple_password_change(&a).unwrap();
        let candidate = a.pending_password.clone().unwrap();
        assert_eq!(candidate.len(), 8);
        assert_eq!(a.password, "Original123");
        assert!(s.begin_apple_password_change(&a).is_err());
        let a = s.record_apple_password_operation(&a, "receipt-1").unwrap();
        let a = s
            .apple_login_failed(
                &a,
                &AppleLoginFailure {
                    domain: None,
                    code: None,
                    message: "transport interrupted".into(),
                    account_status: None,
                },
                true,
            )
            .unwrap();
        assert_eq!(a.init_status, AppleInitStatus::Unknown);
        assert_eq!(a.password, "Original123");
        assert_eq!(a.pending_password.as_ref(), Some(&candidate));
        let done = s.apple_password_change_succeeded(&a).unwrap();
        assert_eq!(done.password, candidate);
        assert_eq!(done.original_password, "Original123");
        assert!(done.pending_password.is_none());
        assert!(done.password_changed_at.is_some());
        assert_eq!(done.init_status, AppleInitStatus::Ready);
        assert!(matches!(
            s.apple_password_change_succeeded(&a),
            Err(StoreError::Conflict)
        ));
        let next = s
            .prepare_apple_login("test@example.com", "", None, None, &serde_json::json!({}))
            .unwrap();
        assert_eq!(next.account.password, done.password);
        assert_eq!(
            next.account.verification_code_url,
            done.verification_code_url
        );
        let locked = s
            .apple_login_failed(
                &next.account,
                &AppleLoginFailure {
                    domain: Some("native".into()),
                    code: Some("locked".into()),
                    message: "Apple explicitly locked this account".into(),
                    account_status: Some(AppleAccountStatus::Locked),
                },
                false,
            )
            .unwrap();
        assert_eq!(locked.account_status, AppleAccountStatus::Locked);
        assert_eq!(locked.init_status, AppleInitStatus::Ready);
        let unchanged = s
            .apple_login_failed(
                &locked,
                &AppleLoginFailure {
                    domain: None,
                    code: None,
                    message: "network timeout".into(),
                    account_status: None,
                },
                true,
            )
            .unwrap();
        assert_eq!(unchanged.account_status, AppleAccountStatus::Locked);
    }
    #[test]
    fn concurrent_login_and_failed_changes_cannot_overwrite_new_password() {
        let (_dir, mut s) = setup();
        let a = first(&mut s);
        let b = s.apple_login_succeeded(&a).unwrap();
        assert!(matches!(
            s.apple_login_succeeded(&a),
            Err(StoreError::Conflict)
        ));
        let change = s.begin_apple_password_change(&b).unwrap();
        let change = s
            .record_apple_password_operation(&change, "old-receipt")
            .unwrap();
        let unknown = s
            .apple_login_failed(
                &change,
                &AppleLoginFailure {
                    domain: None,
                    code: None,
                    message: "interrupted".into(),
                    account_status: None,
                },
                true,
            )
            .unwrap();
        let rejected = s
            .apple_login_failed(
                &unknown,
                &AppleLoginFailure {
                    domain: Some("apple".into()),
                    code: Some("rejected".into()),
                    message: "Password rejected".into(),
                    account_status: None,
                },
                false,
            )
            .unwrap();
        assert_eq!(rejected.init_status, AppleInitStatus::Pending);
        assert!(rejected.pending_password.is_none());
        assert_eq!(rejected.password, "Original123");
        assert!(s.apple_password_change_succeeded(&change).is_err());
        assert!(rejected.password_change_operation_id.is_none());
        let retry = s.begin_apple_password_change(&rejected).unwrap();
        assert!(retry.password_change_operation_id.is_none());
        assert!(retry.pending_password.is_some());
    }
    #[test]
    fn rejected_manual_rotation_preserves_completed_initialization() {
        let (_dir, mut s) = setup();
        let a = first(&mut s);
        let a = s.apple_login_succeeded(&a).unwrap();
        let a = s.begin_apple_password_change(&a).unwrap();
        let ready = s.apple_password_change_succeeded(&a).unwrap();
        let changing = s.begin_apple_password_change(&ready).unwrap();
        assert_ne!(changing.pending_password, Some(ready.password.clone()));
        let failed = s
            .apple_login_failed(
                &changing,
                &AppleLoginFailure {
                    domain: None,
                    code: None,
                    message: "Explicit native rejection".into(),
                    account_status: None,
                },
                false,
            )
            .unwrap();
        assert_eq!(failed.init_status, AppleInitStatus::Ready);
        assert_eq!(failed.password, ready.password);
        assert_eq!(failed.original_password, "Original123");
        assert!(failed.pending_password.is_none());
        assert_eq!(
            s.prepare_apple_login("test@example.com", "", None, None, &serde_json::json!({}))
                .unwrap()
                .account
                .init_status,
            AppleInitStatus::Ready
        );
    }
    #[test]
    fn metadata_search_is_literal_and_does_not_search_passwords() {
        let (_dir, mut s) = setup();
        first(&mut s);
        s.db.execute(
            "UPDATE apple_accounts SET note='literal 100%_value',params='{\"region\":\"France\"}'",
            [],
        )
        .unwrap();
        for query in [
            "TEST@EXAMPLE",
            "device-a",
            "France",
            "100%_value",
            "pending",
        ] {
            assert_eq!(s.query_apple_accounts(query).unwrap().len(), 1);
        }
        for query in ["Original123", "' OR 1=1 --", "missing", "%__"] {
            assert!(s.query_apple_accounts(query).unwrap().is_empty());
        }
    }
    #[test]
    fn generated_passwords_are_exactly_eight_mixed_ascii_characters() {
        for _ in 0..512 {
            let p = generate_apple_password().unwrap();
            assert_eq!(p.len(), 8);
            assert!(p.bytes().all(|v| v.is_ascii_alphanumeric()));
            assert!(p.bytes().any(|v| v.is_ascii_uppercase()));
            assert!(p.bytes().any(|v| v.is_ascii_lowercase()));
            assert!(p.bytes().any(|v| v.is_ascii_digit()));
        }
    }
    #[test]
    fn validation_rejects_missing_first_password_and_transient_secrets() {
        let (_dir, mut s) = setup();
        assert!(
            s.prepare_apple_login("new@test", "", None, None, &serde_json::json!({}))
                .is_err()
        );
        assert!(
            s.prepare_apple_login(
                "new@test",
                "p",
                None,
                None,
                &serde_json::json!({"device_passcode":"123456"})
            )
            .is_err()
        );
        assert!(
            s.prepare_apple_login(
                "new@test",
                "p",
                Some("https://user:secret@code.test"),
                None,
                &serde_json::json!({})
            )
            .is_err()
        );
        assert!(s.apple_accounts().unwrap().is_empty());
    }
}
