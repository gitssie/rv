//! Embedded meow proxy chains with one authenticated loopback entrance.
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
    chain_with_front(front, ExitProtocol::Socks5, exit, protocol)
}

/// Both hop protocols are explicit; no detection, retries or direct fallback.
pub fn chain_with_front(
    front: Endpoint,
    front_protocol: ExitProtocol,
    exit: Endpoint,
    protocol: ExitProtocol,
) -> Arc<dyn Proxy> {
    Arc::new(RelayGroup::new(
        "session",
        vec![adapter(front, front_protocol), adapter(exit, protocol)],
    ))
}
fn adapter(endpoint: Endpoint, protocol: ExitProtocol) -> Arc<dyn Proxy> {
    let auth = if endpoint.username.is_empty() && endpoint.password.is_empty() {
        None
    } else {
        Some((endpoint.username, endpoint.password))
    };
    match protocol {
        ExitProtocol::Http => leaf(HttpAdapter::new(
            "hop",
            &endpoint.host,
            endpoint.port,
            auth,
            false,
            false,
            vec![],
            Arc::new(DirectDialer),
        )),
        ExitProtocol::Socks5 => leaf(Socks5Adapter::new(
            "hop",
            &endpoint.host,
            endpoint.port,
            auth,
            false,
            false,
            Arc::new(DirectDialer),
        )),
    }
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
        self.acquire_credentials(key, secret)
    }
    fn acquire_credentials(&self, key: &str, secret: &str) -> Option<Lease> {
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
    pub fn alive(&self) -> bool {
        !self.task.is_finished()
    }
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
        "Proxy-Authenticate: Basic realm=\"rv-proxy\"\r\n"
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
async fn serve_http(socket: TcpStream, sessions: Sessions) -> io::Result<()> {
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

async fn serve(socket: TcpStream, sessions: Sessions) -> io::Result<()> {
    let mut first = [0];
    let size = tokio::time::timeout(Duration::from_secs(5), socket.peek(&mut first))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "proxy handshake timed out"))??;
    if size == 0 {
        return Ok(());
    }
    if first[0] == 5 {
        serve_socks(socket, sessions).await
    } else {
        serve_http(socket, sessions).await
    }
}
async fn serve_socks(mut socket: TcpStream, sessions: Sessions) -> io::Result<()> {
    let handshake = async {
        if socket.read_u8().await? != 5 {
            return Err(io::Error::other("invalid SOCKS version"));
        }
        let mut methods = vec![0; usize::from(socket.read_u8().await?)];
        socket.read_exact(&mut methods).await?;
        if !methods.contains(&2) {
            socket.write_all(&[5, 255]).await?;
            return Err(io::Error::other("authentication required"));
        }
        socket.write_all(&[5, 2]).await?;
        if socket.read_u8().await? != 1 {
            return Err(io::Error::other("invalid authentication version"));
        }
        let mut user = vec![0; usize::from(socket.read_u8().await?)];
        socket.read_exact(&mut user).await?;
        let mut password = vec![0; usize::from(socket.read_u8().await?)];
        socket.read_exact(&mut password).await?;
        let lease = std::str::from_utf8(&user)
            .ok()
            .zip(std::str::from_utf8(&password).ok())
            .and_then(|(u, p)| sessions.acquire_credentials(u, p));
        let Some(lease) = lease else {
            socket.write_all(&[1, 1]).await?;
            return Err(io::Error::other("invalid proxy credentials"));
        };
        socket.write_all(&[1, 0]).await?;
        let mut header = [0; 4];
        socket.read_exact(&mut header).await?;
        if header[..3] != [5, 1, 0] {
            socket.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            return Err(io::Error::other("unsupported SOCKS command"));
        }
        let mut meta = Metadata {
            network: Network::Tcp,
            ..Default::default()
        };
        match header[3] {
            1 => {
                let mut ip = [0; 4];
                socket.read_exact(&mut ip).await?;
                meta.dst_ip = Some(std::net::Ipv4Addr::from(ip).into());
            }
            4 => {
                let mut ip = [0; 16];
                socket.read_exact(&mut ip).await?;
                meta.dst_ip = Some(std::net::Ipv6Addr::from(ip).into());
            }
            3 => {
                let mut host = vec![0; usize::from(socket.read_u8().await?)];
                socket.read_exact(&mut host).await?;
                if host.is_empty() {
                    return Err(io::Error::other("empty destination"));
                }
                meta.host = std::str::from_utf8(&host)
                    .map_err(|_| io::Error::other("invalid destination"))?
                    .into();
            }
            _ => {
                socket.write_all(&[5, 8, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
                return Err(io::Error::other("unsupported address"));
            }
        }
        meta.dst_port = socket.read_u16().await?;
        if meta.dst_port == 0 {
            return Err(io::Error::other("invalid destination port"));
        }
        Ok::<_, io::Error>((lease, meta))
    };
    let (lease, meta) = tokio::time::timeout(Duration::from_secs(5), handshake)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "proxy handshake timed out"))??;
    let upstream =
        tokio::time::timeout(Duration::from_secs(10), lease.0.route.dial_tcp(&meta)).await;
    let mut upstream = match upstream {
        Ok(Ok(stream)) => stream,
        _ => {
            socket.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
            return Ok(());
        }
    };
    socket.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
    tokio::io::copy_bidirectional(&mut socket, &mut upstream).await?;
    drop(lease);
    Ok(())
}
