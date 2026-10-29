use super::*;
use crate::{ConnectionId, StoreError, StorePaths};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

const COLUMNS: &str = "a.id,a.bundle_id,a.title,a.identity_key,a.tags,a.note,a.metadata,a.favorite,a.pinned,a.archived,a.position,a.group_name,a.created_at,a.updated_at,a.last_used_at,a.revision";
pub struct WorkspaceStore {
    pub(super) db: Connection,
    pub(super) changes: super::changes::ChangeTracker,
    pub(super) events: crate::events::EventBus,
}
impl Drop for WorkspaceStore {
    fn drop(&mut self) {
        self.publish_changes();
    }
}
impl WorkspaceStore {
    pub(super) fn publish_changes(&self) {
        self.changes.publish(&self.db, &self.events);
    }
    /// Trusted Bundle plugins own their schema and repositories in this database.
    /// The callback must not start/commit transactions or perform network I/O.
    pub fn business_transaction<T>(
        &mut self,
        action: impl FnOnce(&Connection) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let result = action(&tx)?;
        tx.commit()?;
        self.publish_changes();
        Ok(result)
    }
    pub fn open(paths: &StorePaths) -> Result<Self, StoreError> {
        let db = crate::database::open(&paths.database)?;
        let changes = super::changes::ChangeTracker::install(&db)?;
        let events = crate::events::workspace_bus(paths)?;
        Ok(Self {
            db,
            changes,
            events,
        })
    }
    pub fn delete_app(&mut self, id: &AppId) -> Result<(), StoreError> {
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT OR IGNORE INTO app_dismissed(id) VALUES(?1)",
            [id.as_str()],
        )?;
        tx.execute("DELETE FROM app_instances WHERE id=?1", [id.as_str()])?;
        tx.commit()?;
        self.publish_changes();
        Ok(())
    }
    pub fn load(&self, id: &AppId) -> Result<AppInstance, StoreError> {
        let mut stmt = self.db.prepare(&format!(
            "SELECT {COLUMNS} FROM app_instances a WHERE a.id=?1"
        ))?;
        let mut rows = stmt.query([id.as_str()])?;
        read(
            rows.next()?
                .ok_or_else(|| StoreError::Invalid("对象不存在".into()))?,
        )
    }
    /// Filter choices remain available independently of the current search.
    pub fn tags(&self) -> Result<Vec<String>, StoreError> {
        let mut statement = self.db.prepare(
            "SELECT DISTINCT tag.value FROM app_instances a, json_each(a.tags) tag ORDER BY tag.value COLLATE NOCASE, tag.value",
        )?;
        let tags = statement.query_map([], |row| row.get(0))?;
        tags.collect::<Result<_, _>>().map_err(Into::into)
    }
    pub fn list(&self, query: &AppQuery<'_>) -> Result<Vec<AppInstance>, StoreError> {
        let text = query.text.trim();
        // MATCH literals cannot inject FTS operators. Short Chinese queries use
        // LIKE on the FTS table, since the trigram index needs three characters.
        let search = if text.is_empty() {
            "1"
        } else if text.chars().count() >= 3 {
            "a.id IN (SELECT app_id FROM app_search WHERE app_search MATCH ?1)"
        } else {
            "a.id IN (SELECT app_id FROM app_search WHERE title LIKE ?1 ESCAPE '\\' OR bundle_id LIKE ?1 ESCAPE '\\' OR tags LIKE ?1 ESCAPE '\\' OR note LIKE ?1 ESCAPE '\\' OR metadata LIKE ?1 ESCAPE '\\' OR group_name LIKE ?1 ESCAPE '\\')"
        };
        let pattern = if text.chars().count() >= 3 {
            format!("\"{}\"", text.replace('"', "\"\""))
        } else {
            format!(
                "%{}%",
                text.replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            )
        };
        let filter = match query.filter {
            AppFilter::All => "a.archived=0",
            AppFilter::Favorites => "a.archived=0 AND a.favorite=1",
            AppFilter::Recent => "a.archived=0 AND a.last_used_at IS NOT NULL",
            AppFilter::Archived => "a.archived=1",
        };
        let order = match query.sort {
            AppSort::Updated => "a.updated_at DESC",
            AppSort::Title => "a.title COLLATE NOCASE",
            AppSort::LastUsed => "a.last_used_at DESC",
        };
        let mut stmt = self.db.prepare(&format!("SELECT {COLUMNS} FROM app_instances a WHERE {search} AND {filter}
            AND (?2 IS NULL OR EXISTS(SELECT 1 FROM json_each(a.tags) WHERE value=?2))
            AND (?3 IS NULL OR EXISTS(SELECT 1 FROM app_devices d WHERE d.app_id=a.id AND d.device_id=?3))
            ORDER BY a.pinned DESC,a.position,{order},a.id"))?;
        // ?1 must exist even when the text is empty so numbered bindings stay stable.
        let mut rows = stmt.query(params![
            pattern,
            query.tag,
            query.device_id.map(|v| v.to_string())
        ])?;
        let mut apps = Vec::new();
        while let Some(row) = rows.next()? {
            apps.push(read(row)?);
        }
        Ok(apps)
    }
    /// Edit existing objects only. Creation is exclusively via phone backups.
    pub fn save(&mut self, app: &AppInstance) -> Result<AppInstance, StoreError> {
        if app.title.trim().is_empty()
            || app.title.len() > 4096
            || app.note.len() > 65536
            || app.tags.len() > 128
            || app.tags.iter().any(|t| t.len() > 1024)
            || app.metadata.len() > 1024 * 1024
        {
            return Err(StoreError::Invalid("标题为空或字段过长".into()));
        }
        let _: Value = serde_json::from_str(&app.metadata)?;
        let mut result = app.clone();
        result.title = app.title.trim().to_owned();
        result.tags = app
            .tags
            .iter()
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect();
        result.tags.sort();
        result.tags.dedup();
        result.updated_at = now();
        result.revision += 1;
        if self.db.execute("UPDATE app_instances SET title=?2,identity_key=?3,tags=?4,note=?5,metadata=?6,favorite=?7,pinned=?8,archived=?9,position=?10,group_name=?11,updated_at=?12,last_used_at=CASE WHEN ?13 IS NULL THEN last_used_at WHEN last_used_at IS NULL OR ?13>last_used_at THEN ?13 ELSE last_used_at END,revision=?14 WHERE id=?1 AND bundle_id=?15 AND revision=?16",
            params![result.id.as_str(),result.title,result.identity_key,serde_json::to_string(&result.tags)?,result.note,result.metadata,result.favorite,result.pinned,result.archived,result.position,result.group_name,result.updated_at,result.last_used_at,result.revision,result.bundle_id,app.revision])?!=1 {
            return Err(StoreError::Conflict);
        }
        result.last_used_at = self.db.query_row(
            "SELECT last_used_at FROM app_instances WHERE id=?1",
            [result.id.as_str()],
            |row| row.get(0),
        )?;
        self.publish_changes();
        Ok(result)
    }
    /// Browsing is usage, not a content edit. Preserve update time and revision.
    pub fn record_use(&self, id: &AppId, timestamp: i64) -> Result<(), StoreError> {
        if self.db.execute(
            "UPDATE app_instances SET last_used_at=?2 WHERE id=?1",
            params![id.as_str(), timestamp],
        )? != 1
        {
            return Err(StoreError::Invalid("App 不存在".into()));
        }
        self.publish_changes();
        Ok(())
    }
    pub fn backups(&self, id: &AppId) -> Result<Vec<BackupLink>, StoreError> {
        let mut stmt=self.db.prepare("SELECT device_id,backup_id,title,metadata FROM app_backups WHERE app_id=?1 ORDER BY device_id,backup_id")?;
        let mut rows = stmt.query([id.as_str()])?;
        let mut items = Vec::new();
        while let Some(row) = rows.next()? {
            items.push(BackupLink {
                app_id: id.clone(),
                device_id: ConnectionId(
                    uuid::Uuid::parse_str(&row.get::<_, String>(0)?)
                        .map_err(|_| StoreError::Invalid("设备主键无效".into()))?,
                ),
                backup_id: row.get(1)?,
                title: row.get(2)?,
                metadata: row.get(3)?,
            });
        }
        Ok(items)
    }
    /// Atomically reconcile a successful, complete App backup list from one phone.
    /// Keeps user tags/notes and business metadata when the same IDFV is seen again.
    pub fn sync_backups(
        &mut self,
        device: ConnectionId,
        bundle: &str,
        result: &Value,
    ) -> Result<SyncResult, StoreError> {
        self.sync_backups_inner(device, bundle, result, None)
    }
    /// Explicitly re-import just the selected phone snapshot. Background sync
    /// continues to respect all other local deletion/dismissal records.
    pub fn reimport_backup(
        &mut self,
        device: ConnectionId,
        bundle: &str,
        result: &Value,
        id: &AppId,
        backup: &str,
    ) -> Result<SyncResult, StoreError> {
        if backup != format!("snapshot:{}", id.as_str())
            || !result["backups"].as_array().is_some_and(|rows| {
                rows.iter().any(|row| {
                    row["kind"] == "snapshot"
                        && row["backup_id"].as_str() == Some(backup)
                        && row["idfv"]
                            .as_str()
                            .and_then(|v| AppId::from_idfv(v).ok())
                            .as_ref()
                            == Some(id)
                })
            })
        {
            return Err(StoreError::Invalid(
                "所选备份不在当前手机列表中，请先刷新备份列表".into(),
            ));
        }
        self.sync_backups_inner(device, bundle, result, Some(id))
    }
    fn sync_backups_inner(
        &mut self,
        device: ConnectionId,
        bundle: &str,
        result: &Value,
        reimport: Option<&AppId>,
    ) -> Result<SyncResult, StoreError> {
        if result["bundle_id"].as_str() != Some(bundle) || bundle.is_empty() {
            return Err(StoreError::Invalid("备份回复与 App 不匹配".into()));
        }
        let rows = result["backups"]
            .as_array()
            .ok_or_else(|| StoreError::Invalid("备份列表缺失".into()))?;
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM devices WHERE id=?1)",
            [device.to_string()],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(StoreError::UnknownConnection);
        }
        if let Some(id) = reimport {
            tx.execute("DELETE FROM app_dismissed WHERE id=?1", [id.as_str()])?;
        }
        tx.execute("INSERT INTO phone_app_state VALUES(?1,?2,?3,?4) ON CONFLICT(device_id,bundle_id) DO UPDATE SET result=excluded.result,updated_at=excluded.updated_at",params![device.to_string(),bundle,serde_json::to_string(result)?,now()])?;
        tx.execute("DELETE FROM app_backups WHERE device_id=?1 AND app_id IN(SELECT id FROM app_instances WHERE bundle_id=?2)",params![device.to_string(),bundle])?;
        tx.execute("DELETE FROM app_devices WHERE device_id=?1 AND app_id IN(SELECT id FROM app_instances WHERE bundle_id=?2)",params![device.to_string(),bundle])?;
        let mut report = SyncResult::default();
        for row in rows {
            let kind = row["kind"]
                .as_str()
                .ok_or_else(|| StoreError::Invalid("备份类型缺失".into()))?;
            if kind == "baseline" {
                report.baselines += 1;
                continue;
            }
            if kind != "snapshot" {
                return Err(StoreError::Invalid("未知备份类型".into()));
            }
            let backup = row["backup_id"]
                .as_str()
                .filter(|v| v.starts_with("snapshot:") && v.len() > 9)
                .ok_or_else(|| StoreError::Invalid("备份标识无效".into()))?;
            let id = match row["idfv"].as_str().map(AppId::from_idfv).transpose() {
                Ok(Some(id)) => id,
                _ => {
                    report.missing_idfv += 1;
                    continue;
                }
            };
            if tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM app_dismissed WHERE id=?1)",
                [id.as_str()],
                |row| row.get::<_, bool>(0),
            )? {
                continue;
            }
            let old: Option<String> = tx
                .query_row(
                    "SELECT bundle_id FROM app_instances WHERE id=?1",
                    [id.as_str()],
                    |r| r.get(0),
                )
                .optional()?;
            if old.as_deref().is_some_and(|v| v != bundle) {
                return Err(StoreError::Invalid(
                    "同一 IDFV 对应不同 App，无法合并".into(),
                ));
            }
            let title = row["display_name"]
                .as_str()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or(backup);
            if old.is_none() {
                tx.execute("INSERT INTO app_instances(id,bundle_id,title,note,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?5)",params![id.as_str(),bundle,title,row["note"].as_str().unwrap_or(""),now()])?;
                report.created += 1;
            }
            tx.execute(
                "INSERT OR IGNORE INTO app_devices VALUES(?1,?2)",
                params![id.as_str(), device.to_string()],
            )?;
            tx.execute("INSERT INTO app_backups VALUES(?1,?2,?3,?4,?5) ON CONFLICT(app_id,device_id,backup_id) DO UPDATE SET title=excluded.title,metadata=excluded.metadata",params![id.as_str(),device.to_string(),backup,title,serde_json::to_string(row)?])?;
            report.linked += 1;
        }
        tx.commit()?;
        self.publish_changes();
        Ok(report)
    }
    pub fn phone_state(
        &self,
        device: ConnectionId,
        bundle: &str,
    ) -> Result<Option<Value>, StoreError> {
        let value: Option<String> = self
            .db
            .query_row(
                "SELECT result FROM phone_app_state WHERE device_id=?1 AND bundle_id=?2",
                params![device.to_string(), bundle],
                |r| r.get(0),
            )
            .optional()?;
        value
            .map(|v| serde_json::from_str(&v))
            .transpose()
            .map_err(Into::into)
    }
}
fn read(row: &rusqlite::Row<'_>) -> Result<AppInstance, StoreError> {
    Ok(AppInstance {
        id: AppId::from_idfv(&row.get::<_, String>(0)?)?,
        bundle_id: row.get(1)?,
        title: row.get(2)?,
        identity_key: row.get(3)?,
        tags: serde_json::from_str(&row.get::<_, String>(4)?)?,
        note: row.get(5)?,
        metadata: row.get(6)?,
        favorite: row.get(7)?,
        pinned: row.get(8)?,
        archived: row.get(9)?,
        position: row.get(10)?,
        group_name: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
        last_used_at: row.get(14)?,
        revision: row.get(15)?,
    })
}
