//! Shared host request transport and reentrant, IDFV-scoped proxy leases.
use rv_core::{
    StorePaths,
    workspace::{AppId, ProxyMode, SocksProxy, WorkspaceStore},
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::sync::Notify;

#[derive(Default)]
struct Gate {
    owners: Mutex<HashMap<String, (usize, usize)>>,
    changed: Notify,
}
pub(crate) struct Lease {
    gate: Arc<Gate>,
    owner: String,
    exclusive: bool,
}
impl Gate {
    async fn acquire(self: &Arc<Self>, owner: &str, mode: ProxyMode) -> Lease {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut owners = self.owners.lock().unwrap();
                let blocked = owners.iter().any(|(id, (shared, exclusive))| {
                    id != owner && (*exclusive > 0 || (mode == ProxyMode::Exclusive && *shared > 0))
                });
                if !blocked {
                    let counts = owners.entry(owner.into()).or_default();
                    let exclusive = mode == ProxyMode::Exclusive;
                    if exclusive {
                        counts.1 += 1
                    } else {
                        counts.0 += 1
                    }
                    return Lease {
                        gate: self.clone(),
                        owner: owner.into(),
                        exclusive,
                    };
                }
            }
            notified.await;
        }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let mut owners = self.gate.owners.lock().unwrap();
        if let Some(counts) = owners.get_mut(&self.owner) {
            if self.exclusive {
                counts.1 -= 1
            } else {
                counts.0 -= 1
            }
            if *counts == (0, 0) {
                owners.remove(&self.owner);
            }
        }
        drop(owners);
        self.gate.changed.notify_waiters();
    }
}
#[derive(Clone)]
pub(crate) struct Route {
    pub client: reqwest::Client,
    gate: Option<Arc<Gate>>,
    mode: ProxyMode,
    owner: String,
}
impl Route {
    pub async fn acquire(&self) -> Option<Lease> {
        match &self.gate {
            Some(gate) => Some(gate.acquire(&self.owner, self.mode).await),
            None => None,
        }
    }
}
fn builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .no_proxy()
        .retry(reqwest::retry::never())
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(40))
}
fn proxy_client(proxy: &SocksProxy, images: bool) -> Result<reqwest::Client, String> {
    // socks5h performs DNS at the same exit as the request.
    let host = if proxy.host.contains(':') {
        format!("[{}]", proxy.host.trim_matches(['[', ']']))
    } else {
        proxy.host.clone()
    };
    let url = format!("socks5h://{host}:{}", proxy.port);
    let mut configured = reqwest::Proxy::all(url).map_err(|_| "代理地址无效")?;
    if !proxy.username.is_empty() || !proxy.password.is_empty() {
        configured = configured.basic_auth(&proxy.username, &proxy.password);
    }
    let builder = if images {
        builder()
            .redirect(reqwest::redirect::Policy::limited(3))
            .timeout(Duration::from_secs(15))
    } else {
        builder()
    };
    builder
        .proxy(configured)
        .build()
        .map_err(|_| "无法初始化代理客户端".into())
}
pub(crate) fn route(
    paths: &StorePaths,
    id: &AppId,
    direct: &reqwest::Client,
) -> Result<Route, String> {
    route_for(paths, id, direct, false)
}
pub(crate) fn image_route(
    paths: &StorePaths,
    id: &AppId,
    direct: &reqwest::Client,
) -> Result<Route, String> {
    route_for(paths, id, direct, true)
}
fn route_for(
    paths: &StorePaths,
    id: &AppId,
    direct: &reqwest::Client,
    images: bool,
) -> Result<Route, String> {
    let proxy = WorkspaceStore::open(paths)
        .map_err(|e| e.to_string())?
        .proxy_for_app(id)
        .map_err(|e| e.to_string())?;
    let Some(proxy) = proxy else {
        return Ok(Route {
            client: direct.clone(),
            gate: None,
            mode: ProxyMode::Shared,
            owner: id.as_str().into(),
        });
    };
    static GATES: OnceLock<Mutex<HashMap<(std::path::PathBuf, String), Arc<Gate>>>> =
        OnceLock::new();
    let key = (
        std::fs::canonicalize(&paths.database).map_err(|_| "代理数据库不可用")?,
        proxy.id.clone(),
    );
    let gate = GATES
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .entry(key)
        .or_default()
        .clone();
    // Cached clients keep HTTP connections pooled across task/chat runner instances.
    static CLIENTS: OnceLock<Mutex<HashMap<Vec<u8>, reqwest::Client>>> = OnceLock::new();
    let fingerprint =
        Sha256::digest(serde_json::to_vec(&(images, &proxy)).map_err(|_| "代理配置无效")?).to_vec();
    let mut clients = CLIENTS.get_or_init(Default::default).lock().unwrap();
    let client = match clients.get(&fingerprint) {
        Some(client) => client.clone(),
        None => {
            let client = proxy_client(&proxy, images)?;
            clients.insert(fingerprint, client.clone());
            client
        }
    };
    Ok(Route {
        client,
        gate: Some(gate),
        mode: proxy.mode,
        owner: id.as_str().into(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn exclusive_waits_for_other_id_and_same_id_is_reentrant() {
        let gate = Arc::new(Gate::default());
        let first = gate.acquire("a", ProxyMode::Exclusive).await;
        let own = gate.acquire("a", ProxyMode::Exclusive).await;
        assert!(
            tokio::time::timeout(
                Duration::from_millis(20),
                gate.acquire("b", ProxyMode::Shared)
            )
            .await
            .is_err()
        );
        drop(first);
        assert!(
            tokio::time::timeout(
                Duration::from_millis(20),
                gate.acquire("b", ProxyMode::Exclusive)
            )
            .await
            .is_err()
        );
        drop(own);
        let _other = tokio::time::timeout(
            Duration::from_secs(1),
            gate.acquire("b", ProxyMode::Exclusive),
        )
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn shared_concurrent_and_cancelled_waiter_does_not_occupy() {
        let gate = Arc::new(Gate::default());
        let a = gate.acquire("a", ProxyMode::Shared).await;
        let b = gate.acquire("b", ProxyMode::Shared).await;
        assert!(
            tokio::time::timeout(
                Duration::from_millis(20),
                gate.acquire("c", ProxyMode::Exclusive)
            )
            .await
            .is_err()
        );
        drop(a);
        drop(b);
        let c = gate.acquire("c", ProxyMode::Exclusive).await;
        drop(c);
        assert!(gate.owners.lock().unwrap().is_empty());
    }
    fn fixture() -> (tempfile::TempDir, StorePaths, AppId) {
        let dir = tempfile::tempdir().unwrap();
        let paths = StorePaths::in_dir(dir.path().into());
        let mut book = rv_core::AddressBook::load(paths.clone()).unwrap();
        let phone = rv_core::Connection::new("phone", "127.0.0.1", 5900);
        book.upsert(phone.clone());
        book.save().unwrap();
        let id = AppId::from_idfv("C9EB7804-2062-4519-AC8D-6F68972574F3").unwrap();
        WorkspaceStore::open(&paths).unwrap().sync_backups(phone.id,"lt.manodrabuziai.fr",&serde_json::json!({
            "bundle_id":"lt.manodrabuziai.fr","backups":[{"kind":"snapshot","backup_id":"snapshot:1","idfv":id.as_str()}]
        })).unwrap();
        (dir, paths, id)
    }
    fn bind(paths: &StorePaths, id: &AppId, port: u16) {
        let mut store = WorkspaceStore::open(paths).unwrap();
        let proxy = SocksProxy {
            id: uuid::Uuid::new_v4().to_string(),
            name: "test".into(),
            host: "127.0.0.1".into(),
            port,
            username: "user".into(),
            password: "pass".into(),
            mode: ProxyMode::Exclusive,
        };
        store.save_proxy(&proxy).unwrap();
        store.bind_proxy(id, Some(&proxy.id)).unwrap();
    }
    #[tokio::test]
    async fn saved_proxy_authentication_and_remote_dns_are_used() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (_dir, paths, id) = fixture();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        bind(&paths, &id, listener.local_addr().unwrap().port());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut hello = [0u8; 2];
            socket.read_exact(&mut hello).await.unwrap();
            assert_eq!(hello[0], 5);
            let mut methods = vec![0; hello[1] as usize];
            socket.read_exact(&mut methods).await.unwrap();
            assert!(methods.contains(&2));
            socket.write_all(&[5, 2]).await.unwrap();
            let mut auth = [0u8; 2];
            socket.read_exact(&mut auth).await.unwrap();
            assert_eq!(auth[0], 1);
            let mut user = vec![0; auth[1] as usize];
            socket.read_exact(&mut user).await.unwrap();
            assert_eq!(user, b"user");
            let length = socket.read_u8().await.unwrap();
            let mut pass = vec![0; length as usize];
            socket.read_exact(&mut pass).await.unwrap();
            assert_eq!(pass, b"pass");
            socket.write_all(&[1, 0]).await.unwrap();
            let mut connect = [0u8; 4];
            socket.read_exact(&mut connect).await.unwrap();
            assert_eq!(connect, [5, 1, 0, 3]);
            let length = socket.read_u8().await.unwrap();
            let mut host = vec![0; length as usize];
            socket.read_exact(&mut host).await.unwrap();
            assert_eq!(host, b"rv-fixture.invalid");
            assert_eq!(socket.read_u16().await.unwrap(), 80);
            socket
                .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 80])
                .await
                .unwrap();
            let mut buffer = [0; 4096];
            let size = socket.read(&mut buffer).await.unwrap();
            assert!(String::from_utf8_lossy(&buffer[..size]).starts_with("GET /fixture "));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                .await
                .unwrap();
        });
        let route = route(&paths, &id, &builder().build().unwrap()).unwrap();
        let lease = route.acquire().await;
        let response = route
            .client
            .get("http://rv-fixture.invalid/fixture")
            .send()
            .await
            .unwrap();
        assert_eq!(response.text().await.unwrap(), "{}");
        drop(lease);
        server.await.unwrap();
        assert!(
            route
                .gate
                .as_ref()
                .unwrap()
                .owners
                .lock()
                .unwrap()
                .is_empty()
        );
    }
    #[tokio::test]
    async fn failed_fixed_proxy_does_not_fall_back_to_direct() {
        let (_dir, paths, id) = fixture();
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        bind(&paths, &id, port);
        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let route = route(&paths, &id, &builder().build().unwrap()).unwrap();
        let lease = route.acquire().await;
        assert!(
            route
                .client
                .get(format!("http://{}/", origin.local_addr().unwrap()))
                .send()
                .await
                .is_err()
        );
        drop(lease);
        assert!(
            tokio::time::timeout(Duration::from_millis(30), origin.accept())
                .await
                .is_err()
        );
        assert!(
            route
                .gate
                .as_ref()
                .unwrap()
                .owners
                .lock()
                .unwrap()
                .is_empty()
        );
    }
    #[tokio::test]
    async fn image_proxy_follows_limited_redirects_and_shares_business_gate() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (_dir, paths, id) = fixture();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = SocksProxy {
            id: uuid::Uuid::new_v4().to_string(),
            name: "images".into(),
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
            username: String::new(),
            password: String::new(),
            mode: ProxyMode::Exclusive,
        };
        let mut store = WorkspaceStore::open(&paths).unwrap();
        store.save_proxy(&proxy).unwrap();
        store.bind_proxy(&id, Some(&proxy.id)).unwrap();
        let server = tokio::spawn(async move {
            for expected in ["GET /redirect ", "GET /image "] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut hello = [0; 2];
                socket.read_exact(&mut hello).await.unwrap();
                let mut methods = vec![0; hello[1] as usize];
                socket.read_exact(&mut methods).await.unwrap();
                socket.write_all(&[5, 0]).await.unwrap();
                let mut connect = [0; 4];
                socket.read_exact(&mut connect).await.unwrap();
                assert_eq!(connect, [5, 1, 0, 3]);
                let length = socket.read_u8().await.unwrap();
                let mut host = vec![0; length as usize];
                socket.read_exact(&mut host).await.unwrap();
                let _port = socket.read_u16().await.unwrap();
                socket
                    .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 80])
                    .await
                    .unwrap();
                let mut bytes = [0; 4096];
                let size = socket.read(&mut bytes).await.unwrap();
                assert!(String::from_utf8_lossy(&bytes[..size]).starts_with(expected));
                if expected.contains("redirect") {
                    socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: /image\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                } else {
                    socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                        )
                        .await
                        .unwrap();
                }
            }
        });
        let direct = builder().build().unwrap();
        let images = image_route(&paths, &id, &direct).unwrap();
        let business = route(&paths, &id, &direct).unwrap();
        assert!(Arc::ptr_eq(
            images.gate.as_ref().unwrap(),
            business.gate.as_ref().unwrap()
        ));
        let lease = images.acquire().await;
        assert_eq!(
            images
                .client
                .get("http://rv-fixture.invalid/redirect")
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "{}"
        );
        drop(lease);
        server.await.unwrap();
    }
}
