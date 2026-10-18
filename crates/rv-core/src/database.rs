//! Shared local database bootstrap. Legacy JSON and credential files are never read.
use crate::StoreError;
use rusqlite::Connection;
use std::{fs, path::Path, time::Duration};

pub(crate) fn open(path: &Path) -> Result<Connection, StoreError> {
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
    if version > 5 {
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
    Ok(db)
}
