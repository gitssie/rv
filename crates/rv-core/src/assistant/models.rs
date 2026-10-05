use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Draft {
    pub id: String,
    pub title: String,
    pub description: String,
    pub price_minor: Option<i64>,
    /// Unfinished/invalid input is retained without treating it as a valid amount.
    #[serde(default)]
    pub price_input: Option<String>,
    pub currency: String,
    pub stock_note: String,
    pub revision: i64,
    pub updated_at: i64,
}
impl Default for Draft {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            title: String::new(),
            description: String::new(),
            price_minor: None,
            price_input: None,
            currency: "EUR".into(),
            stock_note: String::new(),
            revision: 0,
            updated_at: now(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftImage {
    pub id: String,
    pub draft_id: String,
    pub filename: String,
    pub relative_path: String,
    pub sha256: String,
    pub position: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Template {
    pub id: String,
    pub name: String,
    pub content: String,
    pub category: String,
    pub revision: i64,
}

#[derive(Debug, Clone)]
pub struct ImportReceipt {
    pub id: String,
    pub image_id: String,
    pub target: String,
    pub status: String,
    pub asset_id: Option<String>,
    pub error: Option<String>,
}

/// Fixed-point money input, independent of locale and floating point rounding.
pub fn parse_price(value: &str) -> Result<Option<i64>, String> {
    let value = value.trim().trim_start_matches('€').trim();
    if value.is_empty() {
        return Ok(None);
    }
    let value = value.replace(',', ".");
    let parts: Vec<_> = value.split('.').collect();
    if parts.len() > 2 || parts[0].is_empty() || !parts[0].bytes().all(|b| b.is_ascii_digit()) {
        return Err("价格应为非负金额，例如 18.50".into());
    }
    let fraction = parts.get(1).copied().unwrap_or("");
    if fraction.len() > 2 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return Err("价格最多两位小数".into());
    }
    let whole = parts[0].parse::<i64>().map_err(|_| "价格过大")?;
    let cents = match fraction.len() {
        0 => 0,
        1 => fraction.parse::<i64>().unwrap() * 10,
        _ => fraction.parse::<i64>().unwrap(),
    };
    whole
        .checked_mul(100)
        .and_then(|v| v.checked_add(cents))
        .filter(|v| *v <= 999_999_999)
        .map(Some)
        .ok_or_else(|| "价格过大".into())
}
pub fn format_price(value: Option<i64>) -> String {
    value
        .map(|v| format!("{}.{:02}", v / 100, v % 100))
        .unwrap_or_default()
}
