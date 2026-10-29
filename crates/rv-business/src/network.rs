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
pub(crate) struct Gate {
    owners: Mutex<HashMap<OwnerKey, Holder>>,
    changed: Notify,
    last_business_owner: tokio::sync::Mutex<Option<String>>,
}
#[derive(Clone, PartialEq, Eq, Hash)]
enum OwnerKey {
    Business(String),
    Managed(String),
}
#[derive(Default)]
struct Holder {
    shared: usize,
    exclusive: usize,
    preparation: Option<Arc<crate::proxy::PreparationSlot>>,
}
pub(crate) struct Lease {
    gate: Arc<Gate>,
    owner: OwnerKey,
    exclusive: bool,
    pub(crate) preparation: Option<Arc<crate::proxy::PreparationSlot>>,
}
impl Gate {
    pub(crate) async fn acquire(self: &Arc<Self>, owner: &str, mode: ProxyMode) -> Lease {
        self.acquire_inner(OwnerKey::Business(owner.into()), mode, None)
            .await
            .expect("plain leases cannot conflict with their own policy")
    }
    pub(crate) async fn acquire_managed(
        self: &Arc<Self>,
        owner: &str,
        slot: crate::proxy::PreparationSlot,
    ) -> Result<Lease, crate::proxy::ProxyError> {
        self.acquire_inner(
            OwnerKey::Managed(owner.into()),
            ProxyMode::Exclusive,
            Some(slot),
        )
        .await
    }
    async fn acquire_inner(
        self: &Arc<Self>,
        owner: OwnerKey,
        mode: ProxyMode,
        mut slot: Option<crate::proxy::PreparationSlot>,
    ) -> Result<Lease, crate::proxy::ProxyError> {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut owners = self.owners.lock().unwrap();
                if let Some(candidate) = slot.as_ref() {
                    // An IDFV accidentally supplied as a task owner must not wait
                    // behind its own ordinary business lease.
                    if owners.contains_key(&OwnerKey::Business(candidate.owner_id().into())) {
                        return Err(crate::proxy::ProxyError::ReentryConflict);
                    }
                    if let Some(own) = owners.get(&owner) {
                        // Never wait for a shared-to-exclusive upgrade while retaining
                        // a shared lease: two such upgrades can deadlock each other.
                        if own.exclusive == 0
                            || (own.preparation.is_none()
                                && slot.as_ref().unwrap().requires_rotation())
                        {
                            return Err(crate::proxy::ProxyError::ReentryConflict);
                        }
                    }
                }
                let blocked = owners.iter().any(|(id, holder)| {
                    id != &owner
                        && (holder.exclusive > 0
                            || (mode == ProxyMode::Exclusive && holder.shared > 0))
                });
                if !blocked {
                    let holder = owners.entry(owner.clone()).or_default();
                    let reentrant = holder.shared + holder.exclusive > 0;
                    let managed = slot.is_some();
                    if managed && holder.preparation.is_none() {
                        let mut preparation = slot.take().unwrap();
                        preparation.allow_rotation = !reentrant;
                        holder.preparation = Some(Arc::new(preparation));
                    }
                    let preparation = managed.then(|| holder.preparation.clone()).flatten();
                    let exclusive = mode == ProxyMode::Exclusive;
                    if exclusive {
                        holder.exclusive += 1;
                    } else {
                        holder.shared += 1;
                    }
                    return Ok(Lease {
                        gate: self.clone(),
                        owner: owner.clone(),
                        exclusive,
                        preparation,
                    });
                }
            }
            notified.await;
        }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let mut owners = self.gate.owners.lock().unwrap();
        if let Some(holder) = owners.get_mut(&self.owner) {
            if self.exclusive {
                holder.exclusive -= 1;
            } else {
                holder.shared -= 1;
            }
            if holder.shared == 0 && holder.exclusive == 0 {
                owners.remove(&self.owner);
            }
        }
        drop(owners);
        self.gate.changed.notify_waiters();
    }
}

pub(crate) fn proxy_gate(paths: &StorePaths, proxy_id: &str) -> Result<Arc<Gate>, String> {
    static GATES: OnceLock<Mutex<HashMap<(std::path::PathBuf, String), Arc<Gate>>>> =
        OnceLock::new();
    let key = (
        std::fs::canonicalize(&paths.database).map_err(|_| "代理数据库不可用")?,
        proxy_id.to_owned(),
    );
    Ok(GATES
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .entry(key)
        .or_default()
        .clone())
}

/// Keeps the external route alive for as long as its HTTP client is in use.
#[derive(Clone)]
pub(crate) struct ProxyClient {
    inner: reqwest::Client,
    tunnel: Option<Arc<crate::proxy::engine::Tunnel>>,
}
impl std::ops::Deref for ProxyClient {
    type Target = reqwest::Client;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
#[derive(Clone)]
pub(crate) struct Route {
    pub client: reqwest::Client,
    _tunnel: Option<Arc<crate::proxy::engine::Tunnel>>,
    gate: Option<Arc<Gate>>,
    mode: ProxyMode,
    owner: String,
    proxy_fingerprint: Option<Vec<u8>>,
}
impl Route {
    pub fn for_images(&self, paths: &StorePaths, id: &AppId) -> Result<Self, String> {
        let images = image_route(paths, id, &self.client)?;
        if images.proxy_fingerprint != self.proxy_fingerprint {
            return Err("虚拟 App 的代理关联或配置已变更，请重新执行".into());
        }
        Ok(images)
    }
    pub fn validate_binding(&self, paths: &StorePaths, id: &AppId) -> Result<(), String> {
        let current = WorkspaceStore::open(paths)
            .map_err(|e| e.to_string())?
            .proxy_for_app(id)
            .map_err(|e| e.to_string())?;
        let current = current
            .as_ref()
            .map(|p| proxy_fingerprint(paths, p))
            .transpose()?;
        if current != self.proxy_fingerprint {
            return Err("虚拟 App 的代理关联或配置已变更，请重新执行".into());
        }
        Ok(())
    }
    pub async fn acquire(&self) -> Option<Lease> {
        match &self.gate {
            Some(gate) => Some(gate.acquire(&self.owner, self.mode).await),
            None => None,
        }
    }
}
/// Hold the existing proxy occupancy lease through rotation and the entire request.
/// Ownership survives lease release, so consecutive operations for one virtual App
/// keep their exit even when they use different tasks or HTTP clients.
pub(crate) async fn acquire_route(
    paths: &StorePaths,
    id: &AppId,
    direct: &reqwest::Client,
    required: bool,
) -> Result<(Route, Option<Lease>), String> {
    acquire_route_for(paths, id, direct, required, false).await
}
pub(crate) async fn acquire_image_route(
    paths: &StorePaths,
    id: &AppId,
    direct: &reqwest::Client,
) -> Result<(Route, Option<Lease>), String> {
    acquire_route_for(paths, id, direct, false, true).await
}
async fn acquire_route_for(
    paths: &StorePaths,
    id: &AppId,
    direct: &reqwest::Client,
    required: bool,
    images: bool,
) -> Result<(Route, Option<Lease>), String> {
    let initial = route_for(paths, id, direct, images, required)?;
    let lease = initial.acquire().await;
    let Some(gate) = &initial.gate else {
        return Ok((initial, lease));
    };
    // Serialize same-App reentries too: none may send while its switch is pending.
    let mut last = gate.last_business_owner.lock().await;
    let mut route = route_for(paths, id, direct, images, required)?;
    if route.mode != initial.mode
        || !route
            .gate
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, gate))
    {
        return Err("虚拟 App 的代理关联已变更，请重新执行".into());
    }
    if route.mode == ProxyMode::Exclusive {
        if last.as_deref().is_some_and(|owner| owner != id.as_str()) {
            let proxy = WorkspaceStore::open(paths)
                .map_err(|e| e.to_string())?
                .proxy_for_app(id)
                .map_err(|e| e.to_string())?
                .ok_or("绑定的代理不存在")?;
            if !Arc::ptr_eq(&proxy_gate(paths, &proxy.id)?, gate) {
                return Err("虚拟 App 的代理关联已变更，请重新执行".into());
            }
            let (prepared, snapshot) =
                crate::proxy::rotate_for_business(paths, &proxy.id, id.as_str()).await?;
            // HTTP rotation leaves the credential fingerprint unchanged, but old
            // pooled sockets must not carry the new App through the previous exit.
            let database =
                std::fs::canonicalize(&paths.database).map_err(|_| "代理数据库不可用")?;
            CLIENTS
                .get_or_init(Default::default)
                .lock()
                .unwrap()
                .entries
                .retain(|(path, proxy_id, _), _| path != &database || proxy_id != &proxy.id);
            // Retain the exact full chain confirmed by preparation, including the
            // new password. Later config edits must not silently replace that chain.
            let client = if images {
                build_route_client(paths, &snapshot, true, true)?
            } else {
                prepared
            };
            route.client = client.inner;
            route._tunnel = client.tunnel;
            route.proxy_fingerprint = Some(snapshot_fingerprint(paths, &snapshot)?);
            route.validate_binding(paths, id)?;
        }
        *last = Some(id.as_str().into());
    } else {
        // A shared interval has no single owner; establish a new baseline if
        // the user later changes this proxy back to exclusive mode.
        *last = None;
    }
    drop(last);
    Ok((route, lease))
}
fn builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .no_proxy()
        .retry(reqwest::retry::never())
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(40))
}
fn proxy_client(
    paths: &StorePaths,
    proxy: &SocksProxy,
    images: bool,
) -> Result<ProxyClient, String> {
    build_proxy_client(paths, proxy, images, false)
}
pub(crate) fn probe_client(paths: &StorePaths, proxy: &SocksProxy) -> Result<ProxyClient, String> {
    build_proxy_client(paths, proxy, false, true)
}
fn build_proxy_client(
    paths: &StorePaths,
    proxy: &SocksProxy,
    images: bool,
    fresh: bool,
) -> Result<ProxyClient, String> {
    let route = WorkspaceStore::open(paths)
        .and_then(|s| s.resolve_proxy(proxy))
        .map_err(|e| e.to_string())?;
    build_route_client(paths, &route, images, fresh)
}
/// Probe a captured route, including the captured front, after credential rotation.
pub(crate) fn probe_snapshot_client(
    paths: &StorePaths,
    exit: &SocksProxy,
    front: &Option<SocksProxy>,
) -> Result<ProxyClient, String> {
    build_route_client(
        paths,
        &rv_core::workspace::ProxyRoute {
            exit: exit.clone(),
            front: front.clone(),
        },
        false,
        true,
    )
}
fn build_route_client(
    paths: &StorePaths,
    route: &rv_core::workspace::ProxyRoute,
    images: bool,
    fresh: bool,
) -> Result<ProxyClient, String> {
    let tunnel = route
        .front
        .as_ref()
        .map(|_| crate::proxy::engine::acquire(paths, route))
        .transpose()?;
    let mut endpoint = route.exit.clone();
    if let Some(tunnel) = &tunnel {
        endpoint.protocol = rv_core::workspace::ProxyProtocol::Socks5;
        endpoint.host = "127.0.0.1".into();
        endpoint.port = tunnel.port;
        endpoint.username = tunnel.username.clone();
        endpoint.password = tunnel.password.clone();
    }
    let proxy = &endpoint;
    // socks5h performs DNS at the same exit as the request.
    let host = if proxy.host.contains(':') {
        format!("[{}]", proxy.host.trim_matches(['[', ']']))
    } else {
        proxy.host.clone()
    };
    let scheme = match proxy.protocol {
        rv_core::workspace::ProxyProtocol::Socks5 => "socks5h",
        rv_core::workspace::ProxyProtocol::Http => "http",
    };
    let url = format!("{scheme}://{host}:{}", proxy.port);
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
    let builder = if fresh {
        builder.pool_max_idle_per_host(0)
    } else {
        builder
    };
    let inner = builder
        .proxy(configured)
        .build()
        .map_err(|_| "无法初始化代理客户端".to_owned())?;
    Ok(ProxyClient { inner, tunnel })
}
#[cfg(test)]
pub(crate) fn route(
    paths: &StorePaths,
    id: &AppId,
    direct: &reqwest::Client,
) -> Result<Route, String> {
    route_for(paths, id, direct, false, false)
}
pub(crate) fn image_route(
    paths: &StorePaths,
    id: &AppId,
    direct: &reqwest::Client,
) -> Result<Route, String> {
    route_for(paths, id, direct, true, false)
}
fn route_for(
    paths: &StorePaths,
    id: &AppId,
    direct: &reqwest::Client,
    images: bool,
    required: bool,
) -> Result<Route, String> {
    let store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    let required = required
        || (images
            && store.load(id).map_err(|e| e.to_string())?.bundle_id == "lt.manodrabuziai.fr");
    let proxy = store.proxy_for_app(id).map_err(|e| e.to_string())?;
    let Some(proxy) = proxy else {
        if required {
            return Err("请先为此虚拟 App 关联代理；Vinted 请求不允许直连".into());
        }
        return Ok(Route {
            client: direct.clone(),
            _tunnel: None,
            gate: None,
            mode: ProxyMode::Shared,
            owner: id.as_str().into(),
            proxy_fingerprint: None,
        });
    };
    let gate = proxy_gate(paths, &proxy.id)?;
    // Cached clients keep HTTP connections pooled across task/chat runner instances.
    let fingerprint = proxy_fingerprint(paths, &proxy)?;
    let key = (
        std::fs::canonicalize(&paths.database).map_err(|_| "代理数据库不可用")?,
        proxy.id.clone(),
        images,
    );
    start_client_reaper();
    let mut cache = CLIENTS.get_or_init(Default::default).lock().unwrap();
    cache.prune();
    if !cache.watchers.contains_key(&key.0) {
        let events = rv_core::events::workspace_bus(paths)
            .map_err(|e| e.to_string())?
            .subscribe::<rv_core::events::WorkspaceChanged>();
        cache.watchers.insert(key.0.clone(), events);
    }
    let clients = &mut cache.entries;
    let client = match clients.get(&key) {
        Some(entry)
            if entry.fingerprint == fingerprint
                && entry.client.tunnel.as_ref().is_none_or(|t| t.alive()) =>
        {
            entry.client.clone()
        }
        _ => {
            let client = proxy_client(paths, &proxy, images)?;
            clients.insert(
                key.clone(),
                CachedClient {
                    fingerprint: fingerprint.clone(),
                    client: client.clone(),
                    last_used: std::time::Instant::now(),
                },
            );
            client
        }
    };
    clients.get_mut(&key).unwrap().last_used = std::time::Instant::now();
    cache.prune();
    Ok(Route {
        client: client.inner,
        _tunnel: client.tunnel,
        gate: Some(gate),
        mode: proxy.mode,
        owner: id.as_str().into(),
        proxy_fingerprint: Some(fingerprint),
    })
}
type ClientKey = (std::path::PathBuf, String, bool);
struct CachedClient {
    fingerprint: Vec<u8>,
    client: ProxyClient,
    last_used: std::time::Instant,
}
#[derive(Default)]
struct ClientCache {
    entries: HashMap<ClientKey, CachedClient>,
    watchers: HashMap<
        std::path::PathBuf,
        rv_core::events::Subscription<rv_core::events::WorkspaceChanged>,
    >,
}
impl ClientCache {
    fn prune(&mut self) {
        let mut changed = Vec::new();
        for (path, events) in &self.watchers {
            while let Ok(event) = events.try_recv() {
                if event
                    .changes
                    .iter()
                    .any(|c| c.domain == rv_core::events::ChangeDomain::Apps && c.app_id.is_none())
                {
                    changed.push(path.clone());
                }
            }
        }
        self.entries.retain(|(path, _, _), entry| {
            !changed.contains(path) && entry.last_used.elapsed() < Duration::from_secs(120)
        });
        while self.entries.len() > 32 {
            let key = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
                .unwrap();
            self.entries.remove(&key);
        }
        self.watchers
            .retain(|path, _| self.entries.keys().any(|(database, _, _)| database == path));
    }
}
static CLIENTS: OnceLock<Mutex<ClientCache>> = OnceLock::new();
fn start_client_reaper() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        std::thread::spawn(|| {
            loop {
                std::thread::sleep(Duration::from_secs(5));
                let mut cache = CLIENTS.get_or_init(Default::default).lock().unwrap();
                if crate::proxy::engine::is_stopping() {
                    cache.entries.clear();
                    cache.watchers.clear();
                    break;
                }
                cache.prune();
            }
        });
    });
}
fn proxy_fingerprint(paths: &StorePaths, proxy: &SocksProxy) -> Result<Vec<u8>, String> {
    let store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    let route = store.resolve_proxy(proxy).map_err(|e| e.to_string())?;
    snapshot_fingerprint(paths, &route)
}
fn snapshot_fingerprint(
    paths: &StorePaths,
    route: &rv_core::workspace::ProxyRoute,
) -> Result<Vec<u8>, String> {
    let store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    let engine = if route.front.is_some() {
        store.proxy_engine_path().map_err(|e| e.to_string())?
    } else {
        None
    };
    Ok(Sha256::digest(serde_json::to_vec(&(route, engine)).map_err(|_| "代理配置无效")?).to_vec())
}
#[cfg(test)]
pub(crate) fn route_with_policy(
    paths: &StorePaths,
    id: &AppId,
    direct: &reqwest::Client,
    required: bool,
) -> Result<Route, String> {
    route_for(paths, id, direct, false, required)
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
            protocol: Default::default(),
            front_proxy_id: None,
            id: uuid::Uuid::new_v4().to_string(),
            name: "test".into(),
            host: "127.0.0.1".into(),
            port,
            username: "user".into(),
            password: "pass".into(),
            mode: ProxyMode::Exclusive,
            ip: Default::default(),
        };
        store.save_proxy(&proxy).unwrap();
        store.bind_proxy(id, Some(&proxy.id)).unwrap();
    }
    #[tokio::test]
    async fn binding_changes_while_waiting_reject_the_old_route() {
        for rebind in [false, true] {
            let (_dir, paths, id) = fixture();
            bind(&paths, &id, 12345);
            let route = route_with_policy(&paths, &id, &builder().build().unwrap(), true).unwrap();
            let gate = route.gate.as_ref().unwrap().clone();
            let occupied = gate.acquire("other-app", ProxyMode::Exclusive).await;
            let mut waiting = Box::pin(route.acquire());
            assert!(
                tokio::time::timeout(Duration::from_millis(20), &mut waiting)
                    .await
                    .is_err()
            );
            if rebind {
                bind(&paths, &id, 12346);
            } else {
                WorkspaceStore::open(&paths)
                    .unwrap()
                    .bind_proxy(&id, None)
                    .unwrap();
            }
            drop(occupied);
            let lease = waiting.await;
            assert!(
                route
                    .validate_binding(&paths, &id)
                    .unwrap_err()
                    .contains("已变更")
            );
            drop(lease);
            assert!(gate.owners.lock().unwrap().is_empty());
        }
    }
    #[tokio::test]
    async fn task_images_preserve_the_acquired_route_and_reject_rebinding() {
        let (_dir, paths, id) = fixture();
        bind(&paths, &id, 12345);
        let route = route_with_policy(&paths, &id, &builder().build().unwrap(), true).unwrap();
        let lease = route.acquire().await;
        let images = route.for_images(&paths, &id).unwrap();
        assert!(Arc::ptr_eq(
            route.gate.as_ref().unwrap(),
            images.gate.as_ref().unwrap()
        ));
        let image_lease = tokio::time::timeout(Duration::from_millis(100), images.acquire())
            .await
            .unwrap();
        drop(image_lease);
        bind(&paths, &id, 12346);
        assert!(route.for_images(&paths, &id).is_err());
        assert!(images.validate_binding(&paths, &id).is_err());
        assert!(
            crate::assets::cache_business_images_on_route(
                &paths,
                &id,
                vec!["https://rv-fixture.invalid/never-request".into()],
                std::time::Instant::now() + Duration::from_secs(1),
                &route,
                false,
            )
            .await
            .is_err()
        );
        drop(lease);
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
    async fn image_deadline_cancels_proxy_wait_and_releases_the_waiter() {
        let (_dir, paths, id) = fixture();
        bind(&paths, &id, 12345);
        let route = route_with_policy(&paths, &id, &builder().build().unwrap(), true).unwrap();
        let gate = route.gate.as_ref().unwrap().clone();
        let occupied = gate.acquire("another-task", ProxyMode::Exclusive).await;
        let start = std::time::Instant::now();
        let deadline = start + Duration::from_millis(60);
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            tokio::task::spawn_blocking(move || {
                crate::assets::fetch_for_app_until(
                    &paths,
                    &id,
                    "https://rv-fixture.invalid/image",
                    deadline,
                )
            }),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(result.unwrap_err().contains("超时"));
        assert_eq!(gate.owners.lock().unwrap().len(), 1);
        drop(occupied);
        assert!(gate.owners.lock().unwrap().is_empty());
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
            protocol: Default::default(),
            front_proxy_id: None,
            id: uuid::Uuid::new_v4().to_string(),
            name: "images".into(),
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
            username: String::new(),
            password: String::new(),
            mode: ProxyMode::Exclusive,
            ip: Default::default(),
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
    #[test]
    fn cached_clients_are_bounded_expire_and_follow_proxy_change_events() {
        let (_dir, paths, id) = fixture();
        bind(&paths, &id, 12345);
        let database = std::fs::canonicalize(&paths.database).unwrap();
        let bus = rv_core::events::workspace_bus(&paths).unwrap();
        let mut cache = ClientCache::default();
        cache.watchers.insert(
            database.clone(),
            bus.subscribe::<rv_core::events::WorkspaceChanged>(),
        );
        let client = ProxyClient {
            inner: builder().build().unwrap(),
            tunnel: None,
        };
        for number in 0..40 {
            cache.entries.insert(
                (database.clone(), number.to_string(), false),
                CachedClient {
                    fingerprint: vec![],
                    client: client.clone(),
                    last_used: std::time::Instant::now(),
                },
            );
        }
        cache.prune();
        assert_eq!(cache.entries.len(), 32);
        for entry in cache.entries.values_mut() {
            entry.last_used = std::time::Instant::now() - Duration::from_secs(121);
        }
        cache.prune();
        assert!(cache.entries.is_empty());
        cache.watchers.insert(
            database.clone(),
            bus.subscribe::<rv_core::events::WorkspaceChanged>(),
        );
        cache.entries.insert(
            (database.clone(), "exit".into(), false),
            CachedClient {
                fingerprint: vec![],
                client,
                last_used: std::time::Instant::now(),
            },
        );
        let mut store = WorkspaceStore::open(&paths).unwrap();
        let mut front = store.proxies().unwrap().remove(0);
        front.port = 12346;
        store.save_proxy(&front).unwrap();
        cache.prune();
        assert!(cache.entries.is_empty());
        assert!(cache.watchers.is_empty());
    }
    #[test]
    fn changing_front_invalidates_captured_business_and_image_routes() {
        let (_dir, paths, id) = fixture();
        bind(&paths, &id, 12345);
        let mut store = WorkspaceStore::open(&paths).unwrap();
        let mut front = store.proxies().unwrap().remove(0);
        let mut exit = front.clone();
        exit.id = uuid::Uuid::new_v4().to_string();
        exit.front_proxy_id = Some(front.id.clone());
        store.save_proxy(&exit).unwrap();
        store.bind_proxy(&id, Some(&exit.id)).unwrap();
        let captured = proxy_fingerprint(&paths, &exit).unwrap();
        let route = Route {
            client: builder().build().unwrap(),
            _tunnel: None,
            gate: None,
            mode: ProxyMode::Shared,
            owner: id.as_str().into(),
            proxy_fingerprint: Some(captured.clone()),
        };
        front.password = "updated-front-credential".into();
        store.save_proxy(&front).unwrap();
        assert_ne!(captured, proxy_fingerprint(&paths, &exit).unwrap());
        assert!(
            route
                .validate_binding(&paths, &id)
                .unwrap_err()
                .contains("已变更")
        );
    }
    #[tokio::test]
    async fn standalone_http_proxy_uses_absolute_uri_and_authentication() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (_dir, paths, _) = fixture();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = SocksProxy {
            protocol: rv_core::workspace::ProxyProtocol::Http,
            id: uuid::Uuid::new_v4().to_string(),
            name: "HTTP".into(),
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
            username: "user".into(),
            password: "pass".into(),
            mode: ProxyMode::Shared,
            front_proxy_id: None,
            ip: Default::default(),
        };
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                bytes.push(socket.read_u8().await.unwrap());
                assert!(bytes.len() < 4096);
            }
            let header = String::from_utf8(bytes).unwrap();
            assert!(header.starts_with("GET http://rv-target.invalid/fixture HTTP/1.1\r\n"));
            assert!(
                header.lines().any(
                    |line| line.eq_ignore_ascii_case("proxy-authorization: Basic dXNlcjpwYXNz")
                )
            );
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                .await
                .unwrap();
        });
        let client = probe_client(&paths, &proxy).unwrap();
        assert_eq!(
            client
                .get("http://rv-target.invalid/fixture")
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "{}"
        );
        server.await.unwrap();
    }
}
