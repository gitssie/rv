use crate::{AppCommand, AppEvent, SessionEvent, SessionHandle};
use rv_core::{ConnectRequest, Connection};
use serde_json::json;
use std::{
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[tokio::test]
async fn offline_register_is_terminal_and_controller_can_close() {
    let port = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut connection = Connection::new("offline", "127.0.0.1", 5900);
    connection.zxtouch_port = Some(port.local_addr().unwrap().port());
    let (commands, rx) = tokio::sync::mpsc::unbounded_channel();
    let (events, received) = std::sync::mpsc::channel();
    commands
        .send(crate::SessionCommand::Function(
            crate::FunctionCommand::Register {
                text: "a@example.test user secret".into(),
                marketing: false,
                accept_terms: true,
            },
        ))
        .unwrap();
    let task = tokio::spawn(super::run(
        ConnectRequest::from_connection(&connection, None),
        rx,
        events,
    ));
    let event =
        tokio::task::spawn_blocking(move || received.recv_timeout(Duration::from_secs(2)).unwrap())
            .await
            .unwrap();
    assert!(matches!(
        event,
        SessionEvent::Functions(crate::FunctionEvent::Registration(
            crate::functions::registration::Status { running: false, .. }
        ))
    ));
    commands.send(crate::SessionCommand::Close).unwrap();
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
}

#[test]
fn independent_control_recovers_login_and_token_without_connecting_vnc() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let control_port = listener.local_addr().unwrap().port();
    let vnc = TcpListener::bind("127.0.0.1:0").unwrap();
    vnc.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let logins = Arc::new(AtomicUsize::new(0));
    let token_epoch = Arc::new(AtomicUsize::new(0));
    let paths = Arc::new(Mutex::new(Vec::new()));
    let server = {
        let (stop, logins, token_epoch, paths) = (
            stop.clone(),
            logins.clone(),
            token_epoch.clone(),
            paths.clone(),
        );
        thread::spawn(move || {
            tokio::runtime::Runtime::new().unwrap().block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            while !stop.load(Ordering::SeqCst) {
                let socket = tokio::select! {
                    accepted = listener.accept() => accepted.unwrap().0,
                    _ = tokio::time::sleep(Duration::from_millis(10)) => continue,
                };
                let (logins, token_epoch, paths) = (logins.clone(), token_epoch.clone(), paths.clone());
                tokio::spawn(async move {
                    let service = hyper::service::service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                        let (logins, token_epoch, paths) = (logins.clone(), token_epoch.clone(), paths.clone());
                        async move {
                            use http_body_util::BodyExt;
                            assert_eq!(request.version(), hyper::Version::HTTP_2);
                            let path = request.uri().path().to_owned();
                            paths.lock().unwrap().push(path.clone());
                            let bearer = request.headers().get("authorization").and_then(|h| h.to_str().ok()).unwrap_or("").to_owned();
                            let bytes = request.into_body().collect().await.unwrap().to_bytes();
                            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                            let (status, value) = if path == "/api/control/connect" {
                                assert_eq!(body["password"], "fixture-password");
                                let login = logins.fetch_add(1, Ordering::SeqCst) + 1;
                                if login == 1 { (401, json!({"error":"fixture_first_login_unavailable"})) }
                                else {
                                    token_epoch.store(login, Ordering::SeqCst);
                                    let token = format!("header.payload.signature{login}");
                                    (200, json!({"access_token":token,"token_type":"Bearer","expires_in":3600,
                                        "capabilities":{"control":true,"list":true,"launch":true,"foreground":true,"device_info":true,
                                            "zx_control":{"version":3,"protocol":"http2","port":control_port,"token":token}}}))
                                }
                            } else if bearer != format!("Bearer header.payload.signature{}", token_epoch.load(Ordering::SeqCst)) {
                                (401, json!({"error":"expired"}))
                            } else {
                                (200, match path.as_str() {
                                    "/api/control/authorize" | "/api/control/disconnect" => json!({"ok":true}),
                                    "/api/device/info" => json!({"model":"iPhone Test","os_version":"15.8.8"}),
                                    "/api/apps/list" => json!({"apps":[]}),
                                    "/api/apps/foreground" => json!({"bundle_id":"fr.vinted"}),
                                    "/api/apps/launch" => { assert_eq!(body["bundle_id"], "fr.vinted"); json!({"bundle_id":"fr.vinted"}) },
                                    "/api/apple/queryAccountState" => json!({"operation_id":"account","method":"queryAccountState","status":"completed","result":{}}),
                                    _ => panic!("unexpected path {path}"),
                                })
                            };
                            Ok::<_, std::convert::Infallible>(hyper::Response::builder().status(status)
                                .body(http_body_util::Full::new(hyper::body::Bytes::from(value.to_string()))).unwrap())
                        }
                    });
                    let _ = hyper::server::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new())
                        .serve_connection(hyper_util::rt::TokioIo::new(socket), service).await;
                });
            }
        })
        })
    };
    let mut connection = Connection::new("fixture", "127.0.0.1", vnc.local_addr().unwrap().port());
    connection.zxtouch_port = Some(control_port);
    let handle = SessionHandle::spawn_control(ConnectRequest::from_connection(
        &connection,
        Some("fixture-password".into()),
    ));
    let wait = |predicate: &dyn Fn(&SessionEvent) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            for event in handle.drain() {
                if predicate(&event) {
                    return;
                }
            }
            assert!(
                Instant::now() < deadline,
                "independent control event timed out"
            );
            thread::sleep(Duration::from_millis(20));
        }
    };
    wait(&|e| matches!(e, SessionEvent::ControlState(true)));
    handle.app(AppCommand::DeviceInfo);
    wait(
        &|e| matches!(e, SessionEvent::Apps(AppEvent::DeviceInfo(v)) if v["model"] == "iPhone Test"),
    );
    handle.app(AppCommand::Launch("fr.vinted".into()));
    wait(&|e| {
        matches!(
            e,
            SessionEvent::Apps(AppEvent::Finished(AppCommand::Launch(_)))
        )
    });
    // Invalidating the token simulates a phone daemon restart; shared clients recover.
    token_epoch.store(999, Ordering::SeqCst);
    let renewed = Instant::now() + Duration::from_secs(10);
    while logins.load(Ordering::SeqCst) < 3 {
        assert!(
            Instant::now() < renewed,
            "token must be renewed without a VNC session"
        );
        thread::sleep(Duration::from_millis(20));
    }
    handle.app(AppCommand::DeviceInfo);
    wait(&|e| matches!(e, SessionEvent::Apps(AppEvent::DeviceInfo(_))));
    assert!(logins.load(Ordering::SeqCst) >= 3);
    assert!(
        vnc.accept().is_err(),
        "ZXTouch must never open the VNC port"
    );
    handle.close();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !paths
        .lock()
        .unwrap()
        .iter()
        .any(|p| p == "/api/control/disconnect")
    {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        paths
            .lock()
            .unwrap()
            .iter()
            .filter(|p| *p == "/api/apps/launch")
            .count(),
        1,
        "mutation must not be replayed after token renewal"
    );
    stop.store(true, Ordering::SeqCst);
    drop(handle);
    server.join().unwrap();
}

#[tokio::test]
async fn preauthorization_error_never_marks_phone_online_or_reports_wrong_password() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let handler = hyper::service::service_fn(
                    |request: hyper::Request<hyper::body::Incoming>| async move {
                        assert_eq!(request.uri().path(), "/api/control/connect");
                        Ok::<_, std::convert::Infallible>(
                            hyper::Response::builder()
                                .status(401)
                                .body(http_body_util::Full::new(hyper::body::Bytes::from_static(
                                    b"{\"error\":\"Control authorization expired or unavailable\"}",
                                )))
                                .unwrap(),
                        )
                    },
                );
                let _ =
                    hyper::server::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new())
                        .serve_connection(hyper_util::rt::TokioIo::new(socket), handler)
                        .await;
            });
        }
    });
    let mut connection = Connection::new("fixture", "127.0.0.1", 5901);
    connection.zxtouch_port = Some(port);
    let (commands, incoming) = tokio::sync::mpsc::unbounded_channel();
    let (events, received) = std::sync::mpsc::channel();
    let worker = tokio::spawn(super::run(
        ConnectRequest::from_connection(&connection, None),
        incoming,
        events,
    ));
    let rejected = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            while let Ok(event) = received.try_recv() {
                assert!(
                    !matches!(event, SessionEvent::ControlState(true)),
                    "HTTP response must not grant authenticated control"
                );
                if let SessionEvent::Status(message) = event {
                    assert!(
                        !message.contains("密码验证失败"),
                        "old route has not checked the password"
                    );
                    if message.contains("前置授权拦截") {
                        return;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    commands.send(crate::SessionCommand::Close).unwrap();
    worker.await.unwrap();
    server.abort();
    assert!(
        rejected.is_ok(),
        "legacy authorization rejection must be classified correctly"
    );
}
