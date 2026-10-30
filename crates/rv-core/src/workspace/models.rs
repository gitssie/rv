use crate::{ConnectionId, StoreError};
use serde::{Deserialize, Serialize};

/// Canonical, real IDFV. No random-ID constructor is provided.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AppId(String);
impl AppId {
    pub fn from_idfv(value: &str) -> Result<Self, StoreError> {
        let uuid = uuid::Uuid::parse_str(value)
            .map_err(|_| StoreError::Invalid("备份缺少有效 IDFV".into()))?;
        if uuid.is_nil() {
            return Err(StoreError::Invalid("IDFV 不可为空值".into()));
        }
        Ok(Self(uuid.hyphenated().to_string().to_uppercase()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for AppId {
    type Error = StoreError;
    fn try_from(v: String) -> Result<Self, StoreError> {
        Self::from_idfv(&v)
    }
}
impl From<AppId> for String {
    fn from(v: AppId) -> Self {
        v.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInstance {
    pub id: AppId,
    pub bundle_id: String,
    pub title: String,
    pub identity_key: Option<String>,
    pub tags: Vec<String>,
    pub note: String,
    /// Per-plugin data is retained as text, including unknown business fields.
    pub metadata: String,
    pub favorite: bool,
    pub pinned: bool,
    pub archived: bool,
    pub position: i64,
    pub group_name: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_used_at: Option<i64>,
    pub revision: i64,
}
#[derive(Debug, Clone)]
pub struct BackupLink {
    pub app_id: AppId,
    pub device_id: ConnectionId,
    pub backup_id: String,
    pub title: String,
    /// Exact backup result row; extra protocol fields are preserved.
    pub metadata: String,
}
#[derive(Debug, Clone, Copy, Default)]
pub enum AppFilter {
    #[default]
    All,
    Favorites,
    Recent,
    Archived,
}
#[derive(Default)]
pub struct AppQuery<'a> {
    pub text: &'a str,
    pub tag: Option<&'a str>,
    pub device_id: Option<ConnectionId>,
    pub filter: AppFilter,
}
#[derive(Debug, Default)]
pub struct SyncResult {
    pub created: usize,
    pub linked: usize,
    pub missing_idfv: usize,
    pub baselines: usize,
}
pub(super) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
