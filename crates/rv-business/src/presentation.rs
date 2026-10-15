//! Bundle-owned projection: unknown and false values never create positive badges.
use crate::data::Dataset;
use serde_json::Value;
use std::path::PathBuf;
#[derive(Clone, Debug, Default)]
pub struct RowVisual {
    pub price: String,
    pub badges: Vec<String>,
    pub image_url: Option<String>,
    pub image_path: Option<PathBuf>,
}
pub(crate) fn vinted(value: &Value, dataset: Dataset) -> RowVisual {
    let mut badges = Vec::new();
    if dataset == Dataset::Products {
        for (field, label) in [
            ("is_hidden", "隐藏"),
            ("is_draft", "草稿"),
            ("is_reserved", "预留"),
            ("is_processing", "处理中"),
            ("is_closed", "已关闭"),
        ] {
            if value[field].as_bool() == Some(true) {
                badges.push(label.into());
            }
        }
        // These are explicit derived predicates, never defaults for missing fields.
        if value["transaction_permitted"].as_bool() == Some(false) {
            badges.push("不可交易".into());
        }
        if value["item_alert_type"].as_str() == Some("delayed_publication") {
            badges.push("延迟发布".into());
        }
    } else {
        if let Some(status) = value["status"].as_str().filter(|s| !s.is_empty()) {
            badges.push(status.into());
        }
        match value["_rv_direction"].as_str() {
            Some("sold") => badges.push("卖出".into()),
            Some("purchased") => badges.push("买入".into()),
            _ => {}
        }
    }
    let photo = value["photos"].as_array().and_then(|photos| {
        photos
            .iter()
            .find(|p| p["is_main"].as_bool() == Some(true))
            .or_else(|| photos.first())
    });
    let image_url = if dataset == Dataset::Products {
        photo.and_then(|p| {
            p["thumbnails"]
                .as_array()
                .and_then(|ts| ts.iter().find_map(|t| t["url"].as_str()))
                .or_else(|| p["url"].as_str())
        })
    } else {
        value["photo_url"].as_str()
    }
    .filter(|s| !s.is_empty())
    .map(str::to_owned);
    RowVisual {
        price: format!(
            "{} {}",
            value["price"]["amount"].as_str().unwrap_or("—"),
            value["price"]["currency_code"].as_str().unwrap_or("")
        ),
        badges,
        image_url,
        image_path: None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_explicit_states_coexist() {
        let row = vinted(
            &serde_json::json!({"is_hidden":true,"is_draft":false,"is_reserved":"true","is_closed":true,"is_processing":null}),
            Dataset::Products,
        );
        assert_eq!(row.badges, ["隐藏", "已关闭"]);
        assert!(
            vinted(&serde_json::json!({}), Dataset::Products)
                .badges
                .is_empty()
        );
    }
    #[test]
    fn blocked_and_delayed_require_evidence() {
        assert_eq!(vinted(&serde_json::json!({"transaction_permitted":false,"item_alert_type":"delayed_publication"}), Dataset::Products).badges, ["不可交易","延迟发布"]);
        assert!(
            vinted(
                &serde_json::json!({"transaction_permitted":true}),
                Dataset::Products
            )
            .badges
            .is_empty()
        );
    }
}
