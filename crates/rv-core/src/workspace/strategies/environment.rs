use super::*;
pub struct EnvironmentStrategy;
impl AppStrategy for EnvironmentStrategy {
    fn project(&self, app: &AppInstance, _backup_count: usize) -> CardDetails {
        let metadata: serde_json::Value = serde_json::from_str(&app.metadata).unwrap_or_default();
        let name = metadata["app_name"].as_str().unwrap_or(&app.bundle_id);
        CardDetails {
            app_name: name.into(),
            symbol: "A",
            accent: 0x3389f5,
            facts: vec![],
            action_label: "查看环境",
        }
    }
}
