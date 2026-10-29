//! Shared UI/query contracts; concrete schemas remain inside Bundle plugins.
use super::*;
use rv_core::StoreError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dataset {
    Products,
    Orders,
}
impl Dataset {
    pub fn label(self) -> &'static str {
        match self {
            Self::Products => "商品",
            Self::Orders => "订单",
        }
    }
    pub fn operation(self) -> Operation {
        match self {
            Self::Products => Operation::Products,
            Self::Orders => Operation::Orders,
        }
    }
}
#[derive(Clone, Debug)]
pub struct BusinessRow {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub payload: Value,
    pub fetched_at: i64,
    pub visual: crate::presentation::RowVisual,
}
#[derive(Clone, Debug, Default)]
pub struct BusinessPage {
    pub rows: Vec<BusinessRow>,
    pub total: usize,
}
pub fn read_data(
    paths: &StorePaths,
    id: &AppId,
    dataset: Dataset,
    text: &str,
    offset: usize,
) -> Result<BusinessPage, String> {
    let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    let app = store.load(id).map_err(|e| e.to_string())?;
    let plugin = strategy(&app.bundle_id).ok_or("此 App 尚未接入业务数据")?;
    store
        .business_transaction(|db| plugin.read_data(db, id, dataset, text, offset))
        .map_err(|e| e.to_string())
}
pub(crate) fn unsupported_data() -> StoreError {
    StoreError::Invalid("此 App 尚未接入业务数据".into())
}

/// Fetch local records for a scrollable compact panel without UI pagination.
pub fn read_all(paths: &StorePaths, id: &AppId, dataset: Dataset) -> Result<BusinessPage, String> {
    let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    let app = store.load(id).map_err(|e| e.to_string())?;
    let plugin = strategy(&app.bundle_id).ok_or("此 App 尚未接入业务数据")?;
    store
        .business_transaction(|db| {
            let mut page = plugin.read_data(db, id, dataset, "", 0)?;
            while page.rows.len() < page.total {
                let next = plugin.read_data(db, id, dataset, "", page.rows.len())?;
                if next.rows.is_empty() {
                    break;
                }
                page.rows.extend(next.rows);
            }
            Ok(page)
        })
        .map_err(|e| e.to_string())
}
