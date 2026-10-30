//! Bundle-owned projection: explicit visibility and tradeability remain independent;
//! absent fields never imply a state.
use crate::data::Dataset;
use serde_json::Value;
use std::path::PathBuf;
#[derive(Clone, Debug, Default)]
pub struct RowVisual {
    pub price: String,
    pub badges: Vec<String>,
    pub view_count: Option<u64>,
    pub favourite_count: Option<u64>,
    pub image_url: Option<String>,
    pub image_path: Option<PathBuf>,
}
pub(crate) fn vinted(value: &Value, dataset: Dataset) -> RowVisual {
    let mut badges = Vec::new();
    if dataset==Dataset::Products && value["_rv_deleted"]==true {
        badges.push("已删除".into());
    } else if dataset == Dataset::Products {
        match value["is_hidden"].as_bool() {
            Some(true) => badges.push("隐藏".into()),
            Some(false) => badges.push("激活".into()),
            None => {}
        }
        for (field, label) in [
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
        match value["transaction_permitted"].as_bool() {
            Some(false) => badges.push("不可交易".into()),
            Some(true) => badges.push("可交易".into()),
            None => {}
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
        view_count: (dataset == Dataset::Products).then(|| value["view_count"].as_u64()).flatten(),
        favourite_count: (dataset == Dataset::Products).then(|| value["favourite_count"].as_u64()).flatten(),
        image_url,
        image_path: None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn product_counts_preserve_zero_and_missing_fields() {
        let row=vinted(&serde_json::json!({"view_count":0,"favourite_count":12}),Dataset::Products);
        assert_eq!(row.view_count,Some(0));
        assert_eq!(row.favourite_count,Some(12));
        let missing=vinted(&serde_json::json!({"favourite_count":0}),Dataset::Products);
        assert_eq!(missing.view_count,None);
        assert_eq!(missing.favourite_count,Some(0));
        for value in [serde_json::json!({}),serde_json::json!({"view_count":null,"favourite_count":null}),serde_json::json!({"view_count":-1,"favourite_count":"4"})] {
            let row=vinted(&value,Dataset::Products);
            assert_eq!(row.view_count,None);
            assert_eq!(row.favourite_count,None);
        }
        let order=vinted(&serde_json::json!({"view_count":2,"favourite_count":3}),Dataset::Orders);
        assert_eq!(order.view_count,None);
        assert_eq!(order.favourite_count,None);
    }
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
    fn transaction_and_delayed_states_require_evidence() {
        assert_eq!(vinted(&serde_json::json!({"transaction_permitted":false,"item_alert_type":"delayed_publication"}), Dataset::Products).badges, ["不可交易","延迟发布"]);
        assert_eq!(vinted(&serde_json::json!({"transaction_permitted":true}), Dataset::Products).badges,["可交易"]);
        assert!(vinted(&serde_json::json!({"transaction_permitted":null}), Dataset::Products).badges.is_empty());
        assert_eq!(vinted(&serde_json::json!({"is_hidden":false,"transaction_permitted":true}), Dataset::Products).badges,["激活","可交易"]);
        assert_eq!(vinted(&serde_json::json!({"is_hidden":false,"transaction_permitted":false}), Dataset::Products).badges,["激活","不可交易"]);
    }
}
