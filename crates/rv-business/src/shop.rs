//! Public shop fields from the existing current-user request.
use rv_core::workspace::AppInstance;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShopProfile {
    pub user_id: u64,
    pub login: Option<String>,
    pub country_code: Option<String>,
    pub country_title: Option<String>,
    pub avatar_url: Option<String>,
    pub feedback_reputation: Option<f64>,
    pub feedback_count: Option<u64>,
    pub item_count: Option<u64>,
    pub total_items_count: Option<u64>,
}
impl ShopProfile {
    pub(crate) fn from_user(user: &Value) -> Option<Self> {
        let text = |key: &str| {
            user[key]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .map(str::to_owned)
        };
        let avatar_url = user["photo"]["url"]
            .as_str()
            .filter(|value| {
                reqwest::Url::parse(value).is_ok_and(|url| {
                    url.scheme() == "https" && url.username().is_empty() && url.password().is_none()
                })
            })
            .map(str::to_owned);
        Some(Self {
            user_id: user["id"].as_u64().filter(|id| *id > 0)?,
            login: text("login"),
            country_code: text("country_iso_code").or_else(|| text("country_code")),
            country_title: text("country_title_local").or_else(|| text("country_title")),
            avatar_url,
            // Preserve the API value; do not invent a star scale or default score.
            feedback_reputation: user["feedback_reputation"]
                .as_f64()
                .filter(|v| v.is_finite() && *v >= 0.0),
            feedback_count: user["feedback_count"].as_u64(),
            item_count: user["item_count"].as_u64(),
            total_items_count: user["total_items_count"].as_u64(),
        })
    }
    pub fn from_app(app: &AppInstance) -> Option<Self> {
        let data: Value = serde_json::from_str(&app.metadata).ok()?;
        serde_json::from_value(data.get("shop")?.clone()).ok()
    }
}
