use super::*;
pub struct VintedStrategy;
impl AppStrategy for VintedStrategy {
    fn project(&self, app: &AppInstance, _backup_count: usize) -> CardDetails {
        let data: serde_json::Value = serde_json::from_str(&app.metadata).unwrap_or_default();
        let facts = vec![
            CardFact {
                label: "会话".into(),
                value: data["session_status"].as_str().unwrap_or("尚未检测").into(),
            },
            CardFact {
                label: "未读".into(),
                value: data["unread_count"]
                    .as_u64()
                    .map(|count| count.to_string())
                    .unwrap_or("—".into()),
            },
        ];
        let app_name = data["market"]
            .as_str()
            .or_else(|| data["profile"]["market"].as_str())
            .filter(|s| !s.is_empty())
            .map(|s| format!("Vinted · {s}"))
            .unwrap_or("Vinted".into());
        CardDetails {
            app_name,
            symbol: "V",
            accent: 0x007e8d,
            facts,
            action_label: "查看店铺",
        }
    }
}
