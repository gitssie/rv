//! Persistent routes belong to the real IDFV; request occupancy is managed by rv-business.
use super::{AppId, WorkspaceStore};
use crate::StoreError;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProxyMode {
    Shared,
    Exclusive,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct SocksProxy {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub mode: ProxyMode,
}
impl std::fmt::Debug for SocksProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SocksProxy")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}
impl WorkspaceStore {
    pub fn save_with_proxy(
        &mut self,
        app: &super::AppInstance,
        proxy: Option<&str>,
    ) -> Result<super::AppInstance, StoreError> {
        self.db.execute_batch("SAVEPOINT edit_app_proxy")?;
        let result = (|| {
            let saved = self.save(app)?;
            self.bind_proxy(&app.id, proxy)?;
            Ok(saved)
        })();
        match result {
            Ok(saved) => {
                self.db.execute_batch("RELEASE edit_app_proxy")?;
                Ok(saved)
            }
            Err(error) => {
                self.db
                    .execute_batch("ROLLBACK TO edit_app_proxy; RELEASE edit_app_proxy")?;
                Err(error)
            }
        }
    }
    pub fn save_proxy(&mut self, proxy: &SocksProxy) -> Result<(), StoreError> {
        uuid::Uuid::parse_str(&proxy.id).map_err(|_| StoreError::Invalid("代理 ID 无效".into()))?;
        if proxy.name.trim().is_empty()
            || proxy.name.len() > 256
            || proxy.host.is_empty()
            || proxy.host.len() > 253
            || proxy.port == 0
            || proxy
                .host
                .chars()
                .any(|c| c.is_whitespace() || "/@?#\\".contains(c))
            || proxy.username.len() > 255
            || proxy.password.len() > 255
        {
            return Err(StoreError::Invalid("代理配置无效".into()));
        }
        if proxy.host.contains(':')
            && proxy
                .host
                .trim_matches(['[', ']'])
                .parse::<std::net::Ipv6Addr>()
                .is_err()
        {
            return Err(StoreError::Invalid("代理 IPv6 地址无效".into()));
        }
        let encrypted = crate::store::credentials::seal(
            &format!("rv-socks-proxy|{}", proxy.id),
            &serde_json::to_vec(proxy)?,
        )?;
        self.db.execute("INSERT INTO socks_proxies(id,name,encrypted) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET name=excluded.name,encrypted=excluded.encrypted", params![proxy.id,proxy.name,encrypted])?;
        Ok(())
    }
    pub fn proxies(&self) -> Result<Vec<SocksProxy>, StoreError> {
        let mut query = self
            .db
            .prepare("SELECT id,encrypted FROM socks_proxies ORDER BY name,id")?;
        let rows = query.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?;
        rows.map(|row| {
            let (id, data) = row?;
            let plain = crate::store::credentials::unseal(&format!("rv-socks-proxy|{id}"), data)?;
            Ok(serde_json::from_slice(&plain)?)
        })
        .collect()
    }
    pub fn proxy_binding(&self, id: &AppId) -> Result<Option<String>, StoreError> {
        self.load(id)?;
        Ok(self
            .db
            .query_row(
                "SELECT proxy_id FROM app_proxy_bindings WHERE app_id=?1",
                [id.as_str()],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub fn proxy_for_app(&self, id: &AppId) -> Result<Option<SocksProxy>, StoreError> {
        let Some(binding) = self.proxy_binding(id)? else {
            return Ok(None);
        };
        self.proxies()?
            .into_iter()
            .find(|p| p.id == binding)
            .map(Some)
            .ok_or_else(|| StoreError::Invalid("绑定的代理不存在".into()))
    }
    pub fn bind_proxy(&mut self, id: &AppId, proxy: Option<&str>) -> Result<(), StoreError> {
        self.load(id)?;
        if let Some(proxy) = proxy {
            self.db.execute("INSERT INTO app_proxy_bindings VALUES(?1,?2) ON CONFLICT(app_id) DO UPDATE SET proxy_id=excluded.proxy_id",params![id.as_str(),proxy])?;
        } else {
            self.db.execute(
                "DELETE FROM app_proxy_bindings WHERE app_id=?1",
                [id.as_str()],
            )?;
        }
        Ok(())
    }
    /// A bound route cannot be deleted: deletion must never silently change the exit.
    pub fn delete_proxy(&mut self, id: &str) -> Result<(), StoreError> {
        self.db
            .execute("DELETE FROM socks_proxies WHERE id=?1", [id])?;
        Ok(())
    }
}
