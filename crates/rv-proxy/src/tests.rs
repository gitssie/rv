use super::*;
use std::{
    collections::HashSet,
    future::Future,
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::io::AsyncRead;

const LOCAL_SECRET: &str = "fixture-local-secret";
const PROVIDER_USER: &str = "fixture-provider";
const A: &str = "fixture_session-A";
const B: &str = "fixture_session-B";
const A2: &str = "fixture_session-A2";

struct Server {
    addr: SocketAddr,
    task: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn server<F, Fut>(handler: F) -> Server
where
    F: Fn(TcpStream) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut jobs = JoinSet::new();
        loop {
            tokio::select! {
                result = listener.accept() => {
                    let Ok((socket, _)) = result else { break; };
                    jobs.spawn(handler(socket));
                }
                _ = jobs.join_next(), if !jobs.is_empty() => {}
            }
        }
    });
    Server { addr, task }
}
#[derive(Clone, Debug)]
struct Event {
    hop: &'static str,
    username: String,
    password: String,
    target: String,
}
type Events = Arc<Mutex<Vec<Event>>>;
async fn head<R: AsyncRead + Unpin>(socket: &mut R) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    while bytes.len() < 16384 {
        bytes.push(socket.read_u8().await?);
        if bytes.ends_with(b"\r\n\r\n") {
            return Ok(bytes);
        }
    }
    Err(io::Error::other("fixture headers too large"))
}
fn authority(host: &str, port: u16) -> String {
    format!("{host}:{port}")
}
async fn socks_server(hop: &'static str, next: SocketAddr, events: Events) -> Server {
    server(move |mut socket| {
        let events = events.clone();
        async move {
            let _ = async {
                assert_eq!(socket.read_u8().await?, 5);
                let count = socket.read_u8().await?;
                let mut methods = vec![0; usize::from(count)];
                socket.read_exact(&mut methods).await?;
                assert!(methods.contains(&2));
                socket.write_all(&[5, 2]).await?;
                assert_eq!(socket.read_u8().await?, 1);
                let length = socket.read_u8().await?;
                let mut username = vec![0; usize::from(length)];
                socket.read_exact(&mut username).await?;
                let length = socket.read_u8().await?;
                let mut password = vec![0; usize::from(length)];
                socket.read_exact(&mut password).await?;
                let username = String::from_utf8(username).unwrap();
                let password = String::from_utf8(password).unwrap();
                let accepted = if hop == "front" {
                    username == "front" && password == "fixture-front"
                } else {
                    username == PROVIDER_USER && [A, B, A2].contains(&password.as_str())
                };
                if !accepted {
                    events.lock().unwrap().push(Event {
                        hop,
                        username,
                        password,
                        target: String::new(),
                    });
                    socket.write_all(&[1, 1]).await?;
                    return Ok::<(), io::Error>(());
                }
                socket.write_all(&[1, 0]).await?;
                let mut prefix = [0; 4];
                socket.read_exact(&mut prefix).await?;
                assert_eq!(&prefix[..3], &[5, 1, 0]);
                let host = match prefix[3] {
                    3 => {
                        let length = socket.read_u8().await?;
                        let mut bytes = vec![0; usize::from(length)];
                        socket.read_exact(&mut bytes).await?;
                        String::from_utf8(bytes).unwrap()
                    }
                    1 => {
                        let mut bytes = [0; 4];
                        socket.read_exact(&mut bytes).await?;
                        std::net::Ipv4Addr::from(bytes).to_string()
                    }
                    4 => {
                        let mut bytes = [0; 16];
                        socket.read_exact(&mut bytes).await?;
                        std::net::Ipv6Addr::from(bytes).to_string()
                    }
                    _ => panic!("invalid SOCKS address"),
                };
                let port = socket.read_u16().await?;
                events.lock().unwrap().push(Event {
                    hop,
                    username,
                    password,
                    target: authority(&host, port),
                });
                assert_eq!(port, next.port());
                if hop == "front" {
                    assert_eq!(
                        host, "exit.invalid",
                        "exit DNS must be delegated to the front"
                    );
                } else {
                    assert!(host == "origin.invalid" || host == "localhost");
                }
                let mut upstream = TcpStream::connect(next).await?;
                socket.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).await?;
                tokio::io::copy_bidirectional(&mut socket, &mut upstream).await?;
                Ok(())
            }
            .await;
        }
    })
    .await
}
async fn http_exit(origin: SocketAddr, events: Events) -> Server {
    server(move |socket| {
        let events = events.clone();
        async move {
            let _ = async {
                let mut socket = BufReader::new(socket);
                let bytes = head(&mut socket).await?;
                let mut headers = [httparse::EMPTY_HEADER; 32];
                let mut request = httparse::Request::new(&mut headers);
                request.parse(&bytes).unwrap();
                assert_eq!(request.method, Some("CONNECT"));
                let auth = request.headers.iter().find(|h| h.name.eq_ignore_ascii_case("proxy-authorization")).unwrap();
                let decoded = STANDARD.decode(std::str::from_utf8(auth.value).unwrap().strip_prefix("Basic ").unwrap()).unwrap();
                let text = String::from_utf8(decoded).unwrap();
                let (username, password) = text.split_once(':').unwrap();
                let target = request.path.unwrap().parse::<http::uri::Authority>().unwrap();
                events.lock().unwrap().push(Event { hop: "http-exit", username: username.into(), password: password.into(), target: target.to_string() });
                if username != PROVIDER_USER || ![A, B, A2].contains(&password) {
                    socket.write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n").await?;
                    return Ok::<(), io::Error>(());
                }
                assert!(target.host() == "origin.invalid" || target.host() == "localhost");
                assert_eq!(target.port_u16(), Some(origin.port()));
                let mut upstream = TcpStream::connect(origin).await?;
                socket.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await?;
                tokio::io::copy_bidirectional(&mut socket, &mut upstream).await?;
                Ok(())
            }.await;
        }
    }).await
}

struct Fixture {
    sessions: Sessions,
    gateway: Gateway,
    front: Server,
    _exit: Server,
    _origin: Server,
    events: Events,
    origin_hits: Arc<AtomicUsize>,
    protocol: ExitProtocol,
}
impl Fixture {
    async fn new(protocol: ExitProtocol) -> Self {
        let hits = Arc::new(AtomicUsize::new(0));
        let observed = hits.clone();
        let origin = server(move |mut socket| {
            let hits = observed.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0; 1024];
                loop {
                    match socket.read(&mut buf).await {
                        Ok(0) => {
                            let _ = socket.write_all(b"tail").await;
                            break;
                        }
                        Ok(n) => {
                            if socket.write_all(&buf[..n]).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }
        })
        .await;
        Self::with_origin(protocol, origin, hits).await
    }
    async fn with_origin(protocol: ExitProtocol, origin: Server, hits: Arc<AtomicUsize>) -> Self {
        let events = Events::default();
        let exit = match protocol {
            ExitProtocol::Http => http_exit(origin.addr, events.clone()).await,
            ExitProtocol::Socks5 => socks_server("socks-exit", origin.addr, events.clone()).await,
        };
        let front = socks_server("front", exit.addr, events.clone()).await;
        let sessions = Sessions::new(LOCAL_SECRET.into());
        let gateway = Gateway::start(sessions.clone()).await.unwrap();
        Self {
            sessions,
            gateway,
            front,
            _exit: exit,
            _origin: origin,
            events,
            origin_hits: hits,
            protocol,
        }
    }
    fn route(&self, password: &str, bad_front: bool) -> Arc<dyn Proxy> {
        chain(
            Endpoint {
                host: "127.0.0.1".into(),
                port: self.front.addr.port(),
                username: "front".into(),
                password: if bad_front { "wrong" } else { "fixture-front" }.into(),
            },
            Endpoint {
                host: "exit.invalid".into(),
                port: self._exit.addr.port(),
                username: PROVIDER_USER.into(),
                password: password.into(),
            },
            self.protocol,
        )
    }
    fn insert(&self, key: &str, password: &str) {
        self.sessions
            .insert(key.into(), self.route(password, false));
    }
    async fn connect(&self, key: &str) -> BufReader<TcpStream> {
        let (socket, status) = connect(
            self.gateway.address,
            key,
            "origin.invalid",
            self._origin.addr.port(),
            b"",
        )
        .await;
        assert_eq!(status, 200);
        socket
    }
}
async fn connect(
    addr: SocketAddr,
    key: &str,
    host: &str,
    port: u16,
    early: &[u8],
) -> (BufReader<TcpStream>, u16) {
    let mut socket = TcpStream::connect(addr).await.unwrap();
    let auth = STANDARD.encode(format!("{key}:{LOCAL_SECRET}"));
    let mut bytes = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\nProxy-Authorization: Basic {auth}\r\n\r\n").into_bytes();
    bytes.extend_from_slice(early);
    socket.write_all(&bytes).await.unwrap();
    let mut socket = BufReader::new(socket);
    let response = tokio::time::timeout(Duration::from_secs(5), head(&mut socket))
        .await
        .unwrap()
        .unwrap();
    let status = std::str::from_utf8(&response)
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    (socket, status)
}
async fn echo(socket: &mut BufReader<TcpStream>, data: &[u8]) {
    socket.write_all(data).await.unwrap();
    let mut result = vec![0; data.len()];
    tokio::time::timeout(Duration::from_secs(2), socket.read_exact(&mut result))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result, data);
}
async fn finish(mut socket: BufReader<TcpStream>) {
    socket.shutdown().await.unwrap();
    let mut tail = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), socket.read_to_end(&mut tail))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        tail, b"tail",
        "half-close must still drain the origin's reply"
    );
}
#[tokio::test]
async fn one_port_two_sessions_same_provider_username_distinct_passwords_and_mandatory_front() {
    for protocol in [ExitProtocol::Http, ExitProtocol::Socks5] {
        let f = Fixture::new(protocol).await;
        f.insert("shop-a", A);
        f.insert("shop-b", B);
        let (mut a, mut b) = tokio::join!(f.connect("shop-a"), f.connect("shop-b"));
        tokio::join!(
            echo(&mut a, b"shop A\x00payload"),
            echo(&mut b, b"shop B\xffpayload")
        );
        let events = f.events.lock().unwrap().clone();
        assert_eq!(events.iter().filter(|e| e.hop == "front").count(), 2);
        let exits: Vec<_> = events.iter().filter(|e| e.hop != "front").collect();
        assert_eq!(exits.len(), 2);
        assert!(exits.iter().all(|e| e.username == PROVIDER_USER));
        assert_eq!(
            exits
                .iter()
                .map(|e| e.password.as_str())
                .collect::<HashSet<_>>(),
            HashSet::from([A, B])
        );
        assert!(
            events
                .iter()
                .filter(|e| e.hop == "front")
                .all(|e| e.target == authority("exit.invalid", f._exit.addr.port()))
        );
        assert!(
            exits
                .iter()
                .all(|e| e.target == authority("origin.invalid", f._origin.addr.port()))
        );
        assert_eq!(f.origin_hits.load(Ordering::SeqCst), 2);
        tokio::join!(finish(a), finish(b));
    }
}
#[tokio::test]
async fn replacement_removal_and_idle_cleanup_keep_existing_tunnels_alive() {
    let f = Fixture::new(ExitProtocol::Http).await;
    f.insert("shop-a", A);
    f.insert("idle-shop", B);
    let mut old = f.connect("shop-a").await;
    assert_eq!(
        f.sessions.reap_idle(Duration::ZERO),
        1,
        "active A must survive idle eviction"
    );
    f.insert("shop-a", A2);
    let mut new = f.connect("shop-a").await;
    f.sessions.remove("shop-a");
    let (_, status) = connect(
        f.gateway.address,
        "shop-a",
        "origin.invalid",
        f._origin.addr.port(),
        b"",
    )
    .await;
    assert_eq!(status, 407);
    assert!(f.sessions.is_empty());
    tokio::join!(echo(&mut old, b"old tunnel"), echo(&mut new, b"new tunnel"));
    let exits: HashSet<_> = f
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| e.hop == "http-exit")
        .map(|e| e.password.clone())
        .collect();
    assert_eq!(exits, HashSet::from([A.to_owned(), A2.to_owned()]));
    tokio::join!(finish(old), finish(new));
    f.insert("finished", A);
    let socket = f.connect("finished").await;
    finish(socket).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if f.sessions.reap_idle(Duration::ZERO) == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn invalid_key_bad_front_and_rejected_exit_never_fall_back_or_replay() {
    let f = Fixture::new(ExitProtocol::Http).await;
    let (_, status) = connect(
        f.gateway.address,
        "unknown",
        "origin.invalid",
        f._origin.addr.port(),
        b"",
    )
    .await;
    assert_eq!(status, 407);
    assert!(f.events.lock().unwrap().is_empty());
    f.sessions.insert("bad-front".into(), f.route(A, true));
    let (_, status) = connect(
        f.gateway.address,
        "bad-front",
        "origin.invalid",
        f._origin.addr.port(),
        b"",
    )
    .await;
    assert_eq!(status, 502);
    assert_eq!(f.events.lock().unwrap().len(), 1);
    assert_eq!(f.origin_hits.load(Ordering::SeqCst), 0);
    f.insert("bad-exit", "rejected");
    let (_, status) = connect(
        f.gateway.address,
        "bad-exit",
        "origin.invalid",
        f._origin.addr.port(),
        b"",
    )
    .await;
    assert_eq!(status, 502);
    let events = f.events.lock().unwrap();
    assert_eq!(events.iter().filter(|e| e.hop == "front").count(), 2);
    assert_eq!(events.iter().filter(|e| e.hop == "http-exit").count(), 1);
    assert_eq!(f.origin_hits.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn buffered_bytes_after_connect_headers_are_not_lost() {
    let f = Fixture::new(ExitProtocol::Http).await;
    f.insert("early", A);
    let (mut socket, status) = connect(
        f.gateway.address,
        "early",
        "origin.invalid",
        f._origin.addr.port(),
        b"early\0data",
    )
    .await;
    assert_eq!(status, 200);
    let mut data = [0; 10];
    tokio::time::timeout(Duration::from_secs(2), socket.read_exact(&mut data))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&data, b"early\0data");
    finish(socket).await;
}

#[cfg(feature = "openssl-compat")]
fn tls_server() -> (Arc<openssl::ssl::SslAcceptor>, Vec<u8>) {
    use openssl::{
        asn1::Asn1Time,
        bn::BigNum,
        hash::MessageDigest,
        pkey::PKey,
        rsa::Rsa,
        ssl::{SslAcceptor, SslMethod},
        x509::{
            X509, X509NameBuilder,
            extension::{BasicConstraints, SubjectAlternativeName},
        },
    };
    let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_text("CN", "localhost").unwrap();
    let name = name.build();
    let mut cert = X509::builder().unwrap();
    cert.set_version(2).unwrap();
    cert.set_serial_number(&BigNum::from_u32(1).unwrap().to_asn1_integer().unwrap())
        .unwrap();
    cert.set_subject_name(&name).unwrap();
    cert.set_issuer_name(&name).unwrap();
    cert.set_pubkey(&key).unwrap();
    cert.set_not_before(&Asn1Time::days_from_now(0).unwrap())
        .unwrap();
    cert.set_not_after(&Asn1Time::days_from_now(1).unwrap())
        .unwrap();
    cert.append_extension(BasicConstraints::new().critical().build().unwrap())
        .unwrap();
    let san = SubjectAlternativeName::new()
        .dns("localhost")
        .build(&cert.x509v3_context(None, None))
        .unwrap();
    cert.append_extension(san).unwrap();
    cert.sign(&key, MessageDigest::sha256()).unwrap();
    let cert = cert.build();
    let mut acceptor = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls_server()).unwrap();
    acceptor.set_private_key(&key).unwrap();
    acceptor.set_certificate(&cert).unwrap();
    acceptor.check_private_key().unwrap();
    (Arc::new(acceptor.build()), cert.to_der().unwrap())
}
#[cfg(feature = "openssl-compat")]
#[tokio::test]
async fn openssl_coexistence_probe() {
    for (protocol, entrance) in [
        (ExitProtocol::Http, "http"),
        (ExitProtocol::Socks5, "http"),
        (ExitProtocol::Http, "socks5h"),
        (ExitProtocol::Socks5, "socks5h"),
    ] {
        let (tls, certificate) = tls_server();
        let hits = Arc::new(AtomicUsize::new(0));
        let seen = hits.clone();
        let origin = server(move |socket| {
        let tls = tls.clone(); let hits = seen.clone();
        async move {
            let ssl = openssl::ssl::Ssl::new(tls.context()).unwrap();
            let mut stream = tokio_openssl::SslStream::new(ssl, socket).unwrap();
            std::pin::Pin::new(&mut stream).accept().await.unwrap();
            hits.fetch_add(1, Ordering::SeqCst);
            let mut stream = BufReader::new(stream);
            let request = head(&mut stream).await.unwrap();
            assert!(request.starts_with(b"GET /proof HTTP/1.1\r\n"));
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\nloopback-ok").await.unwrap();
            stream.shutdown().await.unwrap();
        }
    }).await;
        let f = Fixture::with_origin(protocol, origin, hits).await;
        f.insert("tls-shop", A);
        let client = reqwest::Client::builder()
            .no_proxy()
            .proxy(
                reqwest::Proxy::all(format!("{entrance}://{}", f.gateway.address))
                    .unwrap()
                    .basic_auth("tls-shop", LOCAL_SECRET),
            )
            .add_root_certificate(reqwest::Certificate::from_der(&certificate).unwrap())
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let response = client
            .get(format!("https://localhost:{}/proof", f._origin.addr.port()))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.text().await.unwrap(), "loopback-ok");
        assert_eq!(f.events.lock().unwrap().len(), 2);
        assert_eq!(f.origin_hits.load(Ordering::SeqCst), 1);
        assert!(openssl::version::version().starts_with("OpenSSL "));
        assert!(rustls::RootCertStore::empty().is_empty());
    }
}

#[tokio::test]
async fn socks_authentication_rejects_wrong_secret_unknown_and_removed_sessions() {
    let f = Fixture::new(ExitProtocol::Http).await;
    f.insert("valid", A);
    f.insert("removed", B);
    f.sessions.remove("removed");
    for (key, secret) in [
        ("valid", "wrong-secret"),
        ("missing", LOCAL_SECRET),
        ("removed", LOCAL_SECRET),
    ] {
        let client = reqwest::Client::builder()
            .no_proxy()
            .proxy(
                reqwest::Proxy::all(format!("socks5h://{}", f.gateway.address))
                    .unwrap()
                    .basic_auth(key, secret),
            )
            .timeout(Duration::from_secs(2))
            .retry(reqwest::retry::never())
            .build()
            .unwrap();
        assert!(
            client
                .get("http://origin.invalid/proof")
                .send()
                .await
                .is_err()
        );
    }
    assert!(f.events.lock().unwrap().is_empty());
    assert_eq!(f.origin_hits.load(Ordering::SeqCst), 0);
}
