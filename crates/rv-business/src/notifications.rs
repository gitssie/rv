//! Read-only platform notifications. Tasks persist responses; UI reads SQLite/cache only.
use crate::{StorePaths, vinted};
use rusqlite::{Connection, params};
use rv_core::{StoreError, workspace::*};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Notification {
    pub app_id: AppId,
    pub remote_id: String,
    pub title: String,
    pub body: String,
    pub note: String,
    pub created_at: Option<i64>,
    pub unread: Option<bool>,
    pub image_url: Option<String>,
    pub image_path: Option<PathBuf>,
    pub item_id: Option<String>,
    pub order_id: Option<String>,
    pub link: Option<String>,
}
fn schema(db: &Connection) -> Result<(), StoreError> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS vinted_notifications (
          app_id TEXT NOT NULL REFERENCES app_instances(id) ON DELETE CASCADE,
          remote_id TEXT NOT NULL, updated_at INTEGER, payload TEXT NOT NULL,
          PRIMARY KEY(app_id,remote_id)
         );
         CREATE INDEX IF NOT EXISTS vinted_notifications_updated
          ON vinted_notifications(app_id,updated_at DESC);",
    )?;
    Ok(())
}
fn identifier(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}
fn text(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| value[*key].as_str().filter(|s| !s.is_empty()))
        .map(crate::chat::notice::plain_text)
        .unwrap_or_default()
}
fn timestamp(row: &Value) -> Option<i64> {
    ["updated_at", "created_at"].iter().find_map(|key| {
        row[*key].as_i64().or_else(|| {
            chrono::DateTime::parse_from_rfc3339(row[*key].as_str()?)
                .ok()
                .map(|date| date.timestamp())
        })
    })
}
fn parse(app_id: &AppId, row: &Value) -> Result<Notification, StoreError> {
    let remote_id =
        identifier(&row["id"]).ok_or_else(|| StoreError::Invalid("通知缺少 ID".into()))?;
    let mut body = text(row, &["body", "message", "text"]);
    let mut title = text(row, &["title"]);
    if title.is_empty() {
        title = body
            .lines()
            .next()
            .unwrap_or("通知")
            .chars()
            .take(60)
            .collect();
    }
    // Unknown notification variants remain readable; no category allowlist drops them.
    if body.is_empty() {
        body = serde_json::to_string_pretty(row)?;
    }
    let image_url = row["photo"]["url"]
        .as_str()
        .or_else(|| row["photo"]["full_size_url"].as_str())
        .map(str::to_owned);
    Ok(Notification {
        app_id: app_id.clone(),
        remote_id,
        title,
        body,
        note: text(row, &["note", "subtitle"]),
        created_at: timestamp(row),
        unread: row["read"]
            .as_bool()
            .or_else(|| row["is_read"].as_bool())
            .map(|read| !read),
        image_url,
        image_path: None,
        item_id: identifier(&row["item_id"]),
        order_id: identifier(&row["order_id"]),
        link: row["link"].as_str().map(str::to_owned),
    })
}
pub(crate) fn image_urls(result: &Value) -> Vec<String> {
    result["response"]["notifications"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            row["photo"]["url"]
                .as_str()
                .or_else(|| row["photo"]["full_size_url"].as_str())
                .map(str::to_owned)
        })
        .collect()
}
pub(crate) fn persist(db: &Connection, app_id: &AppId, result: &Value) -> Result<(), StoreError> {
    if result["http_status"] != 200 {
        return Ok(());
    }
    schema(db)?;
    let rows = result["response"]["notifications"]
        .as_array()
        .ok_or_else(|| StoreError::Invalid("通知列表结构无效".into()))?;
    for row in rows {
        let payload = vinted::redact(row, 0);
        let notification = parse(app_id, &payload)?;
        db.execute(
            "INSERT INTO vinted_notifications(app_id,remote_id,updated_at,payload) VALUES(?1,?2,?3,?4)
             ON CONFLICT(app_id,remote_id) DO UPDATE SET updated_at=excluded.updated_at,payload=excluded.payload",
            params![app_id.as_str(), notification.remote_id, notification.created_at, payload.to_string()],
        )?;
    }
    Ok(())
}
pub fn list(paths: &StorePaths) -> Result<Vec<Notification>, String> {
    let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    store
        .business_transaction(|db| {
            schema(db)?;
            let mut statement = db.prepare(
                "SELECT n.app_id,n.payload FROM vinted_notifications n
             JOIN app_instances a ON a.id=n.app_id
             WHERE a.archived=0 AND a.bundle_id='lt.manodrabuziai.fr'
             ORDER BY n.updated_at DESC,n.remote_id DESC",
            )?;
            let mut rows = statement.query([])?;
            let mut output = Vec::new();
            while let Some(row) = rows.next()? {
                let app = AppId::from_idfv(&row.get::<_, String>(0)?)?;
                let payload: Value = serde_json::from_str(&row.get::<_, String>(1)?)?;
                let mut notification = parse(&app, &payload)?;
                notification.image_path = notification
                    .image_url
                    .as_deref()
                    .and_then(|url| crate::assets::cached(paths, url));
                output.push(notification);
            }
            Ok(output)
        })
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
