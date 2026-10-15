//! Typed device and preference persistence, independent of business objects.
use crate::{Connection, ConnectionId, Preferences, StoreError};
use rusqlite::{Connection as Db, OptionalExtension, params};
const COLUMNS: &str = "id,guid,name,host,port,model,os_version,current_bundle_id,username,remember_password,encryption,quality,clipboard,local_cursor,view_only,zxtouch_port,shared,last_connected,labels,function_favorites,transfer_folders,app_shortcuts,environment_apps";

pub(super) fn load(db: &Db) -> Result<Vec<Connection>, StoreError> {
    let mut stmt = db.prepare(&format!("SELECT {COLUMNS} FROM devices ORDER BY rowid"))?;
    let mut rows = stmt.query([])?;
    let mut result = Vec::new();
    while let Some(r) = rows.next()? {
        let id: String = r.get(0)?;
        let decode = |i| -> Result<serde_json::Value, StoreError> {
            Ok(serde_json::from_str(&r.get::<_, String>(i)?)?)
        };
        result.push(Connection {
            id: ConnectionId(
                uuid::Uuid::parse_str(&id)
                    .map_err(|_| StoreError::Invalid("设备主键无效".into()))?,
            ),
            guid: r.get(1)?,
            name: r.get(2)?,
            host: r.get(3)?,
            port: r.get(4)?,
            model: r.get(5)?,
            os_version: r.get(6)?,
            current_bundle_id: r.get(7)?,
            username: r.get(8)?,
            remember_password: r.get(9)?,
            encryption: serde_json::from_value(decode(10)?)?,
            quality: serde_json::from_value(decode(11)?)?,
            clipboard: serde_json::from_value(decode(12)?)?,
            local_cursor: serde_json::from_value(decode(13)?)?,
            view_only: r.get(14)?,
            zxtouch_port: r.get(15)?,
            shared: r.get(16)?,
            last_connected: r.get(17)?,
            labels: serde_json::from_value(decode(18)?)?,
            function_favorites: serde_json::from_value(decode(19)?)?,
            transfer_folders: serde_json::from_value(decode(20)?)?,
            app_shortcuts: serde_json::from_value(decode(21)?)?,
            environment_apps: serde_json::from_value(decode(22)?)?,
        });
    }
    Ok(result)
}
pub(super) fn save(db: &Db, connections: &[Connection]) -> Result<(), StoreError> {
    let updates = COLUMNS
        .split(',')
        .skip(1)
        .map(|c| format!("{c}=excluded.{c}"))
        .collect::<Vec<_>>()
        .join(",");
    let placeholders = (1..=23)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let mut stmt = db.prepare(&format!("INSERT INTO devices({COLUMNS}) VALUES({placeholders}) ON CONFLICT(id) DO UPDATE SET {updates}"))?;
    for c in connections {
        stmt.execute(params![
            c.id.to_string(),
            c.guid,
            c.name,
            c.host,
            c.port,
            c.model,
            c.os_version,
            c.current_bundle_id,
            c.username,
            c.remember_password,
            serde_json::to_string(&c.encryption)?,
            serde_json::to_string(&c.quality)?,
            serde_json::to_string(&c.clipboard)?,
            serde_json::to_string(&c.local_cursor)?,
            c.view_only,
            c.zxtouch_port,
            c.shared,
            c.last_connected,
            serde_json::to_string(&c.labels)?,
            serde_json::to_string(&c.function_favorites)?,
            serde_json::to_string(&c.transfer_folders)?,
            serde_json::to_string(&c.app_shortcuts)?,
            serde_json::to_string(&c.environment_apps)?
        ])?;
    }
    Ok(())
}
pub(super) fn load_prefs(db: &Db) -> Result<Preferences, StoreError> {
    let row = db.query_row("SELECT theme,hide_screenshots,menu_key,default_scale,pin_toolbar FROM preferences WHERE id=1", [],
        |r| Ok((r.get::<_,String>(0)?, r.get(1)?, r.get(2)?,r.get::<_,String>(3)?,r.get(4)?))).optional()?;
    match row {
        Some((theme, hide_screenshots, menu_key, scale, pin_toolbar)) => Ok(Preferences {
            theme: serde_json::from_str(&theme)?,
            hide_screenshots,
            menu_key,
            default_scale: serde_json::from_str(&scale)?,
            pin_toolbar,
        }),
        None => Ok(Preferences::default()),
    }
}
pub(super) fn save_prefs(db: &Db, p: &Preferences) -> Result<(), StoreError> {
    db.execute("INSERT INTO preferences VALUES(1,?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET theme=excluded.theme,hide_screenshots=excluded.hide_screenshots,menu_key=excluded.menu_key,default_scale=excluded.default_scale,pin_toolbar=excluded.pin_toolbar",
        params![serde_json::to_string(&p.theme)?,p.hide_screenshots,p.menu_key,serde_json::to_string(&p.default_scale)?,p.pin_toolbar])?;
    Ok(())
}
