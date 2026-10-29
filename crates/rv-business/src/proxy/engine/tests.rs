use super::*;
use rv_core::workspace::{ProxyMode, ProxyProtocol};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
fn fixture() -> (tempfile::TempDir, StorePaths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = StorePaths::in_dir(dir.path().into());
    rv_core::AddressBook::load(paths.clone()).unwrap();
    (dir, paths)
}
fn proxy(name: &str, port: u16) -> SocksProxy {
    SocksProxy {
        protocol: Default::default(),
        id: uuid::Uuid::new_v4().to_string(),
        name: name.into(),
        host: "127.0.0.1".into(),
        port,
        username: format!("{name}-user"),
        password: format!("{name}-pass"),
        mode: ProxyMode::Shared,
        front_proxy_id: None,
        ip: Default::default(),
    }
}
async fn handshake(
    socket: &mut tokio::net::TcpStream,
    user: &str,
    password: &str,
) -> (String, u16) {
    let mut greeting = [0; 2];
    socket.read_exact(&mut greeting).await.unwrap();
    assert_eq!(greeting[0], 5);
    let mut methods = vec![0; greeting[1] as usize];
    socket.read_exact(&mut methods).await.unwrap();
    assert!(methods.contains(&2));
    socket.write_all(&[5, 2]).await.unwrap();
    assert_eq!(socket.read_u8().await.unwrap(), 1);
    let mut name = vec![0; socket.read_u8().await.unwrap() as usize];
    socket.read_exact(&mut name).await.unwrap();
    assert_eq!(name, user.as_bytes());
    let mut pass = vec![0; socket.read_u8().await.unwrap() as usize];
    socket.read_exact(&mut pass).await.unwrap();
    assert_eq!(pass, password.as_bytes());
    socket.write_all(&[1, 0]).await.unwrap();
    let mut request = [0; 4];
    socket.read_exact(&mut request).await.unwrap();
    assert_eq!(request, [5, 1, 0, 3]);
    let mut host = vec![0; socket.read_u8().await.unwrap() as usize];
    socket.read_exact(&mut host).await.unwrap();
    let port = socket.read_u16().await.unwrap();
    socket
        .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
        .await
        .unwrap();
    (String::from_utf8(host).unwrap(), port)
}
async fn protocol_handshake(
    socket: &mut tokio::net::TcpStream,
    protocol: ProxyProtocol,
    user: &str,
    password: &str,
) -> (String, u16) {
    if protocol == ProxyProtocol::Socks5 {
        return handshake(socket, user, password).await;
    }
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        bytes.push(socket.read_u8().await.unwrap());
        assert!(bytes.len() < 4096);
    }
    let header = String::from_utf8(bytes).unwrap();
    let target = header
        .lines()
        .next()
        .unwrap()
        .strip_prefix("CONNECT ")
        .unwrap()
        .strip_suffix(" HTTP/1.1")
        .unwrap();
    let auth = if user == "front-user" {
        "ZnJvbnQtdXNlcjpmcm9udC1wYXNz"
    } else {
        "ZXhpdC11c2VyOmV4aXQtcGFzcw=="
    };
    assert!(
        header
            .lines()
            .any(|line| line.eq_ignore_ascii_case(&format!("Proxy-Authorization: Basic {auth}")))
    );
    let (host, port) = target.rsplit_once(':').unwrap();
    let target = (host.to_owned(), port.parse().unwrap());
    socket
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await
        .unwrap();
    target
}
#[tokio::test]
async fn installed_sing_box_chains_existing_records_with_both_auths_and_remote_dns() {
    for (front, exit) in [
        (ProxyProtocol::Socks5, ProxyProtocol::Socks5),
        (ProxyProtocol::Socks5, ProxyProtocol::Http),
        (ProxyProtocol::Http, ProxyProtocol::Socks5),
        (ProxyProtocol::Http, ProxyProtocol::Http),
    ] {
        chain(front, exit).await;
    }
}
async fn chain(front_protocol: ProxyProtocol, exit_protocol: ProxyProtocol) {
    let (_dir, paths) = fixture();
    if info(&paths).is_err() {
        eprintln!("sing-box not installed; external-engine integration skipped");
        return;
    }
    let front_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let exit_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut front = proxy("front", front_listener.local_addr().unwrap().port());
    front.protocol = front_protocol;
    let mut exit = proxy("exit", exit_listener.local_addr().unwrap().port());
    exit.protocol = exit_protocol;
    exit.host = "rv-exit.invalid".into();
    exit.front_proxy_id = Some(front.id.clone());
    let mut store = WorkspaceStore::open(&paths).unwrap();
    store.save_proxy(&front).unwrap();
    store.save_proxy(&exit).unwrap();
    let exit_address = exit_listener.local_addr().unwrap();
    let first = tokio::spawn(async move {
        let (mut incoming, _) = front_listener.accept().await.unwrap();
        assert_eq!(
            protocol_handshake(&mut incoming, front_protocol, "front-user", "front-pass").await,
            ("rv-exit.invalid".into(), exit_address.port())
        );
        let mut outgoing = tokio::net::TcpStream::connect(exit_address).await.unwrap();
        let _ = tokio::io::copy_bidirectional(&mut incoming, &mut outgoing).await;
    });
    let last = tokio::spawn(async move {
        let (mut socket, _) = exit_listener.accept().await.unwrap();
        assert_eq!(
            protocol_handshake(&mut socket, exit_protocol, "exit-user", "exit-pass").await,
            ("rv-target.invalid".into(), 80)
        );
        let mut bytes = [0; 4096];
        let size = socket.read(&mut bytes).await.unwrap();
        assert!(String::from_utf8_lossy(&bytes[..size]).starts_with("GET /chain "));
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nchained")
            .await
            .unwrap();
    });
    let client = crate::network::probe_client(&paths, &exit).unwrap();
    let response = tokio::time::timeout(
        Duration::from_secs(5),
        client.get("http://rv-target.invalid/chain").send(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.text().await.unwrap(), "chained");
    tokio::time::timeout(Duration::from_secs(5), last)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), first)
        .await
        .unwrap()
        .unwrap();
    // Both clients share one immutable sing-box instance for the same route.
    let resolved = store.resolve_proxy(&exit).unwrap();
    let one = acquire(&paths, &resolved).unwrap();
    let two = acquire(&paths, &resolved).unwrap();
    assert!(Arc::ptr_eq(&one, &two));
    drop(client);
    drop(two);
    assert!(one.alive());
    one.stop();
    assert!(!one.alive());
}
#[tokio::test]
async fn failed_front_cannot_bypass_chain_to_origin() {
    let (_dir, paths) = fixture();
    if info(&paths).is_err() {
        return;
    }
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let front = proxy("front", port);
    let mut exit = proxy("exit", 1080);
    exit.front_proxy_id = Some(front.id.clone());
    let mut store = WorkspaceStore::open(&paths).unwrap();
    store.save_proxy(&front).unwrap();
    store.save_proxy(&exit).unwrap();
    let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = crate::network::probe_client(&paths, &exit).unwrap();
    assert!(
        client
            .get(format!("http://{}/", origin.local_addr().unwrap()))
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(40), origin.accept())
            .await
            .is_err()
    );
}
#[test]
fn missing_engine_has_actionable_error_without_starting_a_route() {
    let (_dir, paths) = fixture();
    WorkspaceStore::open(&paths)
        .unwrap()
        .save_proxy_engine_path(Some("/rv-fixture-missing/sing-box"))
        .unwrap();
    assert!(info(&paths).err().unwrap().contains("不存在"));
}
