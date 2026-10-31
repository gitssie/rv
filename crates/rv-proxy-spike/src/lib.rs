//! Isolated feasibility spike: one loopback CONNECT entrance, in-memory sessions,
//! meow-rs SOCKS5 → HTTP/SOCKS5 chains. Not wired into RV's business transport.
use base64::{Engine, engine::general_purpose::STANDARD};
use meow_common::{Metadata, Network, Proxy};
use meow_proxy::{HttpAdapter, RelayGroup, Socks5Adapter, dialer::DirectDialer};
use std::{
    collections::HashMap,
    io,
    net::SocketAddr,
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
};

#[derive(Clone, Copy)]
pub enum ExitProtocol {
    Http,
    Socks5,
}

/// Credentials stay in memory and deliberately do not implement Debug.
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

// The leaf adapters expose ProxyAdapter; RelayGroup consumes the richer Proxy
// interface. Bridge that public API without pulling in the YAML/config engine.
struct Leaf<T>(T);
fn leaf<T: meow_common::ProxyAdapter + 'static>(adapter: T) -> Arc<dyn Proxy> {
    Arc::new(Leaf(adapter))
}
#[async_trait::async_trait]
impl<T: meow_common::ProxyAdapter> meow_common::ProxyAdapter for Leaf<T> {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn adapter_type(&self) -> meow_common::AdapterType {
        self.0.adapter_type()
    }
    fn addr(&self) -> &str {
        self.0.addr()
    }
    fn support_udp(&self) -> bool {
        self.0.support_udp()
    }
    async fn dial_tcp(
        &self,
        meta: &Metadata,
    ) -> meow_common::Result<Box<dyn meow_common::ProxyConn>> {
        self.0.dial_tcp(meta).await
    }
    async fn dial_udp(
        &self,
        meta: &Metadata,
    ) -> meow_common::Result<Box<dyn meow_common::ProxyPacketConn>> {
        self.0.dial_udp(meta).await
    }
    async fn connect_over(
        &self,
        stream: Box<dyn meow_common::ProxyConn>,
        meta: &Metadata,
    ) -> meow_common::Result<Box<dyn meow_common::ProxyConn>> {
        self.0.connect_over(stream, meta).await
    }
    fn health(&self) -> &meow_common::ProxyHealth {
        self.0.health()
    }
    fn reset_sessions(&self) {
        self.0.reset_sessions();
    }
}
impl<T: meow_common::ProxyAdapter> Proxy for Leaf<T> {
    fn alive(&self) -> bool {
        self.0.health().alive()
    }
    fn alive_for_url(&self, _: &str) -> bool {
        self.alive()
    }
    fn last_delay(&self) -> u16 {
        self.0.health().last_delay()
    }
    fn last_delay_for_url(&self, _: &str) -> u16 {
        self.last_delay()
    }
    fn delay_history(&self) -> Vec<meow_common::DelayHistory> {
        self.0.health().delay_history()
    }
}

/// Construct a fresh immutable chain for a session. Both hops are mandatory.
pub fn chain(front: Endpoint, exit: Endpoint, protocol: ExitProtocol) -> Arc<dyn Proxy> {
    let front: Arc<dyn Proxy> = leaf(Socks5Adapter::new(
        "front",
        &front.host,
        front.port,
        Some((front.username, front.password)),
        false,
        false,
        Arc::new(DirectDialer),
    ));
    let auth = Some((exit.username, exit.password));
    let exit: Arc<dyn Proxy> = match protocol {
        ExitProtocol::Http => leaf(HttpAdapter::new(
            "exit",
            &exit.host,
            exit.port,
            auth,
            false,
            false,
            vec![],
            Arc::new(DirectDialer),
        )),
        ExitProtocol::Socks5 => leaf(Socks5Adapter::new(
            "exit",
            &exit.host,
            exit.port,
            auth,
            false,
            false,
            Arc::new(DirectDialer),
        )),
    };
    Arc::new(RelayGroup::new("session", vec![front, exit]))
}

struct Session {
    route: Arc<dyn Proxy>,
    last_used: Mutex<Instant>,
}
struct Lease(Arc<Session>);
impl Drop for Lease {
    fn drop(&mut self) {
        *self.0.last_used.lock().unwrap() = Instant::now();
    }
}

/// The map and local gateway credential disappear when the RV process exits.
#[derive(Clone)]
pub struct Sessions {
    entries: Arc<RwLock<HashMap<String, Arc<Session>>>>,
    secret: Arc<String>,
}
impl Sessions {
    pub fn new(secret: String) -> Self {
        Self {
            entries: Arc::default(),
            secret: Arc::new(secret),
        }
    }
    /// Replacing a key affects future connections; an existing Lease retains its chain.
    pub fn insert(&self, key: String, route: Arc<dyn Proxy>) {
        self.entries.write().unwrap().insert(
            key,
            Arc::new(Session {
                route,
                last_used: Mutex::new(Instant::now()),
            }),
        );
    }
    pub fn remove(&self, key: &str) {
        self.entries.write().unwrap().remove(key);
    }
    pub fn reap_idle(&self, ttl: Duration) -> usize {
        let mut entries = self.entries.write().unwrap();
        let before = entries.len();
        entries.retain(|_, session| {
            Arc::strong_count(session) > 1 || session.last_used.lock().unwrap().elapsed() < ttl
        });
        before - entries.len()
    }
    pub fn len(&self) -> usize {
        self.entries.read().unwrap().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn acquire(&self, auth: &str) -> Option<Lease> {
        let encoded = auth.strip_prefix("Basic ")?;
        let decoded = STANDARD.decode(encoded).ok()?;
        let text = std::str::from_utf8(&decoded).ok()?;
        let (key, secret) = text.split_once(':')?;
        if secret != self.secret.as_str() {
            return None;
        }
        self.entries.read().unwrap().get(key).cloned().map(Lease)
    }
}

pub struct Gateway {
    pub address: SocketAddr,
    task: JoinHandle<()>,
}
impl Drop for Gateway {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Gateway {
    pub async fn start(sessions: Sessions) -> io::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let task = tokio::spawn(async move {
            let mut jobs = JoinSet::new();
            loop {
                tokio::select! {
                    socket = listener.accept() => {
                        let Ok((socket, _)) = socket else { break; };
                        let sessions = sessions.clone();
                        jobs.spawn(async move { let _ = serve(socket, sessions).await; });
                    }
                    _ = jobs.join_next(), if !jobs.is_empty() => {}
                }
            }
        });
        Ok(Self { address, task })
    }
}

async fn reject(socket: &mut BufReader<TcpStream>, status: &str) -> io::Result<()> {
    let challenge = if status.starts_with("407 ") {
        "Proxy-Authenticate: Basic realm=\"rv-proxy-spike\"\r\n"
    } else {
        ""
    };
    socket
        .write_all(
            format!(
                "HTTP/1.1 {status}\r\n{challenge}Content-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await?;
    socket.shutdown().await
}
async fn serve(socket: TcpStream, sessions: Sessions) -> io::Result<()> {
    let mut socket = BufReader::new(socket);
    let headers = tokio::time::timeout(Duration::from_secs(3), async {
        let mut bytes = Vec::new();
        while bytes.len() < 8192 {
            bytes.push(socket.read_u8().await?);
            if bytes.ends_with(b"\r\n\r\n") {
                return Ok::<_, io::Error>(bytes);
            }
        }
        Err(io::Error::other("CONNECT headers too large"))
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "CONNECT timed out"))??;
    let mut parsed_headers = [httparse::EMPTY_HEADER; 32];
    let mut request = httparse::Request::new(&mut parsed_headers);
    if !matches!(request.parse(&headers), Ok(httparse::Status::Complete(_))) {
        return reject(&mut socket, "400 Bad Request").await;
    }
    if request.method != Some("CONNECT") {
        return reject(&mut socket, "405 Method Not Allowed").await;
    }
    let Some(target) = request
        .path
        .and_then(|path| path.parse::<http::uri::Authority>().ok())
    else {
        return reject(&mut socket, "400 Bad Request").await;
    };
    let Some(port) = target.port_u16().filter(|port| *port > 0) else {
        return reject(&mut socket, "400 Bad Request").await;
    };
    let auth = request
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("proxy-authorization"))
        .and_then(|h| std::str::from_utf8(h.value).ok());
    let Some(lease) = auth.and_then(|auth| sessions.acquire(auth)) else {
        return reject(&mut socket, "407 Proxy Authentication Required").await;
    };
    let host = target.host().trim_start_matches('[').trim_end_matches(']');
    let mut metadata = Metadata {
        network: Network::Tcp,
        dst_port: port,
        ..Default::default()
    };
    if let Ok(ip) = host.parse() {
        metadata.dst_ip = Some(ip);
    } else {
        metadata.host = host.into();
    }
    let upstream =
        tokio::time::timeout(Duration::from_secs(3), lease.0.route.dial_tcp(&metadata)).await;
    let mut upstream = match upstream {
        Ok(Ok(stream)) => stream,
        _ => return reject(&mut socket, "502 Bad Gateway").await,
    };
    socket
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;
    tokio::io::copy_bidirectional(&mut socket, &mut upstream).await?;
    drop(lease);
    Ok(())
}

#[cfg(test)]
mod tests;
