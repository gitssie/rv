use super::*;
use std::sync::Arc;

fn fixture() -> (tempfile::TempDir, StorePaths, SocksProxy) {
    let dir = tempfile::tempdir().unwrap();
    let paths = StorePaths::in_dir(dir.path().to_owned());
    rv_core::AddressBook::load(paths.clone())
        .unwrap()
        .save()
        .unwrap();
    let proxy = SocksProxy {
        protocol: Default::default(),
        front_proxy_id: None,
        id: uuid::Uuid::new_v4().to_string(),
        name: "fixture".into(),
        host: "127.0.0.1".into(),
        port: 1,
        username: String::new(),
        password: String::new(),
        mode: rv_core::workspace::ProxyMode::Shared,
        ip: Default::default(),
    };
    WorkspaceStore::open(&paths)
        .unwrap()
        .save_proxy(&proxy)
        .unwrap();
    (dir, paths, proxy)
}

#[test]
fn ip_parser_checks_success_family_and_public_address() {
    for body in [
        b"8.8.8.8\n".as_slice(),
        br#"{"ip":"8.8.8.8","success":true}"#,
    ] {
        assert_eq!(parse_ip(body).unwrap(), Ipv4Addr::new(8, 8, 8, 8));
    }
    for body in [
        br#"{"ip":"8.8.8.8","success":false}"#.as_slice(),
        b"127.0.0.1",
        b"192.168.1.1",
        b"::1",
        b"garbage",
        br#"{"ip":"::ffff:8.8.8.8"}"#,
    ] {
        assert!(parse_ip(body).is_err());
    }
    for ip in [
        "100.64.0.1",
        "100.127.255.254",
        "198.18.0.1",
        "198.19.255.1",
        "192.0.2.1",
        "198.51.100.1",
        "203.0.113.1",
        "192.0.0.8",
        "192.88.99.2",
        "240.0.0.1",
    ] {
        assert!(parse_ip(ip.as_bytes()).is_err(), "{ip}");
    }
}

#[test]
fn history_is_atomic_global_persistent_and_survives_proxy_deletion() {
    let (_dir, paths, proxy) = fixture();
    let mut other = proxy.clone();
    other.id = uuid::Uuid::new_v4().to_string();
    WorkspaceStore::open(&paths)
        .unwrap()
        .save_proxy(&other)
        .unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let threads: Vec<_> = [proxy.id.clone(), other.id.clone()]
        .into_iter()
        .map(|id| {
            let paths = paths.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut store = WorkspaceStore::open(&paths).unwrap();
                barrier.wait();
                store
                    .record_proxy_ip(Ipv4Addr::new(8, 8, 8, 8), &id, "task")
                    .unwrap()
            })
        })
        .collect();
    assert_eq!(
        threads
            .into_iter()
            .map(|thread| usize::from(thread.join().unwrap()))
            .sum::<usize>(),
        1
    );
    let mut store = WorkspaceStore::open(&paths).unwrap();
    assert_eq!(store.proxy_ip_history().unwrap().len(), 1);
    store.delete_proxy(&proxy.id).unwrap();
    store.delete_proxy(&other.id).unwrap();
    let rows = store.proxy_ip_history().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].proxy_id, None);
}

struct State {
    current: Ipv4Addr,
    probe_body: Option<String>,
    next: std::collections::VecDeque<Ipv4Addr>,
    changes: usize,
    probes: usize,
    block_probes: bool,
    drop_change_reply: bool,
    passwords: Vec<String>,
}
struct Server {
    proxy_port: u16,
    control_url: String,
    state: Arc<std::sync::Mutex<State>>,
    entered: Arc<tokio::sync::Notify>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
impl Server {
    async fn new(next: &[Ipv4Addr]) -> Self {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let control = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_port = proxy.local_addr().unwrap().port();
        let control_url = format!(
            "http://{}/change?secret=fixture-secret",
            control.local_addr().unwrap()
        );
        let state = Arc::new(std::sync::Mutex::new(State {
            current: Ipv4Addr::new(8, 8, 8, 8),
            probe_body: None,
            next: next.iter().copied().collect(),
            changes: 0,
            probes: 0,
            block_probes: false,
            drop_change_reply: false,
            passwords: Vec::new(),
        }));
        let entered = Arc::new(tokio::sync::Notify::new());
        let mut tasks = Vec::new();
        for (listener, socks) in [(proxy, true), (control, false)] {
            let state = state.clone();
            let entered = entered.clone();
            tasks.push(tokio::spawn(async move {
                let mut sessions=tokio::task::JoinSet::new();
                loop {
                    let (mut socket,_)=listener.accept().await.unwrap();
                    let state=state.clone();let entered=entered.clone();
                    sessions.spawn(async move {
                        if socks {
                            let mut hello=[0;2];socket.read_exact(&mut hello).await.unwrap();assert_eq!(hello[0],5);
                            let mut methods=vec![0;hello[1] as usize];socket.read_exact(&mut methods).await.unwrap();
                            if methods.contains(&2) {
                                socket.write_all(&[5,2]).await.unwrap();
                                assert_eq!(socket.read_u8().await.unwrap(),1);
                                let n=socket.read_u8().await.unwrap();
                                let mut user=vec![0;n as usize];socket.read_exact(&mut user).await.unwrap();
                                let n=socket.read_u8().await.unwrap();
                                let mut password=vec![0;n as usize];socket.read_exact(&mut password).await.unwrap();
                                let password=String::from_utf8(password).unwrap();
                                {
                                    let mut state=state.lock().unwrap();
                                    if state.passwords.last()!=Some(&password) {
                                        if !state.passwords.is_empty() {
                                            state.changes+=1;
                                            if let Some(ip)=state.next.pop_front(){state.current=ip;}
                                        }
                                        state.passwords.push(password);
                                    }
                                }
                                socket.write_all(&[1,0]).await.unwrap();
                            } else { socket.write_all(&[5,0]).await.unwrap(); }
                            let mut head=[0;4];socket.read_exact(&mut head).await.unwrap();assert_eq!(head,[5,1,0,3]);
                            let length=socket.read_u8().await.unwrap();let mut host=vec![0;length as usize];socket.read_exact(&mut host).await.unwrap();
                            assert_eq!(host,b"ip.fixture.invalid");assert_eq!(socket.read_u16().await.unwrap(),80);
                            socket.write_all(&[5,0,0,1,127,0,0,1,0,80]).await.unwrap();
                        }
                        let mut request=Vec::new();let mut buffer=[0;1024];
                        while !request.windows(4).any(|part|part==b"\r\n\r\n") {
                            let n=socket.read(&mut buffer).await.unwrap();if n==0{return;}request.extend_from_slice(&buffer[..n]);assert!(request.len()<8192);
                        }
                        let text=String::from_utf8_lossy(&request);
                        let (body,blocked,drop_reply)={
                            let mut state=state.lock().unwrap();
                            if socks {
                                assert!(text.starts_with("GET /probe "));
                                state.probes+=1;
                                (state.probe_body.clone().unwrap_or_else(|| serde_json::json!({"ip":state.current.to_string(),"success":true}).to_string()),state.block_probes&&state.changes>0,false)
                            } else {
                                assert!(text.starts_with("GET /change?secret=fixture-secret "));
                                state.changes+=1;
                                if let Some(ip)=state.next.pop_front(){state.current=ip;}
                                ("{\"success\":true}".into(),false,state.drop_change_reply)
                            }
                        };
                        if blocked {entered.notify_one();std::future::pending::<()>().await;}
                        if drop_reply{return;}
                        let reply=format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                        let _=socket.write_all(reply.as_bytes()).await;
                    });
                    while sessions.try_join_next().is_some() {}
                }
            }));
        }
        Self {
            proxy_port,
            control_url,
            state,
            entered,
            tasks,
        }
    }
    fn configure(&self, paths: &StorePaths, proxy: &mut SocksProxy) {
        proxy.port = self.proxy_port;
        proxy.ip.change_url = Some(self.control_url.clone());
        proxy.ip.probe_url = "http://ip.fixture.invalid/probe".into();
        WorkspaceStore::open(paths)
            .unwrap()
            .save_proxy(proxy)
            .unwrap();
    }
    fn changes(&self) -> usize {
        self.state.lock().unwrap().changes
    }
}
fn options() -> AcquireOptions {
    AcquireOptions {
        max_changes: 3,
        settle_timeout: Duration::from_secs(1),
        poll_interval: Duration::from_millis(2),
        preparation_timeout: Duration::from_secs(5),
    }
}

#[tokio::test]
async fn managed_reentry_prepares_once_and_shares_the_existing_business_gate() {
    let (_dir, paths, mut proxy) = fixture();
    let server = Server::new(&[Ipv4Addr::new(1, 1, 1, 1)]).await;
    server.configure(&paths, &mut proxy);
    let manager = ProxyManager::new(paths.clone());
    let (a, b) = tokio::join!(
        manager.acquire_with_options(&proxy.id, "same", IpPolicy::NeverSeen, options()),
        manager.acquire_with_options(&proxy.id, "same", IpPolicy::NeverSeen, options())
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!(a.ip(), b.ip());
    assert_eq!(server.changes(), 1);
    let inner = a.reenter(IpPolicy::NeverSeen).await.unwrap();
    assert_eq!(server.changes(), 1);
    let gate = network::proxy_gate(&paths, &proxy.id).unwrap();
    drop(a);
    drop(b);
    assert!(
        tokio::time::timeout(
            Duration::from_millis(20),
            gate.acquire("another-app", rv_core::workspace::ProxyMode::Shared)
        )
        .await
        .is_err()
    );
    drop(inner);
    let _legacy = tokio::time::timeout(
        Duration::from_secs(1),
        gate.acquire("another-app", rv_core::workspace::ProxyMode::Shared),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn duplicate_ips_are_rotated_again_and_history_is_not_scoped_to_the_exit() {
    let (_dir, paths, mut proxy) = fixture();
    let mut other = proxy.clone();
    other.id = uuid::Uuid::new_v4().to_string();
    let mut store = WorkspaceStore::open(&paths).unwrap();
    store.save_proxy(&other).unwrap();
    store
        .record_proxy_ip(Ipv4Addr::new(1, 1, 1, 1), &other.id, "old-task")
        .unwrap();
    let server = Server::new(&[Ipv4Addr::new(1, 1, 1, 1), Ipv4Addr::new(9, 9, 9, 9)]).await;
    server.configure(&paths, &mut proxy);
    let lease = ProxyManager::new(paths.clone())
        .acquire_with_options(&proxy.id, "new-task", IpPolicy::NeverSeen, options())
        .await
        .unwrap();
    assert_eq!(lease.ip(), Ipv4Addr::new(9, 9, 9, 9));
    assert_eq!(server.changes(), 2);
    drop(lease);
    assert_eq!(
        WorkspaceStore::open(&paths)
            .unwrap()
            .proxy_ip_history()
            .unwrap()
            .len(),
        3
    );
}

#[tokio::test]
async fn stricter_nested_policy_fails_without_changing_ip() {
    let (_dir, paths, mut proxy) = fixture();
    let server = Server::new(&[]).await;
    server.configure(&paths, &mut proxy);
    let manager = ProxyManager::new(paths.clone());
    let lease = manager
        .acquire_with_options(&proxy.id, "owner", IpPolicy::Reuse, options())
        .await
        .unwrap();
    assert_eq!(
        lease.reenter(IpPolicy::NeverSeen).await.unwrap_err(),
        ProxyError::ReentryConflict
    );
    assert_eq!(server.changes(), 0);
    drop(lease);
}

#[tokio::test]
async fn interrupted_preparation_is_not_replayed_by_a_waiting_reentrant_call() {
    let (_dir, paths, mut proxy) = fixture();
    let server = Server::new(&[Ipv4Addr::new(1, 1, 1, 1)]).await;
    server.configure(&paths, &mut proxy);
    server.state.lock().unwrap().block_probes = true;
    let manager = ProxyManager::new(paths.clone());
    let first = {
        let manager = manager.clone();
        let id = proxy.id.clone();
        tokio::spawn(async move {
            manager
                .acquire_with_options(&id, "owner", IpPolicy::NeverSeen, options())
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(2), server.entered.notified())
        .await
        .unwrap();
    let inner = manager.acquire_with_options(&proxy.id, "owner", IpPolicy::Reuse, options());
    tokio::pin!(inner);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), inner.as_mut())
            .await
            .is_err()
    );
    first.abort();
    let _ = first.await;
    assert_eq!(inner.await.unwrap_err(), ProxyError::PreparationInterrupted);
    assert_eq!(server.changes(), 1);
    let gate = network::proxy_gate(&paths, &proxy.id).unwrap();
    let _next = tokio::time::timeout(
        Duration::from_secs(1),
        gate.acquire("next", rv_core::workspace::ProxyMode::Shared),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn unknown_change_reply_is_reconciled_without_replaying_the_trigger() {
    let (_dir, paths, mut proxy) = fixture();
    let server = Server::new(&[Ipv4Addr::new(1, 1, 1, 1)]).await;
    server.configure(&paths, &mut proxy);
    server.state.lock().unwrap().drop_change_reply = true;
    let lease = ProxyManager::new(paths)
        .acquire_with_options(&proxy.id, "owner", IpPolicy::NeverSeen, options())
        .await
        .unwrap();
    assert_eq!(lease.ip(), Ipv4Addr::new(1, 1, 1, 1));
    assert_eq!(server.changes(), 1);
}

#[tokio::test]
async fn unchanged_exit_stops_and_releases_instead_of_retrying_the_trigger() {
    let (_dir, paths, mut proxy) = fixture();
    let server = Server::new(&[]).await;
    server.configure(&paths, &mut proxy);
    let mut opts = options();
    opts.settle_timeout = Duration::from_millis(40);
    assert_eq!(
        ProxyManager::new(paths.clone())
            .acquire_with_options(&proxy.id, "owner", IpPolicy::Rotate, opts)
            .await
            .unwrap_err(),
        ProxyError::RotationUnconfirmed
    );
    assert_eq!(server.changes(), 1);
    let gate = network::proxy_gate(&paths, &proxy.id).unwrap();
    let _next = tokio::time::timeout(
        Duration::from_secs(1),
        gate.acquire("next", rv_core::workspace::ProxyMode::Shared),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn legacy_app_requests_cannot_bypass_a_preparing_task_with_the_same_owner_text() {
    let (_dir, paths, mut proxy) = fixture();
    let server = Server::new(&[Ipv4Addr::new(1, 1, 1, 1)]).await;
    server.configure(&paths, &mut proxy);
    server.state.lock().unwrap().block_probes = true;
    let manager = ProxyManager::new(paths.clone());
    let first = {
        let id = proxy.id.clone();
        tokio::spawn(async move {
            manager
                .acquire_with_options(&id, "app-id", IpPolicy::NeverSeen, options())
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(2), server.entered.notified())
        .await
        .unwrap();
    let gate = network::proxy_gate(&paths, &proxy.id).unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(20),
            gate.acquire("app-id", rv_core::workspace::ProxyMode::Shared)
        )
        .await
        .is_err()
    );
    first.abort();
    let _ = first.await;
    let _legacy = tokio::time::timeout(
        Duration::from_secs(1),
        gate.acquire("app-id", rv_core::workspace::ProxyMode::Shared),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn accidentally_using_an_existing_app_owner_cannot_self_deadlock_on_upgrade() {
    let (_dir, paths, mut proxy) = fixture();
    let server = Server::new(&[]).await;
    server.configure(&paths, &mut proxy);
    let gate = network::proxy_gate(&paths, &proxy.id).unwrap();
    let _outer = gate
        .acquire("existing-app", rv_core::workspace::ProxyMode::Shared)
        .await;
    let manager = ProxyManager::new(paths);
    let result = tokio::time::timeout(
        Duration::from_millis(100),
        manager.acquire_with_options(&proxy.id, "existing-app", IpPolicy::Rotate, options()),
    )
    .await
    .unwrap();
    assert_eq!(result.unwrap_err(), ProxyError::ReentryConflict);
    assert_eq!(server.changes(), 0);
}

#[tokio::test]
async fn exhausting_seen_addresses_releases_the_gate_but_keeps_history() {
    let (_dir, paths, mut proxy) = fixture();
    let server = Server::new(&[Ipv4Addr::new(1, 1, 1, 1), Ipv4Addr::new(9, 9, 9, 9)]).await;
    server.configure(&paths, &mut proxy);
    let mut store = WorkspaceStore::open(&paths).unwrap();
    for ip in [Ipv4Addr::new(1, 1, 1, 1), Ipv4Addr::new(9, 9, 9, 9)] {
        store.record_proxy_ip(ip, &proxy.id, "previous").unwrap();
    }
    let mut opts = options();
    opts.max_changes = 2;
    assert_eq!(
        ProxyManager::new(paths.clone())
            .acquire_with_options(&proxy.id, "owner", IpPolicy::NeverSeen, opts)
            .await
            .unwrap_err(),
        ProxyError::HistoryExhausted
    );
    assert_eq!(server.changes(), 2);
    assert_eq!(store.proxy_ip_history().unwrap().len(), 3);
    let gate = network::proxy_gate(&paths, &proxy.id).unwrap();
    let _next = tokio::time::timeout(
        Duration::from_secs(1),
        gate.acquire("next", rv_core::workspace::ProxyMode::Shared),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn two_different_exits_cannot_both_claim_the_same_new_ip() {
    let (_dir, paths, mut first) = fixture();
    let mut second = first.clone();
    second.id = uuid::Uuid::new_v4().to_string();
    let a = Server::new(&[Ipv4Addr::new(1, 1, 1, 1)]).await;
    let b = Server::new(&[Ipv4Addr::new(1, 1, 1, 1)]).await;
    a.configure(&paths, &mut first);
    b.configure(&paths, &mut second);
    let manager = ProxyManager::new(paths.clone());
    let mut opts = options();
    opts.max_changes = 1;
    let (a, b) = tokio::join!(
        manager.acquire_with_options(&first.id, "a", IpPolicy::NeverSeen, opts),
        manager.acquire_with_options(&second.id, "b", IpPolicy::NeverSeen, opts)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert!(
        a.as_ref().err() == Some(&ProxyError::HistoryExhausted)
            || b.as_ref().err() == Some(&ProxyError::HistoryExhausted)
    );
    assert_eq!(
        WorkspaceStore::open(&paths)
            .unwrap()
            .proxy_ip_history()
            .unwrap()
            .iter()
            .filter(|entry| entry.ip == Ipv4Addr::new(1, 1, 1, 1))
            .count(),
        1
    );
}

#[tokio::test]
async fn manual_inspection_uses_socks_without_rotation_or_history() {
    let (_dir, paths, mut proxy) = fixture();
    let server = Server::new(&[]).await;
    server.configure(&paths, &mut proxy);
    let info = inspect(&paths, &proxy).await.unwrap();
    assert_eq!(info, "IP：8.8.8.8");
    assert_eq!(server.changes(), 0);
    assert_eq!(server.state.lock().unwrap().probes, 1);
    assert!(
        WorkspaceStore::open(&paths)
            .unwrap()
            .record_proxy_ip(Ipv4Addr::new(8, 8, 8, 8), &proxy.id, "after-inspection")
            .unwrap()
    );
    for body in [
        "192.168.1.1",
        "2001:db8::1",
        r#"{"success":false,"message":"provider explanation"}"#,
        r#"{"query":"custom-format","location":"Paris"}"#,
        "arbitrary response text",
    ] {
        server.state.lock().unwrap().probe_body = Some(body.to_owned());
        assert_eq!(inspect(&paths, &proxy).await.unwrap(), body);
    }
    assert_eq!(server.changes(), 0);
    proxy.port = 1;
    assert_eq!(
        inspect(&paths, &proxy).await.unwrap_err(),
        ProxyError::ProbeFailed
    );
}

#[test]
fn manual_observation_formats_json_without_validating_ip() {
    let body = r#"{"ip":"2a01:cb01:1072:aaa4:461a:5e81:3513:7b39","success":true,"type":"IPv6","country":"France","region":"Pays de la Loire","city":"La Suze-sur-Sarthe"}"#;
    assert_eq!(
        format_observation(body),
        "IP：2a01:cb01:1072:aaa4:461a:5e81:3513:7b39　地区：France · Pays de la Loire · La Suze-sur-Sarthe"
    );
    for ip in ["192.168.1.1", "::1", "provider-value"] {
        let body = serde_json::json!({"ip":ip,"success":false,"country":"France"}).to_string();
        assert_eq!(format_observation(&body), format!("IP：{ip}　地区：France"));
    }
    for body in ["plain response", r#"{"message":"provider error"}"#] {
        assert_eq!(format_observation(body), body);
    }
}

#[tokio::test]
async fn password_session_rotation_uses_new_socks_credentials_and_persists_them() {
    let (_dir, paths, mut proxy) = fixture();
    let server = Server::new(&[Ipv4Addr::new(1, 1, 1, 1)]).await;
    server.configure(&paths, &mut proxy);
    proxy.username = "fixture-user".into();
    proxy.password = "secret_country-fr_lifetime-5_session-a7ozznml".into();
    proxy.ip.change_url = None;
    proxy.ip.rotation = rv_core::workspace::ProxyRotation::PasswordSession {
        marker: "_session-".into(),
    };
    WorkspaceStore::open(&paths)
        .unwrap()
        .save_proxy(&proxy)
        .unwrap();
    let lease = ProxyManager::new(paths.clone())
        .acquire_with_options(&proxy.id, "password-task", IpPolicy::Rotate, options())
        .await
        .unwrap();
    assert_eq!(lease.ip(), Ipv4Addr::new(1, 1, 1, 1));
    let saved = load_proxy(&paths, &proxy.id).unwrap();
    assert_ne!(saved.password, proxy.password);
    assert!(
        saved
            .password
            .starts_with("secret_country-fr_lifetime-5_session-")
    );
    assert_eq!(saved.password.len(), proxy.password.len());
    let state = server.state.lock().unwrap();
    assert_eq!(state.passwords, vec![proxy.password, saved.password]);
    assert_eq!(state.changes, 1);
}

#[test]
fn password_rotation_does_not_overwrite_an_edited_proxy() {
    let (_dir, paths, mut proxy) = fixture();
    proxy.password = "secret_session-12345678".into();
    proxy.ip.rotation = rv_core::workspace::ProxyRotation::PasswordSession {
        marker: "_session-".into(),
    };
    let mut store = WorkspaceStore::open(&paths).unwrap();
    store.save_proxy(&proxy).unwrap();
    let mut edited = proxy.clone();
    edited.host = "new.fixture.invalid".into();
    store.save_proxy(&edited).unwrap();
    assert!(store.rotate_proxy_password_session(&proxy, &None).is_err());
    let loaded = load_proxy(&paths, &proxy.id).unwrap();
    assert_eq!(loaded.host, edited.host);
    assert_eq!(loaded.password, edited.password);
}

fn business_apps(paths: &StorePaths, proxy: &SocksProxy) -> [AppId; 2] {
    let mut book = rv_core::AddressBook::load(paths.clone()).unwrap();
    let phone = rv_core::Connection::new("fixture", "127.0.0.1", 5900);
    book.upsert(phone.clone());
    book.save().unwrap();
    let ids = [
        AppId::from_idfv("A148FA97-37B0-4BA1-9DBB-AF619062F420").unwrap(),
        AppId::from_idfv("B148FA97-37B0-4BA1-9DBB-AF619062F420").unwrap(),
    ];
    let mut store = WorkspaceStore::open(paths).unwrap();
    for (i, id) in ids.iter().enumerate() {
        store.sync_backups(phone.id,"lt.manodrabuziai.fr",&serde_json::json!({
            "bundle_id":"lt.manodrabuziai.fr","backups":[{"kind":"snapshot","backup_id":format!("snapshot:{i}"),"idfv":id.as_str()}]
        })).unwrap();
        store.bind_proxy(id, Some(&proxy.id)).unwrap();
    }
    ids
}
async fn business_probe(paths: &StorePaths, app: &AppId) -> Result<String, String> {
    let direct = reqwest::Client::new();
    let (route, _lease) = network::acquire_route(paths, app, &direct, true).await?;
    route
        .client
        .get("http://ip.fixture.invalid/probe")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())
}
#[tokio::test]
async fn exclusive_business_rotates_only_when_virtual_app_changes() {
    let (_dir, paths, mut proxy) = fixture();
    let server = Server::new(&[Ipv4Addr::new(1, 1, 1, 1), Ipv4Addr::new(9, 9, 9, 9)]).await;
    server.configure(&paths, &mut proxy);
    proxy.mode = rv_core::workspace::ProxyMode::Exclusive;
    proxy.username = "fixture".into();
    proxy.password = "secret_session-12345678".into();
    proxy.ip.change_url = None;
    proxy.ip.rotation = rv_core::workspace::ProxyRotation::PasswordSession {
        marker: "_session-".into(),
    };
    WorkspaceStore::open(&paths)
        .unwrap()
        .save_proxy(&proxy)
        .unwrap();
    let [a, b] = business_apps(&paths, &proxy);
    assert!(
        business_probe(&paths, &a)
            .await
            .unwrap()
            .contains("8.8.8.8")
    );
    assert!(
        business_probe(&paths, &a)
            .await
            .unwrap()
            .contains("8.8.8.8")
    );
    assert_eq!(server.changes(), 0);
    // Concurrent callers for B join the same ownership transition.
    let (first, second) = tokio::join!(business_probe(&paths, &b), business_probe(&paths, &b));
    assert!(first.unwrap().contains("1.1.1.1"));
    assert!(second.unwrap().contains("1.1.1.1"));
    assert_eq!(server.changes(), 1);
    assert!(
        business_probe(&paths, &b)
            .await
            .unwrap()
            .contains("1.1.1.1")
    );
    assert_eq!(server.changes(), 1);
    assert!(
        business_probe(&paths, &a)
            .await
            .unwrap()
            .contains("9.9.9.9")
    );
    assert_eq!(server.changes(), 2);
}
#[tokio::test]
async fn shared_business_does_not_rotate_when_virtual_app_changes() {
    let (_dir, paths, mut proxy) = fixture();
    let server = Server::new(&[Ipv4Addr::new(1, 1, 1, 1)]).await;
    server.configure(&paths, &mut proxy);
    let [a, b] = business_apps(&paths, &proxy);
    business_probe(&paths, &a).await.unwrap();
    business_probe(&paths, &b).await.unwrap();
    business_probe(&paths, &a).await.unwrap();
    assert_eq!(server.changes(), 0);
}
#[tokio::test]
async fn exclusive_switch_failure_does_not_send_the_next_apps_request() {
    let (_dir, paths, mut proxy) = fixture();
    let server = Server::new(&[]).await;
    server.configure(&paths, &mut proxy);
    proxy.mode = rv_core::workspace::ProxyMode::Exclusive;
    proxy.ip.change_url = None;
    WorkspaceStore::open(&paths)
        .unwrap()
        .save_proxy(&proxy)
        .unwrap();
    let [a, b] = business_apps(&paths, &proxy);
    business_probe(&paths, &a).await.unwrap();
    let before = server.state.lock().unwrap().probes;
    assert!(
        business_probe(&paths, &b)
            .await
            .unwrap_err()
            .contains("换 IP")
    );
    assert_eq!(server.state.lock().unwrap().probes, before);
    // A remains the owner and can continue without a provider rotation URL.
    business_probe(&paths, &a).await.unwrap();
    assert_eq!(server.changes(), 0);
}

#[tokio::test]
async fn exclusive_http_business_switches_before_the_next_apps_request() {
    let (_dir, paths, mut proxy) = fixture();
    let server = Server::new(&[Ipv4Addr::new(1, 1, 1, 1)]).await;
    server.configure(&paths, &mut proxy);
    proxy.mode = rv_core::workspace::ProxyMode::Exclusive;
    WorkspaceStore::open(&paths)
        .unwrap()
        .save_proxy(&proxy)
        .unwrap();
    let [a, b] = business_apps(&paths, &proxy);
    business_probe(&paths, &a).await.unwrap();
    assert!(
        business_probe(&paths, &b)
            .await
            .unwrap()
            .contains("1.1.1.1")
    );
    assert!(
        business_probe(&paths, &b)
            .await
            .unwrap()
            .contains("1.1.1.1")
    );
    assert_eq!(server.changes(), 1);
}
