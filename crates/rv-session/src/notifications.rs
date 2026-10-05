//! Bounded notification snapshots received from the optional SpringBoard adapter.
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct PhoneNotification {
    pub id: String,
    pub source: String,
    pub title: String,
    pub subtitle: String,
    pub message: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct NotificationSnapshot {
    pub revision: String,
    pub notifications: Vec<PhoneNotification>,
}

impl NotificationSnapshot {
    pub fn parse(value: serde_json::Value) -> Result<Self, String> {
        let snapshot: Self = serde_json::from_value(value)
            .map_err(|_| "Invalid notification snapshot".to_string())?;
        if snapshot.revision.is_empty()
            || snapshot.revision.len() > 128
            || snapshot.notifications.len() > 100
            || snapshot.notifications.iter().any(|n| {
                n.id.is_empty()
                    || n.id.len() > 512
                    || n.source.len() > 255
                    || n.title.len() > 1024
                    || n.subtitle.len() > 1024
                    || n.message.len() > 4096
                    || n.timestamp < 0
            })
        {
            return Err("Notification snapshot exceeds limits".into());
        }
        let mut ids = std::collections::HashSet::new();
        if snapshot.notifications.iter().any(|n| !ids.insert(&n.id)) {
            return Err("Duplicate notification identifiers".into());
        }
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn accepts_system_sources_and_rejects_duplicate_or_oversized_entries() {
        let n = json!({"id":"call-1", "source":"com.apple.mobilephone", "title":"来电",
            "subtitle":"", "message":"测试", "timestamp":1});
        assert!(
            NotificationSnapshot::parse(json!({"revision":"1", "notifications":[n.clone()]}))
                .is_ok()
        );
        assert!(
            NotificationSnapshot::parse(
                json!({"revision":"1", "notifications":[n.clone(), n.clone()]})
            )
            .is_err()
        );
        let mut large = n.clone();
        large["message"] = json!("x".repeat(4097));
        assert!(
            NotificationSnapshot::parse(json!({"revision":"1", "notifications":[large]})).is_err()
        );
        assert!(
            NotificationSnapshot::parse(json!({"revision":"1", "notifications":vec![n;101]}))
                .is_err()
        );
    }
}
