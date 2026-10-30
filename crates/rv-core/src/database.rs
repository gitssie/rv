//! Shared local database bootstrap. Legacy JSON and credential files are never read.
use crate::StoreError;
use rusqlite::Connection;
use std::{fs, path::Path, time::Duration};

pub(crate) fn open(path: &Path) -> Result<Connection, StoreError> {
    open_inner(path)
        .inspect_err(|error| tracing::error!(error=%error,"Local database initialization failed"))
}
fn open_inner(path: &Path) -> Result<Connection, StoreError> {
    let parent = path
        .parent()
        .ok_or_else(|| StoreError::Invalid("数据库路径无效".into()))?;
    fs::create_dir_all(parent)?;
    // SQLite derives permissions of WAL/SHM files from the database file.
    let file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    drop(file);
    let db = Connection::open(path)?;
    db.busy_timeout(Duration::from_secs(5))?;
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")?;
    let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version > 15 {
        return Err(StoreError::Invalid("数据库版本高于当前程序支持版本".into()));
    }
    if version == 0 {
        db.execute_batch(include_str!("schema.sql"))?;
    }
    if version < 2 {
        db.execute_batch(include_str!("workspace/offline.sql"))?;
    }
    if version < 3 {
        db.execute_batch(include_str!("workspace/proxies.sql"))?;
    }
    if version < 4 {
        db.execute_batch(include_str!("workspace/dismissed.sql"))?;
    }
    if version < 5 {
        db.execute_batch(include_str!("workspace/mailboxes.sql"))?;
    }
    if version < 6 {
        db.execute_batch(include_str!("workspace/ip_history.sql"))?;
    }
    if version < 7 {
        db.execute_batch(include_str!("workspace/apple_accounts.sql"))?;
    }
    if version < 8 {
        db.execute_batch(include_str!("workspace/tasks.sql"))?;
    }
    if version < 9 {
        db.execute_batch(include_str!("workspace/app_state/schema.sql"))?;
    }
    if version < 10 {
        initialize_tasks(&db)?;
    }
    if version < 11 {
        db.execute_batch(include_str!("workspace/execution.sql"))?;
    }
    if version < 12 {
        db.execute_batch("BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS proxy_engine_settings(id INTEGER PRIMARY KEY CHECK(id=1), executable TEXT NOT NULL); PRAGMA user_version=12; COMMIT;")?;
    }
    if version < 13 {
        db.execute_batch("BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS app_request_sessions(app_id TEXT NOT NULL REFERENCES app_instances(id) ON DELETE CASCADE,access_token_sha256 BLOB NOT NULL,session_id TEXT NOT NULL,PRIMARY KEY(app_id,access_token_sha256)); PRAGMA user_version=13; COMMIT;")?;
    }
    if version < 14 {
        db.execute_batch("BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS app_token_sources(app_id TEXT PRIMARY KEY REFERENCES app_instances(id) ON DELETE CASCADE,device_id TEXT REFERENCES devices(id) ON DELETE SET NULL); PRAGMA user_version=14; COMMIT;")?;
    }
    if version < 15 {
        db.execute_batch(include_str!("workspace/login_credentials.sql"))?;
    }
    Ok(db)
}

/// This feature has no legacy task data to migrate. Only an empty intermediate
/// task table can be rebuilt; unrelated tables and nonempty tasks are preserved.
fn initialize_tasks(db: &Connection) -> Result<(), StoreError> {
    db.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| {
        let columns = db
            .prepare("PRAGMA table_info(tasks)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        let complete = ["task_key", "depends_on", "enabled"]
            .iter()
            .all(|name| columns.iter().any(|column| column == name));
        if !complete && !columns.is_empty() {
            let count: i64 = db.query_row("SELECT count(*) FROM tasks", [], |r| r.get(0))?;
            if count != 0 {
                return Err(StoreError::Invalid(
                    "任务表结构不匹配且包含记录，已保留原数据，无法自动重建".into(),
                ));
            }
            tracing::warn!("Reinitializing empty intermediate task table");
            db.execute_batch("DROP TABLE tasks")?;
        }
        let schema = include_str!("workspace/tasks.sql")
            .replace("BEGIN IMMEDIATE;", "")
            .replace("PRAGMA user_version=8;", "")
            .replace("COMMIT;", "");
        db.execute_batch(&schema)?;
        db.pragma_update(None, "user_version", 10)?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            db.execute_batch("COMMIT")?;
            Ok(())
        }
        Err(error) => {
            let _ = db.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}
