//! External, user-installed sing-box. Immutable route instances never switch an
//! in-flight request to a different exit. Config travels over stdin, not argv/files.
use rv_core::{
    StorePaths,
    workspace::{ProxyRoute, SocksProxy, WorkspaceStore},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

static STOPPING: AtomicBool = AtomicBool::new(false);
type Tunnels = HashMap<Vec<u8>, Weak<Tunnel>>;
static TUNNELS: OnceLock<Mutex<Tunnels>> = OnceLock::new();

pub struct EngineInfo {
    pub path: PathBuf,
    pub version: String,
}
/// GUI launches do not necessarily inherit Homebrew's PATH.
fn executable(paths: &StorePaths) -> Result<PathBuf, String> {
    if let Some(path) = WorkspaceStore::open(paths)
        .map_err(|e| e.to_string())?
        .proxy_engine_path()
        .map_err(|e| e.to_string())?
    {
        return Path::new(&path)
            .is_file()
            .then(|| PathBuf::from(path))
            .ok_or_else(|| "指定的 sing-box 程序不存在".into());
    }
    let mut candidates = vec![
        PathBuf::from("/opt/homebrew/bin/sing-box"),
        PathBuf::from("/usr/local/bin/sing-box"),
    ];
    if let Some(paths) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&paths).map(|p| {
            p.join(if cfg!(windows) {
                "sing-box.exe"
            } else {
                "sing-box"
            })
        }));
    }
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| "未找到 sing-box，请先安装或指定程序路径".into())
}
pub fn info(paths: &StorePaths) -> Result<EngineInfo, String> {
    let path = executable(paths)?;
    let mut child = Command::new(&path)
        .arg("version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "无法执行 sing-box，请检查程序路径与权限")?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return Err("无法读取 sing-box 版本".into()),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("读取 sing-box 版本超时".into());
            }
        }
    }
    let mut output = String::new();
    child
        .stdout
        .take()
        .ok_or("无法读取 sing-box 版本")?
        .take(4096)
        .read_to_string(&mut output)
        .map_err(|_| "无法读取 sing-box 版本")?;
    let version = output
        .lines()
        .next()
        .and_then(|s| s.strip_prefix("sing-box version "))
        .filter(|s| !s.is_empty())
        .ok_or("指定的程序不是 sing-box")?
        .to_string();
    Ok(EngineInfo { path, version })
}

pub(crate) struct Tunnel {
    child: Mutex<Child>,
    pub port: u16,
    pub username: String,
    pub password: String,
}
impl Tunnel {
    pub(crate) fn alive(&self) -> bool {
        matches!(self.child.lock().unwrap().try_wait(), Ok(None))
    }
    fn stop(&self) {
        let mut child = self.child.lock().unwrap();
        let _ = child.kill();
        let _ = child.wait();
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
    if let Some(tunnels) = TUNNELS.get() {
        let instances: Vec<_> = tunnels
            .lock()
            .unwrap()
            .values()
            .filter_map(Weak::upgrade)
            .collect();
        for tunnel in instances {
            tunnel.stop();
        }
    }
}
fn outbound(proxy: &SocksProxy, tag: &str) -> Value {
    let mut value = json!({"tag":tag,"server":proxy.host,"server_port":proxy.port,"username":proxy.username,"password":proxy.password});
    match proxy.protocol {
        rv_core::workspace::ProxyProtocol::Socks5 => {
            value["type"] = json!("socks");
            value["version"] = json!("5");
            value["network"] = json!("tcp");
        }
        rv_core::workspace::ProxyProtocol::Http => {
            value["type"] = json!("http");
        }
    }
    value
}
fn configuration(route: &ProxyRoute, port: u16, user: &str, password: &str) -> Value {
    let mut exit = outbound(&route.exit, "exit");
    exit["detour"] = json!("front");
    json!({"log":{"disabled":true},
        "inbounds":[{"type":"socks","tag":"rv","listen":"127.0.0.1","listen_port":port,"users":[{"username":user,"password":password}]}],
        "outbounds":[outbound(route.front.as_ref().expect("resolved chain"),"front"),exit],
        "route":{"final":"exit"}})
}
/// Includes both records and the selected executable, so changing a front never
/// reuses the previous endpoint or its HTTP connection pool.
pub(crate) fn acquire(paths: &StorePaths, route: &ProxyRoute) -> Result<Arc<Tunnel>, String> {
    if STOPPING.load(Ordering::SeqCst) {
        return Err("代理引擎正在退出".into());
    }
    let executable = executable(paths)?;
    let key = Sha256::digest(
        serde_json::to_vec(&(
            std::fs::canonicalize(&paths.database).map_err(|_| "代理数据库不可用")?,
            &executable,
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
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| "无法分配本地代理端口")?;
    let port = listener
        .local_addr()
        .map_err(|_| "无法读取本地代理端口")?
        .port();
    let username = uuid::Uuid::new_v4().simple().to_string();
    let password = uuid::Uuid::new_v4().simple().to_string();
    let config = serde_json::to_vec(&configuration(route, port, &username, &password))
        .map_err(|_| "代理配置无效")?;
    drop(listener);
    let child = Command::new(executable)
        .args(["run", "-c", "stdin", "--disable-color"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "无法启动 sing-box，请检查程序路径与权限")?;
    let tunnel = Arc::new(Tunnel {
        child: Mutex::new(child),
        port,
        username,
        password,
    });
    {
        let input = tunnel
            .child
            .lock()
            .unwrap()
            .stdin
            .take()
            .ok_or("无法传递 sing-box 配置")?;
        // EOF completes sing-box's configuration read. No plaintext config file.
        let mut input = input;
        input
            .write_all(&config)
            .map_err(|_| "无法传递 sing-box 配置")?;
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if !tunnel.alive() {
            return Err("sing-box 启动失败，请检查代理配置及程序版本".into());
        }
        if ready(&tunnel) {
            if STOPPING.load(Ordering::SeqCst) {
                return Err("代理引擎正在退出".into());
            }
            tunnels.insert(key, Arc::downgrade(&tunnel));
            return Ok(tunnel);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err("sing-box 本地入口启动超时".into())
}
fn ready(tunnel: &Tunnel) -> bool {
    let address = SocketAddr::from(([127, 0, 0, 1], tunnel.port));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(100)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
    (|| -> std::io::Result<bool> {
        stream.write_all(&[5, 1, 2])?;
        let mut reply = [0; 2];
        stream.read_exact(&mut reply)?;
        if reply != [5, 2] {
            return Ok(false);
        }
        let mut auth = vec![1, tunnel.username.len() as u8];
        auth.extend_from_slice(tunnel.username.as_bytes());
        auth.push(tunnel.password.len() as u8);
        auth.extend_from_slice(tunnel.password.as_bytes());
        stream.write_all(&auth)?;
        stream.read_exact(&mut reply)?;
        Ok(reply == [1, 0])
    })()
    .unwrap_or(false)
}
#[cfg(test)]
mod tests;
