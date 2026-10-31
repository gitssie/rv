//! Embedded meow chains. One process-wide loopback port, immutable authenticated routes.
use rv_core::{
    StorePaths,
    workspace::{ProxyProtocol, ProxyRoute, SocksProxy},
};
use rv_proxy::{Endpoint, ExitProtocol, Gateway, Sessions};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

static STOPPING: AtomicBool = AtomicBool::new(false);
static ENGINE: OnceLock<Mutex<Option<Arc<Engine>>>> = OnceLock::new();
type Tunnels = HashMap<Vec<u8>, Weak<Tunnel>>;
static TUNNELS: OnceLock<Mutex<Tunnels>> = OnceLock::new();
pub struct EngineInfo {
    pub version: String,
}
pub fn info(_paths: &StorePaths) -> Result<EngineInfo, String> {
    Ok(EngineInfo {
        version: "meow-rs 0.28.0".into(),
    })
}
struct Engine {
    sessions: Sessions,
    port: u16,
    secret: String,
    alive: Arc<AtomicBool>,
    stop: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}
impl Engine {
    fn start() -> Result<Arc<Self>, String> {
        let secret = uuid::Uuid::new_v4().simple().to_string();
        let sessions = Sessions::new(secret.clone());
        let routes = sessions.clone();
        let alive = Arc::new(AtomicBool::new(false));
        let running = alive.clone();
        let (stop, done) = tokio::sync::oneshot::channel();
        let (ready, started) = std::sync::mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("rv-proxy".into())
            .spawn(move || {
                struct Running(Arc<AtomicBool>);
                impl Drop for Running {
                    fn drop(&mut self) {
                        self.0.store(false, Ordering::SeqCst);
                    }
                }
                let _guard = Running(running.clone());
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    let _ = ready.send(Err("无法初始化代理运行环境"));
                    return;
                };
                runtime.block_on(async move {
                    let gateway = match Gateway::start(routes.clone()).await {
                        Ok(g) => g,
                        Err(_) => {
                            let _ = ready.send(Err("无法启动本地代理入口"));
                            return;
                        }
                    };
                    running.store(true, Ordering::SeqCst);
                    if ready.send(Ok(gateway.address.port())).is_err() {
                        return;
                    }
                    tokio::pin!(done);
                    let mut reaper = tokio::time::interval(Duration::from_secs(5));
                    loop {
                        tokio::select! {
                            _=&mut done => break,
                            _=reaper.tick()=>{ if !gateway.alive() { break; } }
                        }
                    }
                    drop(gateway);
                });
            })
            .map_err(|_| "无法启动代理运行线程")?;
        match started.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(port)) => Ok(Arc::new(Self {
                sessions,
                port,
                secret,
                alive,
                stop: Mutex::new(Some(stop)),
                thread: Mutex::new(Some(thread)),
            })),
            error => {
                let _ = stop.send(());
                let _ = thread.join();
                Err(match error {
                    Ok(Err(e)) => e,
                    _ => "本地代理入口启动超时",
                }
                .into())
            }
        }
    }
    fn stop(&self) {
        self.alive.store(false, Ordering::SeqCst);
        if let Some(stop) = self.stop.lock().unwrap().take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.lock().unwrap().take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        self.stop();
    }
}
pub(crate) struct Tunnel {
    engine: Arc<Engine>,
    active: AtomicBool,
    pub port: u16,
    pub username: String,
    pub password: String,
}
impl Tunnel {
    pub(crate) fn alive(&self) -> bool {
        self.active.load(Ordering::SeqCst) && self.engine.alive.load(Ordering::SeqCst)
    }
    fn stop(&self) {
        self.active.store(false, Ordering::SeqCst);
        self.engine.sessions.remove(&self.username);
    }
}
impl Drop for Tunnel {
    fn drop(&mut self) {
        self.stop();
    }
}
pub(crate) fn is_stopping() -> bool {
    STOPPING.load(Ordering::SeqCst)
}
pub fn shutdown() {
    STOPPING.store(true, Ordering::SeqCst);
    if let Some(engine) = ENGINE.get() {
        if let Some(engine) = engine.lock().unwrap().take() {
            engine.stop();
        }
    }
}
fn protocol(proxy: &SocksProxy) -> ExitProtocol {
    match proxy.protocol {
        ProxyProtocol::Http => ExitProtocol::Http,
        ProxyProtocol::Socks5 => ExitProtocol::Socks5,
    }
}
fn endpoint(proxy: &SocksProxy) -> Endpoint {
    Endpoint {
        host: proxy.host.trim_matches(['[', ']']).into(),
        port: proxy.port,
        username: proxy.username.clone(),
        password: proxy.password.clone(),
    }
}
pub(crate) fn acquire(paths: &StorePaths, route: &ProxyRoute) -> Result<Arc<Tunnel>, String> {
    let front = route.front.as_ref().ok_or("缺少前置代理")?;
    if is_stopping() {
        return Err("代理引擎正在退出".into());
    }
    let key = Sha256::digest(
        serde_json::to_vec(&(
            std::fs::canonicalize(&paths.database).map_err(|_| "代理数据库不可用")?,
            route,
        ))
        .map_err(|_| "代理配置无效")?,
    )
    .to_vec();
    let mut tunnels = TUNNELS.get_or_init(Default::default).lock().unwrap();
    tunnels.retain(|_, t| t.strong_count() > 0);
    if let Some(tunnel) = tunnels
        .get(&key)
        .and_then(Weak::upgrade)
        .filter(|t| t.alive())
    {
        return Ok(tunnel);
    }
    let mut current = ENGINE.get_or_init(Default::default).lock().unwrap();
    if is_stopping() {
        return Err("代理引擎正在退出".into());
    }
    if current
        .as_ref()
        .is_none_or(|e| !e.alive.load(Ordering::SeqCst))
    {
        *current = Some(Engine::start()?);
    }
    let engine = current.as_ref().unwrap().clone();
    let username = uuid::Uuid::new_v4().simple().to_string();
    engine.sessions.insert(
        username.clone(),
        rv_proxy::chain_with_front(
            endpoint(front),
            protocol(front),
            endpoint(&route.exit),
            protocol(&route.exit),
        ),
    );
    let tunnel = Arc::new(Tunnel {
        port: engine.port,
        password: engine.secret.clone(),
        username,
        engine,
        active: AtomicBool::new(true),
    });
    tunnels.insert(key, Arc::downgrade(&tunnel));
    Ok(tunnel)
}
#[cfg(test)]
mod tests;
