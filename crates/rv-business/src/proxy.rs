//! Stable proxy-exit leases, sharing the gate used by ordinary business requests.
//! Preparation runs once per owner/lease scope. History records observations,
//! and is never rolled back when work fails or a lease is cancelled.
pub mod engine;
mod rotation;
use crate::network;
use rv_core::{
    StorePaths,
    workspace::{AppId, SocksProxy, WorkspaceStore},
};
use std::{
    net::Ipv4Addr,
    sync::atomic::{AtomicU8, Ordering},
    time::Duration,
};
use tokio::{sync::OnceCell, time::Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IpPolicy {
    Reuse,
    Rotate,
    NeverSeen,
}

#[derive(Clone, Copy, Debug)]
pub struct AcquireOptions {
    pub max_changes: u8,
    pub settle_timeout: Duration,
    pub poll_interval: Duration,
    /// Excludes waiting for another owner to release the gate.
    pub preparation_timeout: Duration,
}
impl Default for AcquireOptions {
    fn default() -> Self {
        Self {
            max_changes: 3,
            settle_timeout: Duration::from_secs(20),
            poll_interval: Duration::from_secs(1),
            preparation_timeout: Duration::from_secs(75),
        }
    }
}
impl AcquireOptions {
    fn valid(self) -> bool {
        (1..=10).contains(&self.max_changes)
            && !self.poll_interval.is_zero()
            && self.poll_interval < self.settle_timeout
            && self.settle_timeout <= self.preparation_timeout
            && self.preparation_timeout <= Duration::from_secs(300)
    }
}

/// Safe for UI/logging: no URLs, credentials or response payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyError {
    InvalidOptions,
    InvalidOwner,
    ProxyUnavailable,
    Unbound,
    InvalidConfiguration,
    MissingChangeUrl,
    ReentryConflict,
    PreparationInterrupted,
    PreparationTimedOut,
    ProbeFailed,
    InvalidIp,
    RotationRejected,
    RotationUnconfirmed,
    HistoryUnavailable,
    HistoryExhausted,
}
impl std::fmt::Display for ProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidOptions => "代理准备参数无效",
            Self::InvalidOwner => "代理持有者标识无效",
            Self::ProxyUnavailable => "代理配置不可用",
            Self::Unbound => "尚未绑定代理出口",
            Self::InvalidConfiguration => "代理 IP 配置无效",
            Self::MissingChangeUrl => "代理未配置换 IP 方式",
            Self::ReentryConflict => "当前租约不满足更严格的 IP 要求",
            Self::PreparationInterrupted => "出口准备已中断，请结束当前租约后重试",
            Self::PreparationTimedOut => "出口准备超时",
            Self::ProbeFailed => "无法查询代理出口 IP",
            Self::InvalidIp => "查询结果不是有效的公网 IPv4",
            Self::RotationRejected => "换 IP 请求被拒绝",
            Self::RotationUnconfirmed => "未确认出口 IP 已切换",
            Self::HistoryUnavailable => "IP 历史库不可用",
            Self::HistoryExhausted => "已达到换 IP 次数上限，未获得符合要求的 IP",
        })
    }
}
impl std::error::Error for ProxyError {}

#[derive(Clone)]
pub struct ProxyManager {
    paths: StorePaths,
}
impl ProxyManager {
    pub fn new(paths: StorePaths) -> Self {
        Self { paths }
    }

    pub async fn acquire(
        &self,
        proxy_id: &str,
        owner_id: &str,
        policy: IpPolicy,
    ) -> Result<ProxyLease, ProxyError> {
        self.acquire_with_options(proxy_id, owner_id, policy, AcquireOptions::default())
            .await
    }

    /// Uses the existing IDFV binding; never silently substitutes another exit.
    pub async fn acquire_bound(
        &self,
        app: &AppId,
        owner_id: &str,
        policy: IpPolicy,
    ) -> Result<ProxyLease, ProxyError> {
        let proxy_id = WorkspaceStore::open(&self.paths)
            .map_err(|_| ProxyError::ProxyUnavailable)?
            .proxy_binding(app)
            .map_err(|_| ProxyError::ProxyUnavailable)?
            .ok_or(ProxyError::Unbound)?;
        self.acquire(&proxy_id, owner_id, policy).await
    }

    /// Dropping this future cancels waiting/preparation. Dropping the returned
    /// lease releases its count. Keep it alive for the full request sequence.
    pub async fn acquire_with_options(
        &self,
        proxy_id: &str,
        owner_id: &str,
        policy: IpPolicy,
        options: AcquireOptions,
    ) -> Result<ProxyLease, ProxyError> {
        if !options.valid() {
            return Err(ProxyError::InvalidOptions);
        }
        if owner_id.is_empty() || owner_id.len() > 256 || owner_id.chars().any(char::is_control) {
            return Err(ProxyError::InvalidOwner);
        }
        load_proxy(&self.paths, proxy_id)?;
        let gate =
            network::proxy_gate(&self.paths, proxy_id).map_err(|_| ProxyError::ProxyUnavailable)?;
        let candidate = PreparationSlot {
            paths: self.paths.clone(),
            proxy_id: proxy_id.into(),
            owner: owner_id.into(),
            policy,
            options,
            allow_rotation: true,
            started: AtomicU8::new(0),
            ready: OnceCell::new(),
        };
        // Task owners and legacy IDFV owners occupy separate namespaces.
        // A business request must not bypass an in-progress managed rotation
        // merely because its App ID happens to match the task's owner string.
        let lock = gate.acquire_managed(owner_id, candidate).await?;
        let slot = lock
            .preparation
            .as_ref()
            .expect("managed gate installs a preparation slot");
        // The first acquisition determines preparation. Parallel/nested callers
        // join that same initialization and cannot trigger a second rotation.
        let prepared = slot
            .ready
            .get_or_init(|| async {
                if slot
                    .started
                    .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
                    .is_err()
                {
                    return Err(ProxyError::PreparationInterrupted);
                }
                let mut interrupted = Interrupted {
                    state: &slot.started,
                    armed: true,
                };
                let result = tokio::time::timeout(slot.options.preparation_timeout, prepare(slot))
                    .await
                    .unwrap_or(Err(ProxyError::PreparationTimedOut));
                interrupted.armed = false;
                slot.started.store(3, Ordering::SeqCst);
                result
            })
            .await
            .clone()?;
        if !prepared.satisfies(policy) {
            return Err(ProxyError::ReentryConflict);
        }
        Ok(ProxyLease {
            lock,
            exit: prepared,
        })
    }
}

pub struct ProxyLease {
    lock: network::Lease,
    exit: PreparedExit,
}
impl ProxyLease {
    pub fn ip(&self) -> Ipv4Addr {
        self.exit.ip
    }
    /// Requests made with this client must remain within the lease's lifetime.
    pub fn client(&self) -> &reqwest::Client {
        &self.exit.client
    }
    pub fn proxy_id(&self) -> &str {
        &self.lock.preparation.as_ref().unwrap().proxy_id
    }
    pub async fn reenter(&self, policy: IpPolicy) -> Result<Self, ProxyError> {
        let slot = self.lock.preparation.as_ref().unwrap();
        ProxyManager::new(slot.paths.clone())
            .acquire_with_options(&slot.proxy_id, &slot.owner, policy, slot.options)
            .await
    }
}
impl std::fmt::Debug for ProxyLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyLease")
            .field("ip", &self.exit.ip)
            .finish_non_exhaustive()
    }
}

pub(crate) struct PreparationSlot {
    paths: StorePaths,
    proxy_id: String,
    owner: String,
    policy: IpPolicy,
    options: AcquireOptions,
    pub(crate) allow_rotation: bool,
    started: AtomicU8,
    ready: OnceCell<Result<PreparedExit, ProxyError>>,
}
impl PreparationSlot {
    pub(crate) fn owner_id(&self) -> &str {
        &self.owner
    }
    pub(crate) fn requires_rotation(&self) -> bool {
        self.policy != IpPolicy::Reuse
    }
}
struct Interrupted<'a> {
    state: &'a AtomicU8,
    armed: bool,
}
impl Drop for Interrupted<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.state.store(2, Ordering::SeqCst);
        }
    }
}
#[derive(Clone)]
struct PreparedExit {
    snapshot: rv_core::workspace::ProxyRoute,
    ip: Ipv4Addr,
    rotated: bool,
    first_seen: bool,
    client: network::ProxyClient,
}
impl PreparedExit {
    fn satisfies(&self, policy: IpPolicy) -> bool {
        match policy {
            IpPolicy::Reuse => true,
            IpPolicy::Rotate => self.rotated,
            IpPolicy::NeverSeen => self.rotated && self.first_seen,
        }
    }
}

fn load_proxy(paths: &StorePaths, id: &str) -> Result<SocksProxy, ProxyError> {
    WorkspaceStore::open(paths)
        .map_err(|_| ProxyError::ProxyUnavailable)?
        .proxies()
        .map_err(|_| ProxyError::ProxyUnavailable)?
        .into_iter()
        .find(|proxy| proxy.id == id)
        .ok_or(ProxyError::ProxyUnavailable)
}
fn record(slot: &PreparationSlot, ip: Ipv4Addr) -> Result<bool, ProxyError> {
    WorkspaceStore::open(&slot.paths)
        .map_err(|_| ProxyError::HistoryUnavailable)?
        .record_proxy_ip(ip, &slot.proxy_id, &slot.owner)
        .map_err(|_| ProxyError::HistoryUnavailable)
}

async fn bounded_body(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, ProxyError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ProxyError::ProbeFailed)?
    {
        if bytes.len() + chunk.len() > limit {
            return Err(ProxyError::ProbeFailed);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn parse_ip(bytes: &[u8]) -> Result<Ipv4Addr, ProxyError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| ProxyError::InvalidIp)?
        .trim();
    let value = serde_json::from_str::<serde_json::Value>(text).ok();
    let raw = if let Some(value) = &value {
        if value.get("success").is_some_and(|v| v != true) {
            return Err(ProxyError::ProbeFailed);
        }
        value["ip"].as_str().ok_or(ProxyError::InvalidIp)?
    } else {
        text
    };
    let ip: Ipv4Addr = raw.parse().map_err(|_| ProxyError::InvalidIp)?;
    let [a, b, c, d] = ip.octets();
    // IANA special-purpose ranges that are not public unicast exits.
    let shared = a == 100 && (64..=127).contains(&b);
    let benchmarking = a == 198 && (b == 18 || b == 19);
    let deprecated_relay = a == 192 && b == 88 && c == 99;
    let protocol_assignment = a == 192 && b == 0 && c == 0 && d != 9 && d != 10;
    if ip.is_private()
        || ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip.is_documentation()
        || shared
        || benchmarking
        || deprecated_relay
        || protocol_assignment
        || a == 0
        || a >= 240
    {
        return Err(ProxyError::InvalidIp);
    }
    Ok(ip)
}
async fn probe(client: &reqwest::Client, url: &str) -> Result<Ipv4Addr, ProxyError> {
    let response = client
        .get(url)
        .header(reqwest::header::CACHE_CONTROL, "no-cache")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|_| ProxyError::ProbeFailed)?;
    if !response.status().is_success() {
        return Err(ProxyError::ProbeFailed);
    }
    parse_ip(&bounded_body(response, 4096).await?)
}

// Format recognized query fields for display without validating their values.
fn format_observation(body: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return body.to_owned();
    };
    let Some(ip) = value.get("ip").and_then(|v| v.as_str()) else {
        return body.to_owned();
    };
    let mut location = Vec::new();
    for keys in [
        &["country", "country_name"][..],
        &["region", "regionName"][..],
        &["city"][..],
    ] {
        if let Some(part) = keys
            .iter()
            .find_map(|key| value.get(*key).and_then(|v| v.as_str()))
        {
            let part = part.trim();
            if !part.is_empty() && !location.iter().any(|p| p == part) {
                location.push(part.to_owned());
            }
        }
    }
    if location.is_empty() {
        format!("IP：{ip}")
    } else {
        format!("IP：{ip}　地区：{}", location.join(" · "))
    }
}

/// Displays query fields without IP validation; never rotates or writes history.
pub fn inspect_blocking(paths: &StorePaths, proxy: &SocksProxy) -> Result<String, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "无法启动代理检测".to_owned())?
        .block_on(inspect(paths, proxy))
        .map_err(|error| error.to_string())
}
pub async fn inspect(paths: &StorePaths, proxy: &SocksProxy) -> Result<String, ProxyError> {
    // Only the observation URL matters: an unfinished change URL must not block a test.
    rv_core::workspace::ProxyIpConfig {
        rotation: Default::default(),
        change_url: None,
        probe_url: proxy.ip.probe_url.clone(),
    }
    .validate()
    .map_err(|_| ProxyError::InvalidConfiguration)?;
    if proxy.host.trim().is_empty() || proxy.port == 0 {
        return Err(ProxyError::InvalidConfiguration);
    }
    let gate = network::proxy_gate(paths, &proxy.id).map_err(|_| ProxyError::ProxyUnavailable)?;
    let owner = format!("proxy-inspect-{}", uuid::Uuid::new_v4());
    let _lease = gate.acquire(&owner, proxy.mode).await;
    let client =
        network::probe_client(paths, proxy).map_err(|_| ProxyError::InvalidConfiguration)?;
    let response = client
        .get(&proxy.ip.probe_url)
        .header(reqwest::header::CACHE_CONTROL, "no-cache")
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|_| ProxyError::ProbeFailed)?;
    let body = bounded_body(response, 16384).await?;
    Ok(format_observation(&String::from_utf8_lossy(&body)))
}

/// Ordinary business traffic already holds the proxy's exclusive occupancy lease.
/// Reuse the same rotation implementation without recursively acquiring that gate.
pub(crate) async fn rotate_for_business(
    paths: &StorePaths,
    proxy_id: &str,
    owner: &str,
) -> Result<(network::ProxyClient, rv_core::workspace::ProxyRoute), String> {
    let slot = PreparationSlot {
        paths: paths.clone(),
        proxy_id: proxy_id.into(),
        owner: owner.into(),
        policy: IpPolicy::Rotate,
        options: AcquireOptions::default(),
        allow_rotation: true,
        started: AtomicU8::new(0),
        ready: OnceCell::new(),
    };
    tokio::time::timeout(slot.options.preparation_timeout, prepare(&slot))
        .await
        .map_err(|_| ProxyError::PreparationTimedOut.to_string())?
        .map(|exit| (exit.client, exit.snapshot))
        .map_err(|e| e.to_string())
}

async fn prepare(slot: &PreparationSlot) -> Result<PreparedExit, ProxyError> {
    let mut proxy = load_proxy(&slot.paths, &slot.proxy_id)?;
    proxy
        .ip
        .validate()
        .map_err(|_| ProxyError::InvalidConfiguration)?;
    if slot.requires_rotation()
        && (!slot.allow_rotation
            || matches!(proxy.ip.rotation, rv_core::workspace::ProxyRotation::Http)
                && proxy.ip.change_url.is_none())
    {
        return Err(if !slot.allow_rotation {
            ProxyError::ReentryConflict
        } else {
            ProxyError::MissingChangeUrl
        });
    }
    let front = WorkspaceStore::open(&slot.paths)
        .and_then(|store| store.resolve_proxy(&proxy))
        .map_err(|_| ProxyError::InvalidConfiguration)?
        .front;
    let mut client = network::probe_snapshot_client(&slot.paths, &proxy, &front)
        .map_err(|_| ProxyError::InvalidConfiguration)?;
    let mut previous = probe(&client, &proxy.ip.probe_url).await?;
    let first_seen = record(slot, previous)?;
    if slot.policy == IpPolicy::Reuse {
        return Ok(PreparedExit {
            ip: previous,
            rotated: false,
            first_seen,
            snapshot: rv_core::workspace::ProxyRoute {
                exit: proxy.clone(),
                front: front.clone(),
            },
            client,
        });
    }
    for _ in 0..slot.options.max_changes {
        client = rotation::switch_exit(&slot.paths, &mut proxy, &front).await?;
        let deadline = Instant::now() + slot.options.settle_timeout;
        let mut candidate: Option<(Ipv4Addr, bool)> = None;
        let stable = loop {
            let result =
                tokio::time::timeout_at(deadline, probe(&client, &proxy.ip.probe_url)).await;
            match result {
                Err(_) => return Err(ProxyError::RotationUnconfirmed),
                Ok(Ok(ip)) => {
                    let unseen = record(slot, ip)?;
                    if ip != previous {
                        if let Some((prior, claimed)) = candidate {
                            if prior == ip {
                                break (ip, claimed);
                            }
                        }
                        candidate = Some((ip, unseen));
                    } else {
                        candidate = None;
                    }
                }
                Ok(Err(_)) => {}
            }
            if Instant::now() >= deadline {
                return Err(ProxyError::RotationUnconfirmed);
            }
            tokio::time::sleep_until((Instant::now() + slot.options.poll_interval).min(deadline))
                .await;
        };
        if slot.policy != IpPolicy::NeverSeen || stable.1 {
            return Ok(PreparedExit {
                ip: stable.0,
                rotated: true,
                first_seen: stable.1,
                snapshot: rv_core::workspace::ProxyRoute {
                    exit: proxy.clone(),
                    front: front.clone(),
                },
                client,
            });
        }
        previous = stable.0;
    }
    Err(ProxyError::HistoryExhausted)
}

#[cfg(test)]
mod tests;
