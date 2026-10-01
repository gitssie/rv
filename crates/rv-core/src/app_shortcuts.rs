use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppShortcut {
    pub bundle_id: String,
    pub name: String,
}

pub fn default_app_shortcuts() -> Vec<AppShortcut> {
    [
        ("com.apple.mobilesafari", "Safari"),
        ("com.apple.mobileslideshow", "照片"),
        ("com.apple.Preferences", "设置"),
        ("com.apple.AppStore", "App Store"),
    ]
    .into_iter()
    .map(|(id, name)| AppShortcut {
        bundle_id: id.into(),
        name: name.into(),
    })
    .collect()
}

/// Insert before a target; append when dropped on the end of the shortcut strip.
pub fn move_app_shortcut(
    shortcuts: &mut Vec<AppShortcut>,
    source: &str,
    before: Option<&str>,
) -> bool {
    if before == Some(source) {
        return false;
    }
    let Some(from) = shortcuts.iter().position(|item| item.bundle_id == source) else {
        return false;
    };
    if before.is_some_and(|id| !shortcuts.iter().any(|item| item.bundle_id == id)) {
        return false;
    }
    let item = shortcuts.remove(from);
    let to = before
        .and_then(|id| shortcuts.iter().position(|item| item.bundle_id == id))
        .unwrap_or(shortcuts.len());
    shortcuts.insert(to, item);
    from != to
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connection_migration_preserves_explicit_empty_favorites() {
        let conn = crate::Connection::new("test", "localhost", 5900);
        let mut value = serde_json::to_value(&conn).unwrap();
        value.as_object_mut().unwrap().remove("app_shortcuts");
        let migrated: crate::Connection = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(migrated.app_shortcuts, default_app_shortcuts());
        assert_eq!(
            migrated
                .app_shortcuts
                .iter()
                .map(|app| app.bundle_id.as_str())
                .collect::<Vec<_>>(),
            [
                "com.apple.mobilesafari",
                "com.apple.mobileslideshow",
                "com.apple.Preferences",
                "com.apple.AppStore"
            ]
        );
        value["app_shortcuts"] =
            serde_json::json!([{ "bundle_id": "com.example.custom", "name": "Custom" }]);
        let custom: crate::Connection = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(custom.app_shortcuts[0].bundle_id, "com.example.custom");
        value["app_shortcuts"] = serde_json::json!([]);
        let empty: crate::Connection = serde_json::from_value(value).unwrap();
        assert!(empty.app_shortcuts.is_empty());
        let restored: crate::Connection =
            serde_json::from_str(&serde_json::to_string(&empty).unwrap()).unwrap();
        assert!(restored.app_shortcuts.is_empty());
    }
    #[test]
    fn reorder_handles_forward_backward_end_and_invalid_targets() {
        let mut apps = default_app_shortcuts();
        let first = apps[0].bundle_id.clone();
        let last = apps.last().unwrap().bundle_id.clone();
        assert!(move_app_shortcut(&mut apps, &first, None));
        assert_eq!(apps.last().unwrap().bundle_id, first);
        assert!(move_app_shortcut(&mut apps, &first, Some(&last)));
        assert!(!move_app_shortcut(&mut apps, &first, Some(&first)));
        assert!(!move_app_shortcut(&mut apps, &first, Some("missing")));
        assert_eq!(apps.len(), 4);
    }
}
