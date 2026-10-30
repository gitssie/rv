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
/// How the provider changes its exit; independent of the transport protocol.
#[derive(Clone, Default, Serialize, Deserialize)]
pub enum ProxyRotation {
    #[default]
    Http,
    PasswordSession {
        marker: String,
    },
}
impl ProxyRotation {
    /// Replace only the alphanumeric session following the configured marker.
    pub fn next_password(&self, password: &str) -> Result<String, StoreError> {
        let Self::PasswordSession { marker } = self else {
            return Err(StoreError::Invalid("未配置密码 Session 换 IP".into()));
        };
        if marker.is_empty() || marker.len() > 64 || marker.chars().any(char::is_control) {
            return Err(StoreError::Invalid("密码 Session 标记无效".into()));
        }
        let start = password
            .rfind(marker)
            .ok_or_else(|| StoreError::Invalid("代理密码中没有配置的 Session 标记".into()))?
            + marker.len();
        let length = password[start..]
            .bytes()
            .take_while(u8::is_ascii_alphanumeric)
            .count();
        if length == 0 {
            return Err(StoreError::Invalid("代理密码中的 Session 为空".into()));
        }
        let mut random = String::new();
        while random.len() < length {
            random.push_str(&uuid::Uuid::new_v4().simple().to_string());
        }
        random.truncate(length);
        // Guarantee a different value even for short provider sessions.
        if random == password[start..start + length] {
            random.replace_range(..1, if random.starts_with('a') { "b" } else { "a" });
        }
        Ok(format!(
            "{}{}{}",
            &password[..start],
            random,
            &password[start + length..]
        ))
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ProxyIpConfig {
    #[serde(default)]
    pub rotation: ProxyRotation,
    pub change_url: Option<String>,
    pub probe_url: String,
}
impl Default for ProxyIpConfig {
    fn default() -> Self {
        Self {
            rotation: ProxyRotation::Http,
            change_url: None,
            probe_url: "https://ipwho.is/?fields=success,ip,type,country,region,city".into(),
        }
    }
}
impl ProxyIpConfig {
    pub fn validate(&self) -> Result<(), StoreError> {
        for raw in self
            .change_url
            .iter()
            .filter(|_| matches!(self.rotation, ProxyRotation::Http))
            .chain(std::iter::once(&self.probe_url))
        {
            let url =
                url::Url::parse(raw).map_err(|_| StoreError::Invalid("IP 接口地址无效".into()))?;
            if raw.len() > 4096
                || raw.trim() != raw
                || raw.chars().any(char::is_control)
                || !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
            {
                return Err(StoreError::Invalid(
                    "IP 接口须为不带用户信息或片段的 HTTP(S) 地址".into(),
                ));
            }
        }
        Ok(())
    }
}
impl std::fmt::Debug for ProxyIpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProxyIpConfig { [redacted] }")
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProxyProtocol {
    #[default]
    Socks5,
    Http,
}
impl ProxyProtocol {
    pub fn label(self) -> &'static str {
        match self {
            Self::Socks5 => "SOCKS5",
            Self::Http => "HTTP",
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct SocksProxy {
    #[serde(default)]
    pub protocol: ProxyProtocol,
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub mode: ProxyMode,
    #[serde(default)]
    pub front_proxy_id: Option<String>,
    #[serde(default)]
    pub ip: ProxyIpConfig,
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
        self.save_with_proxy_and_login(app, proxy, None)
    }
    /// Save the editor's app settings and known login credentials atomically.
    pub fn save_with_proxy_and_login(
        &mut self,
        app: &super::AppInstance,
        proxy: Option<&str>,
        login: Option<&super::AppLoginCredentials>,
    ) -> Result<super::AppInstance, StoreError> {
        self.db.execute_batch("SAVEPOINT edit_app_proxy")?;
        let checkpoint = self.changes.checkpoint();
        let result = (|| {
            let saved = self.save(app)?;
            self.bind_proxy(&app.id, proxy)?;
            if let Some(login) = login {
                let mut login = login.clone();
                // The editor displays a read-only username. A background profile
                // sync may have renamed it since the editor was opened.
                if let Some(current) = self.login_credentials(&app.id)? {
                    login.username = current.username;
                }
                super::login_credentials::write_login_credentials(&self.db, &app.id, &login)?;
            }
            Ok(saved)
        })();
        match result {
            Ok(saved) => {
                self.db.execute_batch("RELEASE edit_app_proxy")?;
                self.publish_changes();
                Ok(saved)
            }
            Err(error) => {
                self.db
                    .execute_batch("ROLLBACK TO edit_app_proxy; RELEASE edit_app_proxy")?;
                self.changes.restore(checkpoint);
                Err(error)
            }
        }
    }
    pub fn save_proxy(&mut self, proxy: &SocksProxy) -> Result<(), StoreError> {
        proxy.ip.validate()?;
        if matches!(proxy.ip.rotation, ProxyRotation::PasswordSession { .. }) {
            proxy.ip.rotation.next_password(&proxy.password)?;
        }
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
        self.business_transaction(|db| {
            let entries = read_proxies(db)?;
            validate_front(proxy, &entries)?;
            let encrypted = crate::store::credentials::seal(
                &format!("rv-socks-proxy|{}", proxy.id), &serde_json::to_vec(proxy)?,
            )?;
            db.execute("INSERT INTO socks_proxies(id,name,encrypted) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET name=excluded.name,encrypted=excluded.encrypted", params![proxy.id,proxy.name,encrypted])?;
            Ok(())
        })
    }
    /// Read and replace the password in one transaction, preserving other settings.
    pub fn rotate_proxy_password_session(
        &mut self,
        expected: &SocksProxy,
        expected_front: &Option<SocksProxy>,
    ) -> Result<SocksProxy, StoreError> {
        self.business_transaction(|db| {
            let entries = read_proxies(db)?;
            let front = entries
                .iter()
                .find(|p| Some(&p.id) == expected.front_proxy_id.as_ref());
            if serde_json::to_vec(&front)? != serde_json::to_vec(expected_front)? {
                return Err(StoreError::Invalid(
                    "前置代理配置已修改，请重新换 IP".into(),
                ));
            }
            let mut proxy = entries
                .into_iter()
                .find(|p| p.id == expected.id)
                .ok_or_else(|| StoreError::Invalid("代理不存在".into()))?;
            if serde_json::to_vec(&proxy)? != serde_json::to_vec(expected)? {
                return Err(StoreError::Invalid("代理配置已修改，请重新换 IP".into()));
            }
            proxy.password = proxy.ip.rotation.next_password(&proxy.password)?;
            let encrypted = crate::store::credentials::seal(
                &format!("rv-socks-proxy|{}", proxy.id),
                &serde_json::to_vec(&proxy)?,
            )?;
            db.execute(
                "UPDATE socks_proxies SET encrypted=?2 WHERE id=?1",
                params![expected.id, encrypted],
            )?;
            Ok(proxy)
        })
    }
    pub fn proxies(&self) -> Result<Vec<SocksProxy>, StoreError> {
        read_proxies(&self.db)
    }
    /// Resolve the optional existing front record once, including unsaved exit forms.
    pub fn resolve_proxy(&self, proxy: &SocksProxy) -> Result<ProxyRoute, StoreError> {
        let entries = self.proxies()?;
        validate_front(proxy, &entries)?;
        let front = proxy
            .front_proxy_id
            .as_ref()
            .and_then(|id| entries.into_iter().find(|p| &p.id == id));
        Ok(ProxyRoute {
            exit: proxy.clone(),
            front,
        })
    }
    pub fn proxy_engine_path(&self) -> Result<Option<String>, StoreError> {
        Ok(self
            .db
            .query_row(
                "SELECT executable FROM proxy_engine_settings WHERE id=1",
                [],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub fn save_proxy_engine_path(&mut self, path: Option<&str>) -> Result<(), StoreError> {
        if path.is_some_and(|p| p.is_empty() || p.len() > 4096 || p.chars().any(char::is_control)) {
            return Err(StoreError::Invalid("sing-box 路径无效".into()));
        }
        self.business_transaction(|db| {
            if let Some(path) = path {
                db.execute("INSERT INTO proxy_engine_settings VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET executable=excluded.executable", [path])?;
            } else { db.execute("DELETE FROM proxy_engine_settings WHERE id=1", [])?; }
            Ok(())
        })
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
        self.publish_changes();
        Ok(())
    }
    /// A bound route cannot be deleted: deletion must never silently change the exit.
    pub fn delete_proxy(&mut self, id: &str) -> Result<(), StoreError> {
        self.business_transaction(|db| {
            if read_proxies(db)?
                .iter()
                .any(|p| p.front_proxy_id.as_deref() == Some(id))
            {
                return Err(StoreError::Invalid(
                    "此代理仍被其他代理用作前置，请先解除引用".into(),
                ));
            }
            db.execute("DELETE FROM socks_proxies WHERE id=?1", [id])?;
            Ok(())
        })
    }
}

#[derive(Clone, Serialize)]
pub struct ProxyRoute {
    pub exit: SocksProxy,
    pub front: Option<SocksProxy>,
}
fn read_proxies(db: &rusqlite::Connection) -> Result<Vec<SocksProxy>, StoreError> {
    let mut query = db.prepare("SELECT id,encrypted FROM socks_proxies ORDER BY name,id")?;
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
fn validate_front(proxy: &SocksProxy, entries: &[SocksProxy]) -> Result<(), StoreError> {
    let Some(id) = &proxy.front_proxy_id else {
        return Ok(());
    };
    if id == &proxy.id {
        return Err(StoreError::Invalid("不能选择自身作为前置代理".into()));
    }
    let front = entries
        .iter()
        .find(|p| &p.id == id)
        .ok_or_else(|| StoreError::Invalid("前置代理不存在".into()))?;
    if front.front_proxy_id.is_some()
        || entries
            .iter()
            .any(|p| p.front_proxy_id.as_ref() == Some(&proxy.id))
    {
        return Err(StoreError::Invalid("仅支持一层前置代理".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn proxy() -> SocksProxy {
        SocksProxy {
            protocol: Default::default(),
            id: uuid::Uuid::new_v4().to_string(),
            name: "fixture".into(),
            host: "127.0.0.1".into(),
            port: 1080,
            username: String::new(),
            password: String::new(),
            mode: ProxyMode::Shared,
            front_proxy_id: None,
            ip: Default::default(),
        }
    }
    #[test]
    fn front_is_a_reference_and_only_one_level_is_allowed_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::StorePaths::in_dir(dir.path().into());
        let mut store = WorkspaceStore::open(&paths).unwrap();
        let mut a = proxy();
        let mut b = proxy();
        let mut c = proxy();
        for p in [&a, &b, &c] {
            store.save_proxy(p).unwrap();
        }
        b.front_proxy_id = Some(b.id.clone());
        assert!(store.save_proxy(&b).is_err());
        b.front_proxy_id = Some(uuid::Uuid::new_v4().to_string());
        assert!(store.save_proxy(&b).is_err());
        b.front_proxy_id = Some(a.id.clone());
        store.save_proxy(&b).unwrap();
        assert_eq!(store.proxies().unwrap().len(), 3);
        assert_eq!(store.resolve_proxy(&b).unwrap().front.unwrap().id, a.id);
        c.front_proxy_id = Some(b.id.clone());
        assert!(store.save_proxy(&c).is_err());
        a.front_proxy_id = Some(c.id.clone());
        assert!(store.save_proxy(&a).is_err());
        assert!(store.delete_proxy(&a.id).is_err());
        assert!(
            store
                .proxies()
                .unwrap()
                .iter()
                .find(|p| p.id == a.id)
                .unwrap()
                .front_proxy_id
                .is_none()
        );
        b.front_proxy_id = None;
        store.save_proxy(&b).unwrap();
        store.delete_proxy(&a.id).unwrap();
        assert_eq!(store.proxies().unwrap().len(), 2);
    }
    #[test]
    fn existing_proxy_json_without_front_remains_a_single_proxy() {
        let mut value = serde_json::to_value(proxy()).unwrap();
        value.as_object_mut().unwrap().remove("front_proxy_id");
        value.as_object_mut().unwrap().remove("protocol");
        let proxy = serde_json::from_value::<SocksProxy>(value).unwrap();
        assert!(proxy.front_proxy_id.is_none());
        assert_eq!(proxy.protocol, ProxyProtocol::Socks5);
    }
}

#[cfg(test)]
mod rotation_tests {
    use super::*;
    #[test]
    fn password_rotation_preserves_other_provider_parameters() {
        let rule = ProxyRotation::PasswordSession {
            marker: "_session-".into(),
        };
        let old = "secret_country-fr_lifetime-5_session-a7ozznml_more-value";
        let new = rule.next_password(old).unwrap();
        assert_ne!(new, old);
        assert!(new.starts_with("secret_country-fr_lifetime-5_session-"));
        assert!(new.ends_with("_more-value"));
        assert_eq!(new.len(), old.len());
        assert!(rule.next_password("secret_without_session").is_err());
        assert!(rule.next_password("secret_session-").is_err());
        let old: ProxyIpConfig = serde_json::from_str(
            r#"{"change_url":"https://example.test/change","probe_url":"https://example.test/ip"}"#,
        )
        .unwrap();
        assert!(matches!(old.rotation, ProxyRotation::Http));
    }
}
