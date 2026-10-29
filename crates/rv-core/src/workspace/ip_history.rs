use super::WorkspaceStore;
use crate::StoreError;
use rusqlite::params;
use std::net::Ipv4Addr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpHistoryEntry {
    pub ip: Ipv4Addr,
    pub first_seen: i64,
    pub proxy_id: Option<String>,
    pub owner_id: String,
}

impl WorkspaceStore {
    /// Atomic global claim. The primary key spans all exits and owners.
    /// Releasing a lease or deleting a proxy never removes this history.
    pub fn record_proxy_ip(
        &mut self,
        ip: Ipv4Addr,
        proxy_id: &str,
        owner: &str,
    ) -> Result<bool, StoreError> {
        if owner.is_empty() || owner.len() > 256 {
            return Err(StoreError::Invalid("代理持有者标识无效".into()));
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(i64::MAX as u128) as i64;
        Ok(self.db.execute("INSERT INTO proxy_ip_history(ip,first_seen,proxy_id,owner_id) VALUES(?1,?2,?3,?4) ON CONFLICT(ip) DO NOTHING",params![ip.to_string(),now,proxy_id,owner])?==1)
    }

    pub fn proxy_ip_history(&self) -> Result<Vec<IpHistoryEntry>, StoreError> {
        let mut statement=self.db.prepare("SELECT ip,first_seen,proxy_id,owner_id FROM proxy_ip_history ORDER BY first_seen DESC,ip")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
            ))
        })?;
        rows.map(|row| {
            let (ip, first_seen, proxy_id, owner_id) = row?;
            Ok(IpHistoryEntry {
                ip: ip
                    .parse()
                    .map_err(|_| StoreError::Invalid("IP 历史数据无效".into()))?,
                first_seen,
                proxy_id,
                owner_id,
            })
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AddressBook, StorePaths,
        workspace::{ProxyIpConfig, ProxyMode, SocksProxy},
    };

    #[test]
    fn sqlite_upgrade_preserves_encrypted_proxy_settings_and_adds_global_history() {
        let dir = tempfile::tempdir().unwrap();
        let paths = StorePaths::in_dir(dir.path().to_owned());
        AddressBook::load(paths.clone()).unwrap().save().unwrap();
        let proxy = SocksProxy {
            protocol: Default::default(),
            front_proxy_id: None,
            id: uuid::Uuid::new_v4().to_string(),
            name: "proxy".into(),
            host: "127.0.0.1".into(),
            port: 1080,
            username: "user".into(),
            password: "password-secret".into(),
            mode: ProxyMode::Shared,
            ip: ProxyIpConfig {
                rotation: Default::default(),
                change_url: Some("https://example.test/change/url-secret".into()),
                probe_url: "https://example.test/probe".into(),
            },
        };
        let mut store = WorkspaceStore::open(&paths).unwrap();
        store.save_proxy(&proxy).unwrap();
        let encrypted: Vec<u8> = store
            .db
            .query_row("SELECT encrypted FROM socks_proxies", [], |row| row.get(0))
            .unwrap();
        let bytes = String::from_utf8_lossy(&encrypted);
        assert!(!bytes.contains("url-secret"));
        assert!(!bytes.contains("password-secret"));
        assert!(!format!("{:?}", proxy.ip).contains("url-secret"));
        store
            .db
            .execute_batch("DROP TABLE proxy_ip_history; PRAGMA user_version=5;")
            .unwrap();
        drop(store);
        let mut store = WorkspaceStore::open(&paths).unwrap();
        let loaded = store.proxies().unwrap().pop().unwrap();
        assert_eq!(loaded.ip.change_url, proxy.ip.change_url);
        assert!(
            store
                .record_proxy_ip(Ipv4Addr::new(8, 8, 8, 8), &proxy.id, "task-1")
                .unwrap()
        );
        assert!(
            !store
                .record_proxy_ip(Ipv4Addr::new(8, 8, 8, 8), &proxy.id, "task-2")
                .unwrap()
        );
        assert_eq!(store.proxy_ip_history().unwrap().len(), 1);
    }

    #[test]
    fn default_ip_settings_keep_existing_sqlite_proxy_records_readable_and_validate_urls() {
        let proxy: SocksProxy=serde_json::from_value(serde_json::json!({"id":"id","name":"name","host":"host","port":1080,"username":"","password":"","mode":"Shared"})).unwrap();
        assert!(proxy.ip.change_url.is_none());
        assert!(proxy.ip.validate().is_ok());
        for url in [
            "file:///secret",
            "https://user:password@example.test/path",
            "https://example.test/#secret",
            "https://example.test/\nsecret",
        ] {
            let ip = ProxyIpConfig {
                rotation: Default::default(),
                change_url: Some(url.into()),
                ..Default::default()
            };
            assert!(ip.validate().is_err());
        }
    }
}
