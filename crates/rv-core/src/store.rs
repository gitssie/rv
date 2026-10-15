//! Address book facade over SQLite. Password and device repositories are separate modules.
use crate::{Connection, ConnectionId, Preferences};
use std::{fs, path::PathBuf};
use thiserror::Error;
pub(crate) mod credentials;
mod devices;
#[cfg(test)]
mod tests;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("数据库错误：{0}")]
    Database(#[from] rusqlite::Error),
    #[error("saved password error: {0}")]
    Credential(String),
    #[error("unknown connection")]
    UnknownConnection,
    #[error("{0}")]
    Invalid(String),
    #[error("记录已在另一窗口修改，请重新载入")]
    Conflict,
}

#[derive(Debug, Clone)]
pub struct StorePaths {
    pub root: PathBuf,
    pub database: PathBuf,
    /// Derived preview cache, not configuration or credentials.
    pub thumbs: PathBuf,
}
impl StorePaths {
    pub fn default_dir() -> Result<Self, StoreError> {
        if let Some(dir) = std::env::var_os("RV_DATA_DIR").filter(|d| !d.is_empty()) {
            return Ok(Self::in_dir(PathBuf::from(dir)));
        }
        let dirs = directories::ProjectDirs::from("app", "RV", "rv").ok_or_else(|| {
            StoreError::Io(std::io::Error::other("cannot resolve application data dir"))
        })?;
        Ok(Self::in_dir(dirs.data_dir().to_path_buf()))
    }
    pub fn in_dir(root: PathBuf) -> Self {
        Self {
            database: root.join("rv.sqlite3"),
            thumbs: root.join("thumbs"),
            root,
        }
    }
    pub fn ensure(&self) -> Result<(), StoreError> {
        fs::create_dir_all(&self.root)?;
        fs::create_dir_all(&self.thumbs)?;
        Ok(())
    }
    pub fn thumb_path(&self, id: ConnectionId) -> PathBuf {
        self.thumbs.join(format!("{id}.png"))
    }
}

#[derive(Debug, Clone)]
pub struct AddressBook {
    paths: StorePaths,
    connections: Vec<Connection>,
    prefs: Preferences,
}

impl AddressBook {
    /// Load strictly: any unreadable file is an error.
    pub fn load(paths: StorePaths) -> Result<Self, StoreError> {
        paths.ensure()?;
        let db = crate::database::open(&paths.database)?;
        let connections = devices::load(&db)?;
        let prefs = devices::load_prefs(&db)?;
        Ok(Self {
            paths,
            connections,
            prefs,
        })
    }

    pub fn paths(&self) -> &StorePaths {
        &self.paths
    }

    pub fn connections(&self) -> &[Connection] {
        &self.connections
    }

    pub fn prefs(&self) -> &Preferences {
        &self.prefs
    }

    pub fn prefs_mut(&mut self) -> &mut Preferences {
        &mut self.prefs
    }

    pub fn get(&self, id: ConnectionId) -> Option<&Connection> {
        self.connections.iter().find(|c| c.id == id)
    }

    pub fn get_mut(&mut self, id: ConnectionId) -> Option<&mut Connection> {
        self.connections.iter_mut().find(|c| c.id == id)
    }

    pub fn upsert(&mut self, conn: Connection) {
        if let Some(existing) = self.connections.iter_mut().find(|c| c.id == conn.id) {
            *existing = conn;
        } else {
            self.connections.push(conn);
        }
    }

    pub fn remove(&mut self, id: ConnectionId) -> Result<(), StoreError> {
        if !self.connections.iter().any(|c| c.id == id) {
            return Err(StoreError::UnknownConnection);
        }
        let mut db = crate::database::open(&self.paths.database)?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM credentials WHERE id=?1", [id.to_string()])?;
        tx.execute("DELETE FROM devices WHERE id=?1", [id.to_string()])?;
        tx.commit()?;
        self.connections.retain(|c| c.id != id);
        let thumb = self.paths.thumb_path(id);
        let _ = fs::remove_file(thumb);
        Ok(())
    }

    pub fn labels(&self) -> Vec<String> {
        let mut labels: Vec<String> = self
            .connections
            .iter()
            .flat_map(|c| c.labels.iter().cloned())
            .collect();
        labels.sort();
        labels.dedup();
        labels
    }

    pub fn recents(&self, n: usize) -> Vec<Connection> {
        let mut list: Vec<Connection> = self
            .connections
            .iter()
            .filter(|c| c.last_connected.is_some())
            .cloned()
            .collect();
        list.sort_by_key(|b| std::cmp::Reverse(b.last_connected));
        list.truncate(n);
        list
    }

    pub fn filtered(&self, query: &str, label: Option<&str>) -> Vec<Connection> {
        let q = query.trim().to_ascii_lowercase();
        self.connections
            .iter()
            .filter(|c| {
                if let Some(label) = label
                    && !c.labels.iter().any(|l| l == label)
                {
                    return false;
                }
                if q.is_empty() {
                    return true;
                }
                c.name.to_ascii_lowercase().contains(&q)
                    || c.host.to_ascii_lowercase().contains(&q)
                    || c.server_display().to_ascii_lowercase().contains(&q)
                    || c.labels.iter().any(|l| l.to_ascii_lowercase().contains(&q))
            })
            .cloned()
            .collect()
    }

    pub fn save(&self) -> Result<(), StoreError> {
        self.paths.ensure()?;
        let mut db = crate::database::open(&self.paths.database)?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        devices::save(&tx, &self.connections)?;
        devices::save_prefs(&tx, &self.prefs)?;
        tx.commit()?;
        Ok(())
    }

    pub fn forget_sensitive(&mut self) -> Result<(), StoreError> {
        for conn in &self.connections {
            self.paths.delete_credentials(conn.id)?;
            let _ = fs::remove_file(self.paths.thumb_path(conn.id));
        }
        for conn in &mut self.connections {
            conn.remember_password = false;
        }
        self.prefs.hide_screenshots = true;
        self.save()
    }
}
