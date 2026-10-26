use super::*;
use tokio::net::TcpListener;

use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, body::Incoming, service::service_fn};
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::sync::oneshot;
use tokio::{io::AsyncWriteExt, net::TcpStream};
struct TestStream {
    packets: mpsc::Receiver<(String, Value, oneshot::Sender<Value>)>,
    pending: Option<oneshot::Sender<Value>>,
    connection: tokio::task::JoinHandle<()>,
}
impl Drop for TestStream {
    fn drop(&mut self) {
        if self.pending.is_some() {
            self.connection.abort();
        }
    }
}
impl TestStream {
    fn new(socket: TcpStream) -> Self {
        let (send, packets) = mpsc::channel(4);
        let connection = tokio::spawn(async move {
            let service = service_fn(move |request: Request<Incoming>| {
                let send = send.clone();
                async move {
                    assert_eq!(request.version(), hyper::Version::HTTP_2);
                    assert!(
                        request.headers()["authorization"]
                            .to_str()
                            .unwrap()
                            .starts_with("Bearer ")
                    );
                    let path = request.uri().path().to_string();
                    let bytes = request.into_body().collect().await.unwrap().to_bytes();
                    let value = serde_json::from_slice(&bytes).unwrap();
                    let (reply, result) = oneshot::channel::<Value>();
                    send.send((path, value, reply)).await.unwrap();
                    let value = result
                        .await
                        .unwrap_or_else(|_| json!({"error":"disconnected"}));
                    let status = if value["error"].is_null() { 200 } else { 400 };
                    Ok::<_, std::convert::Infallible>(
                        Response::builder()
                            .status(status)
                            .header("content-type", "application/json")
                            .body(Full::new(hyper::body::Bytes::from(value.to_string())))
                            .unwrap(),
                    )
                }
            });
            let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(socket), service)
                .await;
        });
        Self {
            packets,
            pending: None,
            connection,
        }
    }
    async fn read_line(&mut self, _: &mut String) -> std::io::Result<usize> {
        let value = tokio::time::timeout(Duration::from_millis(200), self.packets.recv()).await;
        assert!(!matches!(value, Ok(Some(_))), "unexpected HTTP request");
        Ok(0)
    }
}
async fn packet(stream: &mut TestStream) -> (u8, Value) {
    let (path, body, reply) = stream.packets.recv().await.unwrap();
    stream.pending = Some(reply);
    if path == "/api/control/authorize" {
        return (33, json!({"op":0}));
    }
    let (module, method) = if let Some(method) = path.strip_prefix("/api/apple/") {
        (31, method)
    } else {
        (
            32,
            path.strip_prefix("/api/app-state/")
                .expect("named native route"),
        )
    };
    let request = if matches!(method, "result" | "submit_code" | "cancel_challenge") {
        let mut body = body;
        body["action"] = json!(method);
        body
    } else {
        json!({"action":"invoke", "method":method, "params":body})
    };
    (module, request)
}
async fn reply(stream: &mut TestStream, value: Value) {
    let value = if value["version"] == 1 {
        json!({"ok":true})
    } else {
        value
    };
    stream.pending.take().unwrap().send(value).unwrap();
}
async fn reconnect(_listener: &TcpListener, mut socket: TestStream) -> TestStream {
    // Reauthorize a new operation on the same multiplexed connection.
    let (module, auth) = packet(&mut socket).await;
    assert_eq!(module, 33);
    assert_eq!(auth["op"], 0);
    reply(&mut socket, json!({"ok":true})).await;
    socket
}

async fn next(events: &mut mpsc::Receiver<FunctionEvent>) -> FunctionEvent {
    tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn resume_authorization_failure_keeps_unknown_and_allows_explicit_requery() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = crate::phone_http::PhoneHttpClient::new(
        "127.0.0.1",
        listener.local_addr().unwrap().port(),
        "test".into(),
    )
    .unwrap();
    let (commands, rx) = mpsc::channel(8);
    let (events, mut states) = mpsc::channel(8);
    let actor = tokio::spawn(worker(client, rx, events));
    assert!(matches!(
        next(&mut states).await,
        FunctionEvent::Available(true)
    ));
    commands
        .send(FunctionCommand::Resume {
            task: 32,
            operation_id: "original".into(),
        })
        .await
        .unwrap();
    let (socket, _) = listener.accept().await.unwrap();
    let mut socket = TestStream::new(socket);
    assert_eq!(packet(&mut socket).await.0, 33);
    reply(&mut socket, json!({"error":"unauthorized"})).await;
    assert!(matches!(
        next(&mut states).await,
        FunctionEvent::Uncertain(_)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(1100), socket.packets.recv())
            .await
            .is_err(),
        "failed query must not retry in background"
    );
    commands
        .send(FunctionCommand::Resume {
            task: 32,
            operation_id: "original".into(),
        })
        .await
        .unwrap();
    assert_eq!(packet(&mut socket).await.0, 33);
    reply(&mut socket, json!({"ok":true})).await;
    let (task, query) = packet(&mut socket).await;
    assert_eq!(task, 32);
    assert_eq!(query, json!({"action":"result","operation_id":"original"}));
    reply(
        &mut socket,
        json!({"operation_id":"original","status":"completed","result":{}}),
    )
    .await;
    assert!(
        matches!(next(&mut states).await, FunctionEvent::State { task:32, record } if record["status"] == "completed")
    );
    actor.abort();
}

#[tokio::test]
async fn challenge_fetches_once_and_never_replays_login() {
    challenge_flow(false, false, false).await;
}
#[tokio::test]
async fn automatic_code_rejection_is_reported() {
    challenge_flow(true, false, false).await;
}
#[tokio::test]
async fn challenge_retries_code_link_until_code_arrives() {
    tokio::time::timeout(Duration::from_secs(12), challenge_flow(false, true, false))
        .await
        .unwrap();
}
#[tokio::test]
async fn challenge_manual_fetch_bypasses_retry_delay() {
    tokio::time::timeout(Duration::from_secs(12), challenge_flow(false, true, true))
        .await
        .unwrap();
}
async fn challenge_flow(reject: bool, delayed_code: bool, manual_retry: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://127.0.0.1:{}/get?id=123",
        http.local_addr().unwrap().port()
    );
    let http_task = tokio::spawn(async move {
        if delayed_code {
            let (mut stream, _) = http.accept().await.unwrap();
            let mut request = [0; 4096];
            use tokio::io::AsyncReadExt;
            stream.read(&mut request).await.unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                .await
                .unwrap();
        }
        let (mut stream, _) = tokio::time::timeout(
            Duration::from_secs(if manual_retry { 2 } else { 6 }),
            http.accept(),
        )
        .await
        .unwrap()
        .unwrap();
        let mut buffer = [0; 4096];
        use tokio::io::AsyncReadExt;
        let count = stream.read(&mut buffer).await.unwrap();
        assert!(
            std::str::from_utf8(&buffer[..count])
                .unwrap()
                .starts_with("GET /get?id=123 ")
        );
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 17\r\nConnection: close\r\n\r\n{\"code\":\"012345\"}").await.unwrap();
    });
    let endpoint = ControlEndpoint {
        version: 2,
        port: listener.local_addr().unwrap().port(),
        token: "a".repeat(64),
    };
    let (commands, rx) = mpsc::channel(8);
    let (events, mut states) = mpsc::channel(32);
    let actor = tokio::spawn(worker(
        crate::phone_http::PhoneHttpClient::new("127.0.0.1", endpoint.port, endpoint.token)
            .unwrap(),
        rx,
        events,
    ));
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = TestStream::new(socket);
        let (task, auth) = packet(&mut socket).await;
        assert_eq!(task, 33);
        assert_eq!(auth["op"], 0);
        reply(
            &mut socket,
            json!({"version":1,"op":0,"id":0,"status":0,"payload":{"ok":true}}),
        )
        .await;
        let (task, invoke) = packet(&mut socket).await;
        assert_eq!(task, 31);
        assert_eq!(invoke["method"], "signInAppStore");
        assert!(
            !invoke["params"]
                .as_object()
                .unwrap()
                .contains_key("code_url")
        );
        reply(
            &mut socket,
            json!({"operation_id":"operation-1","method":"signInAppStore","status":"running"}),
        )
        .await;
        socket = reconnect(&listener, socket).await;
        let (_, query) = packet(&mut socket).await;
        assert_eq!(query["action"], "result");
        reply(&mut socket,json!({"operation_id":"operation-1","method":"signInAppStore","status":"requires_verification_code","challenge":{"challenge_id":"challenge-1"}})).await;
        let submit = loop {
            socket = reconnect(&listener, socket).await;
            let (_, request) = packet(&mut socket).await;
            if delayed_code && request["action"] == "result" {
                reply(&mut socket,json!({"operation_id":"operation-1","method":"signInAppStore","status":"requires_verification_code","challenge":{"challenge_id":"challenge-1"}})).await;
            } else {
                break request;
            }
        };
        assert_eq!(submit["action"], "submit_code");
        assert_eq!(submit["code"], "012345");
        if reject {
            reply(&mut socket, json!({"error":{"code":"code_rejected"}})).await;
            // Repeated challenge is never fetched/submitted automatically again.
            socket = reconnect(&listener, socket).await;
            let (_, query) = packet(&mut socket).await;
            assert_eq!(query["action"], "result");
            reply(&mut socket,json!({"operation_id":"operation-1","method":"signInAppStore","status":"requires_verification_code","challenge":{"challenge_id":"challenge-1"}})).await;
        } else {
            reply(
                &mut socket,
                json!({"operation_id":"operation-1","accepted":true}),
            )
            .await;
            socket = reconnect(&listener, socket).await;
            let (_, query) = packet(&mut socket).await;
            assert_eq!(query["action"], "result");
            reply(&mut socket,json!({"operation_id":"operation-1","method":"signInAppStore","status":"completed","result":{"outcome":"succeeded"}})).await;
        }
    });
    assert!(matches!(
        next(&mut states).await,
        FunctionEvent::Available(true)
    ));
    // No HTTP call occurs before the native challenge.
    commands
        .send(FunctionCommand::Invoke {
            id: "store-login".into(),
            text: format!("account@example.test----secret----{url}"),
        })
        .await
        .unwrap();
    loop {
        match next(&mut states).await {
            FunctionEvent::CodeFetchWaiting { .. } if manual_retry => {
                commands.send(FunctionCommand::FetchCode).await.unwrap();
            }
            FunctionEvent::CodeFetched(code) if manual_retry => {
                commands
                    .send(FunctionCommand::SubmitCode {
                        operation_id: "operation-1".into(),
                        challenge_id: "challenge-1".into(),
                        code,
                    })
                    .await
                    .unwrap();
            }
            FunctionEvent::Error(e) if reject => {
                assert!(e.contains("code_rejected"));
                break;
            }
            FunctionEvent::State { record, .. } if !reject && record["status"] == "completed" => {
                break;
            }
            FunctionEvent::Uncertain(e) => panic!("{e}"),
            _ => {}
        }
    }
    http_task.await.unwrap();
    peer.await.unwrap();
    actor.abort();
}

#[tokio::test]
async fn resumed_worker_requires_query_before_code_submission() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ControlEndpoint {
        version: 2,
        port: listener.local_addr().unwrap().port(),
        token: "b".repeat(64),
    };
    let (commands, rx) = mpsc::channel(8);
    let (events, mut states) = mpsc::channel(32);
    let actor = tokio::spawn(worker(
        crate::phone_http::PhoneHttpClient::new("127.0.0.1", endpoint.port, endpoint.token)
            .unwrap(),
        rx,
        events,
    ));
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = TestStream::new(socket);
        packet(&mut socket).await;
        reply(&mut socket, json!({"version":1,"op":0,"id":0,"status":0})).await;
        let (_, query) = packet(&mut socket).await;
        assert_eq!(query["action"], "result");
        assert_eq!(query["operation_id"], "old-operation");
        reply(&mut socket,json!({"operation_id":"old-operation","status":"requires_verification_code","challenge":{"challenge_id":"challenge"}})).await;
        socket = reconnect(&listener, socket).await;
        let (_, submit) = packet(&mut socket).await;
        assert_eq!(submit["action"], "submit_code");
        reply(
            &mut socket,
            json!({"operation_id":"old-operation","accepted":true}),
        )
        .await;
    });
    assert!(matches!(
        next(&mut states).await,
        FunctionEvent::Available(true)
    ));
    commands
        .send(FunctionCommand::SubmitCode {
            operation_id: "old-operation".into(),
            challenge_id: "challenge".into(),
            code: "012345".into(),
        })
        .await
        .unwrap();
    assert!(matches!(next(&mut states).await, FunctionEvent::Error(_)));
    commands
        .send(FunctionCommand::Resume {
            task: 31,
            operation_id: "old-operation".into(),
        })
        .await
        .unwrap();
    assert!(matches!(
        next(&mut states).await,
        FunctionEvent::State { .. }
    ));
    commands
        .send(FunctionCommand::SubmitCode {
            operation_id: "old-operation".into(),
            challenge_id: "challenge".into(),
            code: "012345".into(),
        })
        .await
        .unwrap();
    peer.await.unwrap();
    actor.abort();
}

#[test]
fn native_unknown_is_distinct_from_a_finished_partial_failure() {
    assert!(uncertain_result(
        31,
        &json!({"error":{"domain":"org.trollvnc.apple","code":6}})
    ));
    assert!(!uncertain_result(
        31,
        &json!({"error":{"domain":"Accounts","code":6}})
    ));
    assert!(!uncertain_result(
        32,
        &json!({"error":{"domain":"TVAppStateServices","code":3}})
    ));
    assert!(!uncertain_result(
        32,
        &json!({"status":"completed","progress":{"phase":"restore","stage":"copy"},"error":{"domain":"TVAppStateServices","code":2,"state_may_be_partial":true}})
    ));
    assert!(uncertain_result(
        32,
        &json!({"status":"completed","error":{"domain":"TVAppStateServices","code":4,"state_may_be_partial":true}})
    ));
    assert!(uncertain_result(
        31,
        &json!({"error":{"code":"missing_result"}})
    ));
    assert!(accepted_code(&json!({"operation_id":"wrong","accepted":true}), "expected").is_err());
}

#[tokio::test]
async fn next_operation_reauthorizes_on_shared_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ControlEndpoint {
        version: 2,
        port: listener.local_addr().unwrap().port(),
        token: "a".repeat(64),
    };
    let (commands, rx) = mpsc::channel(8);
    let (events, mut states) = mpsc::channel(32);
    let actor = tokio::spawn(worker(
        crate::phone_http::PhoneHttpClient::new("127.0.0.1", endpoint.port, endpoint.token)
            .unwrap(),
        rx,
        events,
    ));
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = TestStream::new(socket);
        for id in ["first", "second"] {
            let (task, auth) = packet(&mut socket).await;
            assert_eq!(task, 33);
            assert_eq!(auth["op"], 0);
            reply(&mut socket, json!({"version":1,"op":0,"id":0,"status":0})).await;
            let (task, invoke) = packet(&mut socket).await;
            assert_eq!(task, 31);
            assert_eq!(invoke["action"], "invoke");
            reply(
                &mut socket,
                json!({"operation_id":id,"status":"completed","result":{}}),
            )
            .await;
            // The next operation reuses this connection.
        }
    });
    assert!(matches!(
        next(&mut states).await,
        FunctionEvent::Available(true)
    ));
    for id in ["first", "second"] {
        commands
            .send(FunctionCommand::Invoke {
                id: "apple-query".into(),
                text: String::new(),
            })
            .await
            .unwrap();
        match next(&mut states).await {
            FunctionEvent::State { record, .. } => assert_eq!(record["operation_id"], id),
            event => panic!("unexpected event: {event:?}"),
        }
    }
    peer.await.unwrap();
    actor.abort();
}

#[tokio::test]
async fn logout_resolves_phone_cloud_on_short_sockets_before_one_write() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ControlEndpoint {
        version: 2,
        port: listener.local_addr().unwrap().port(),
        token: "a".repeat(64),
    };
    let (commands, rx) = mpsc::channel(8);
    let (events, mut states) = mpsc::channel(32);
    let actor = tokio::spawn(worker(
        crate::phone_http::PhoneHttpClient::new("127.0.0.1", endpoint.port, endpoint.token)
            .unwrap(),
        rx,
        events,
    ));
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = TestStream::new(socket);
        assert_eq!(packet(&mut socket).await.0, 33);
        reply(&mut socket, json!({"version":1,"op":0,"id":0,"status":0})).await;
        assert_eq!(
            packet(&mut socket).await,
            (
                31,
                json!({"action":"invoke","method":"queryAccountState","params":{}})
            )
        );
        reply(
            &mut socket,
            json!({"operation_id":"read","status":"running"}),
        )
        .await;
        socket = reconnect(&listener, socket).await;
        assert_eq!(
            packet(&mut socket).await,
            (31, json!({"action":"result","operation_id":"read"}))
        );
        reply(
            &mut socket,
            json!({"operation_id":"read","status":"completed","result":{
                "icloud":{"status":"signed_in","account":{"apple_id":"cloud@example.test"}},
                "app_store":{"status":"signed_in","account":{"apple_id":"store@example.test"}}
            }}),
        )
        .await;
        socket = reconnect(&listener, socket).await;
        assert_eq!(
            packet(&mut socket).await,
            (
                31,
                json!({"action":"invoke","method":"signOutICloud","params":{"apple_id":"cloud@example.test","local_data_policy":"keep"}})
            )
        );
        reply(&mut socket, json!({"operation_id":"logout","method":"signOutICloud","status":"completed","result":{"outcome":"succeeded"}})).await;
        let mut line = String::new();
        assert_eq!(socket.read_line(&mut line).await.unwrap(), 0);
        assert!(
            tokio::time::timeout(Duration::from_millis(300), listener.accept())
                .await
                .is_err(),
            "must not replay logout"
        );
    });
    assert!(matches!(
        next(&mut states).await,
        FunctionEvent::Available(true)
    ));
    commands
        .send(FunctionCommand::Invoke {
            id: "cloud-logout".into(),
            text: "本机数据=保留".into(),
        })
        .await
        .unwrap();
    assert!(
        matches!(next(&mut states).await, FunctionEvent::State { record, .. } if record["operation_id"] == "logout")
    );
    peer.await.unwrap();
    actor.abort();
}

#[tokio::test]
async fn absent_cloud_never_submits_logout_or_falls_back_to_store() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ControlEndpoint {
        version: 2,
        port: listener.local_addr().unwrap().port(),
        token: "a".repeat(64),
    };
    let (commands, rx) = mpsc::channel(8);
    let (events, mut states) = mpsc::channel(32);
    let actor = tokio::spawn(worker(
        crate::phone_http::PhoneHttpClient::new("127.0.0.1", endpoint.port, endpoint.token)
            .unwrap(),
        rx,
        events,
    ));
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = TestStream::new(socket);
        assert_eq!(packet(&mut socket).await.0, 33);
        reply(&mut socket, json!({"version":1,"op":0,"id":0,"status":0})).await;
        assert_eq!(packet(&mut socket).await.1["method"], "queryAccountState");
        reply(
            &mut socket,
            json!({"operation_id":"read","status":"completed","result":{
                "icloud":{"status":"not_signed_in","account":null},
                "app_store":{"status":"signed_in","account":{"apple_id":"store@example.test"}}
            }}),
        )
        .await;
        let mut line = String::new();
        assert_eq!(socket.read_line(&mut line).await.unwrap(), 0);
        assert!(
            tokio::time::timeout(Duration::from_millis(300), listener.accept())
                .await
                .is_err()
        );
    });
    assert!(matches!(
        next(&mut states).await,
        FunctionEvent::Available(true)
    ));
    commands
        .send(FunctionCommand::Invoke {
            id: "cloud-logout".into(),
            text: "本机数据=移除".into(),
        })
        .await
        .unwrap();
    assert!(
        matches!(next(&mut states).await, FunctionEvent::Error(message) if message.contains("未登录 iCloud"))
    );
    peer.await.unwrap();
    actor.abort();
}

#[tokio::test]
async fn query_followup_rechecks_bound_account_before_exactly_one_mutation() {
    for matching in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (commands, rx) = mpsc::channel(8);
        let (events, mut states) = mpsc::channel(32);
        let actor = tokio::spawn(worker(
            crate::phone_http::PhoneHttpClient::new(
                "127.0.0.1",
                listener.local_addr().unwrap().port(),
                "a".repeat(64),
            )
            .unwrap(),
            rx,
            events,
        ));
        let peer = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = TestStream::new(socket);
            assert_eq!(packet(&mut socket).await.0, 33);
            reply(&mut socket, json!({"version":1,"op":0,"id":0,"status":0})).await;
            assert_eq!(packet(&mut socket).await.1["method"], "queryAccountState");
            let current = if matching {
                "original@example.test"
            } else {
                "changed@example.test"
            };
            reply(
                &mut socket,
                json!({"operation_id":"read","status":"completed","result":{
                    "icloud":{"status":"signed_in","account":{"apple_id":current}},
                    "identity":{"status":"signed_in","account":{"apple_id":current}}
                }}),
            )
            .await;
            if matching {
                socket = reconnect(&listener, socket).await;
                assert_eq!(
                    packet(&mut socket).await,
                    (
                        31,
                        json!({"action":"invoke","method":"stopUsingAppleID",
                    "params":{"apple_id":"original@example.test","bundle_id":"fr.vinted"}})
                    )
                );
                reply(&mut socket, json!({"operation_id":"stop","method":"stopUsingAppleID","status":"completed","result":{"outcome":"succeeded"}})).await;
            }
            let mut line = String::new();
            assert_eq!(socket.read_line(&mut line).await.unwrap(), 0);
            assert!(
                tokio::time::timeout(Duration::from_millis(300), listener.accept())
                    .await
                    .is_err(),
                "unexpected mutation or replay"
            );
        });
        assert!(matches!(
            next(&mut states).await,
            FunctionEvent::Available(true)
        ));
        commands
            .send(FunctionCommand::InvokeForAccount {
                id: "authorization-stop".into(),
                text: "App=fr.vinted".into(),
                apple_id: "original@example.test".into(),
            })
            .await
            .unwrap();
        let event = next(&mut states).await;
        if matching {
            assert!(
                matches!(event, FunctionEvent::State {record,..} if record["operation_id"] == "stop")
            );
        } else {
            assert!(
                matches!(event, FunctionEvent::Error(message) if message.contains("账户已改变"))
            );
        }
        peer.await.unwrap();
        actor.abort();
    }
}

#[tokio::test]
async fn email_verification_supplies_native_challenge_without_user_identifiers() {
    email_flow(false).await;
}

#[tokio::test]
async fn resumed_email_addition_restores_challenge_before_code_only_submission() {
    email_flow(true).await;
}

async fn email_flow(resumed: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ControlEndpoint {
        version: 2,
        port: listener.local_addr().unwrap().port(),
        token: "a".repeat(64),
    };
    let (commands, rx) = mpsc::channel(8);
    let (events, mut states) = mpsc::channel(32);
    let actor = tokio::spawn(worker(
        crate::phone_http::PhoneHttpClient::new("127.0.0.1", endpoint.port, endpoint.token)
            .unwrap(),
        rx,
        events,
    ));
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = TestStream::new(socket);
        for method in [
            if resumed { "" } else { "queryAccountState" },
            "beginAddingContactEmail",
            "queryAccountState",
            "verifyContactEmail",
        ] {
            if method.is_empty() {
                continue;
            }
            assert_eq!(packet(&mut socket).await.0, 33);
            reply(&mut socket, json!({"version":1,"op":0,"id":0,"status":0})).await;
            let (task, request) = packet(&mut socket).await;
            assert_eq!(task, 31);
            if resumed && method == "beginAddingContactEmail" {
                assert_eq!(
                    request,
                    json!({"action":"result","operation_id":"beginAddingContactEmail"})
                );
            } else {
                assert_eq!(request["method"], method);
            }
            let result = match method {
                "queryAccountState" => json!({
                    "icloud":{"status":"signed_in","account":{"apple_id":"phone@example.test"}},
                    "identity":{"status":"signed_in","account":{"apple_id":"phone@example.test"}}
                }),
                "beginAddingContactEmail" => {
                    if !resumed {
                        assert_eq!(
                            request["params"],
                            json!({"apple_id":"phone@example.test","email":"contact@example.test"})
                        );
                    }
                    json!({"outcome":"requires_email_verification","challenge_id":"native-challenge","expires_in":300})
                }
                "verifyContactEmail" => {
                    assert_eq!(
                        request["params"],
                        json!({"apple_id":"phone@example.test","challenge_id":"native-challenge","code":"012345"})
                    );
                    json!({"outcome":"succeeded"})
                }
                _ => unreachable!(),
            };
            reply(
                &mut socket,
                json!({"operation_id":method,"method":method,"status":"completed","result":result}),
            )
            .await;
        }
    });
    assert!(matches!(
        next(&mut states).await,
        FunctionEvent::Available(true)
    ));
    for (id, text, method) in [
        (
            "email-add",
            "邮箱=contact@example.test",
            "beginAddingContactEmail",
        ),
        ("email-verify", "验证码=012345", "verifyContactEmail"),
    ] {
        commands
            .send(if resumed && id == "email-add" {
                FunctionCommand::Resume {
                    task: 31,
                    operation_id: "beginAddingContactEmail".into(),
                }
            } else {
                FunctionCommand::Invoke {
                    id: id.into(),
                    text: text.into(),
                }
            })
            .await
            .unwrap();
        assert!(
            matches!(next(&mut states).await, FunctionEvent::State { record, .. } if record["method"] == method)
        );
    }
    peer.await.unwrap();
    actor.abort();
}

struct MockMailboxProvider;
struct MockMailbox;
impl mailbox::MailboxProvider for MockMailboxProvider {
    fn validate(&self, _: &mailbox::Credentials) -> Result<(), String> {
        Ok(())
    }
    fn prepare(
        &self,
        credentials: mailbox::Credentials,
    ) -> futures_util::future::BoxFuture<'_, Result<Box<dyn mailbox::CodeMailbox>, String>> {
        Box::pin(async move {
            assert_eq!(credentials.email, "demo@outlook.com");
            assert_eq!(credentials.refresh_token.as_str(), "test-refresh");
            Ok(Box::new(MockMailbox) as Box<dyn mailbox::CodeMailbox>)
        })
    }
}
impl mailbox::CodeMailbox for MockMailbox {
    fn arm(&mut self) -> futures_util::future::BoxFuture<'_, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
    fn poll(&mut self) -> futures_util::future::BoxFuture<'_, Result<Option<String>, String>> {
        Box::pin(async { Ok(Some("012345".into())) })
    }
}
#[tokio::test]
async fn oauth_email_addition_verifies_once_without_exposing_material_to_phone() {
    automatic_email_flow(false, false).await;
}
#[tokio::test]
async fn oauth_email_verification_disconnect_is_uncertain_and_never_replays() {
    automatic_email_flow(true, false).await;
}
#[tokio::test]
async fn oauth_email_does_not_verify_after_phone_account_changes() {
    automatic_email_flow(false, true).await;
}
async fn automatic_email_flow(disconnect: bool, changed: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ControlEndpoint {
        version: 2,
        port: listener.local_addr().unwrap().port(),
        token: "a".repeat(64),
    };
    let (commands, rx) = mpsc::channel(8);
    let (events, mut states) = mpsc::channel(32);
    let actor = tokio::spawn(worker_with_provider(
        crate::phone_http::PhoneHttpClient::new("127.0.0.1", endpoint.port, endpoint.token)
            .unwrap(),
        rx,
        events,
        &MockMailboxProvider,
    ));
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = TestStream::new(socket);
        for (index, method) in [
            "queryAccountState",
            "queryAccountState",
            "beginAddingContactEmail",
            "queryAccountState",
            "verifyContactEmail",
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(packet(&mut socket).await.0, 33);
            reply(&mut socket, json!({"version":1,"op":0,"id":0,"status":0})).await;
            let (_, request) = packet(&mut socket).await;
            assert_eq!(request["method"], method);
            let result = match method {
                "queryAccountState" => {
                    json!({"icloud":{"status":"signed_in","account":{"apple_id":"phone@example.test"}},"identity":{"status":"signed_in","account":{"apple_id":"phone@example.test"}}})
                }
                "beginAddingContactEmail" => {
                    assert_eq!(
                        request["params"],
                        json!({"apple_id":"phone@example.test","email":"demo@outlook.com"})
                    );
                    json!({"outcome":"requires_email_verification","challenge_id":"native-challenge","expires_in":300})
                }
                "verifyContactEmail" => {
                    assert_eq!(
                        request["params"],
                        json!({"apple_id":"phone@example.test","challenge_id":"native-challenge","code":"012345"})
                    );
                    if disconnect {
                        drop(socket);
                        break;
                    }
                    json!({"outcome":"succeeded"})
                }
                _ => unreachable!(),
            };
            let result = if changed && index == 3 {
                json!({"icloud":{"status":"signed_in","account":{"apple_id":"other@example.test"}},"identity":{"status":"signed_in","account":{"apple_id":"other@example.test"}}})
            } else {
                result
            };
            reply(
                &mut socket,
                json!({"operation_id":method,"method":method,"status":"completed","result":result}),
            )
            .await;
            if changed && index == 3 {
                break;
            }
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(200), listener.accept())
                .await
                .is_err(),
            "mutation must never replay"
        );
    });
    assert!(matches!(
        next(&mut states).await,
        FunctionEvent::Available(true)
    ));
    commands.send(FunctionCommand::Invoke {id:"email-add".into(),text:"demo@outlook.com----test-password----00000000-0000-4000-8000-000000000001----test-refresh".into()}).await.unwrap();
    let mut fetching = false;
    let mut received_code = false;
    loop {
        match next(&mut states).await {
            FunctionEvent::Progress(message) => {
                fetching |= message.contains("验证码");
            }
            FunctionEvent::VerificationCodeReceived(code) => {
                assert_eq!(code, "012345");
                received_code = true;
            }
            FunctionEvent::State { record, .. } if record["method"] == "verifyContactEmail" => {
                assert!(!disconnect);
                assert_eq!(record["result"]["outcome"], "succeeded");
                break;
            }
            FunctionEvent::Uncertain(_) => {
                assert!(disconnect);
                break;
            }
            FunctionEvent::Error(error) => {
                assert!(
                    changed && error.contains("账户已改变"),
                    "unexpected error: {error}"
                );
                break;
            }
            _ => (),
        }
    }
    assert!(fetching);
    assert!(received_code);
    peer.await.unwrap();
    actor.abort();
}

#[tokio::test]
async fn injected_shared_client_does_not_enable_unconfigured_or_view_only_functions() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ControlEndpoint {
        version: 2,
        port: listener.local_addr().unwrap().port(),
        token: "a".repeat(64),
    };
    for allowed in [false, true] {
        let mut runtime = FunctionRuntime::new("127.0.0.1".into(), None, allowed);
        runtime.set_http_client(Some(
            crate::phone_http::PhoneHttpClient::new(
                "127.0.0.1",
                endpoint.port,
                endpoint.token.clone(),
            )
            .unwrap(),
        ));
        if !allowed {
            runtime.configure(&crate::AppCapabilities {
                control: true,
                zx_control: Some(endpoint.clone()),
                ..Default::default()
            });
        }
        let errors = std::cell::Cell::new(0);
        for command in [
            FunctionCommand::Invoke {
                id: "store-login".into(),
                text: "test@example.com----secret".into(),
            },
            FunctionCommand::RefreshAccountState,
            FunctionCommand::ReadEmailCode {
                text: "mail@outlook.com".into(),
            },
        ] {
            runtime.command(command, &|event| {
                assert!(matches!(event, FunctionEvent::Error(_)));
                errors.set(errors.get() + 1);
            });
        }
        assert_eq!(errors.get(), 3);
        assert!(runtime.workers.is_empty());
        assert!(runtime.tasks.is_empty());
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn independent_function_finishes_while_another_socket_waits() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ControlEndpoint {
        version: 2,
        port: listener.local_addr().unwrap().port(),
        token: "a".repeat(64),
    };
    let (entered, mut waiting) = mpsc::channel(1);
    let server = tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let entered = entered.clone();
            tokio::spawn(async move {
                let handler = service_fn(move |request: Request<Incoming>| {
                    let entered = entered.clone();
                    async move {
                        let value = match request.uri().path() {
                            "/api/control/authorize" => json!({"ok":true}),
                            "/api/apple/signInAppStore" => {
                                entered.send(()).await.unwrap();
                                std::future::pending::<Value>().await
                            }
                            "/api/apple/queryAccountState" => {
                                json!({"operation_id":"account","method":"queryAccountState","status":"completed","result":{}})
                            }
                            "/api/apple/queryState" => {
                                json!({"operation_id":"read","method":"queryState","status":"completed","result":{}})
                            }
                            path => panic!("unexpected route {path}"),
                        };
                        Ok::<_, std::convert::Infallible>(Response::new(Full::new(
                            hyper::body::Bytes::from(value.to_string()),
                        )))
                    }
                });
                let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                    .serve_connection(TokioIo::new(socket), handler)
                    .await;
            });
        }
    });
    let mut runtime = FunctionRuntime::new("127.0.0.1".into(), None, true);
    runtime.configure(&crate::AppCapabilities {
        control: true,
        zx_control: Some(endpoint),
        ..Default::default()
    });
    runtime.command(
        FunctionCommand::Invoke {
            id: "store-login".into(),
            text: "test@example.com----secret".into(),
        },
        &|_| {},
    );
    tokio::time::timeout(Duration::from_secs(2), waiting.recv())
        .await
        .unwrap()
        .unwrap();
    runtime.command(
        FunctionCommand::Invoke {
            id: "apple-query".into(),
            text: String::new(),
        },
        &|_| {},
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let event = tokio::time::timeout_at(deadline, runtime.rx.recv())
            .await
            .unwrap()
            .unwrap();
        if let FunctionEvent::ForOperation { id, event } = event {
            if let FunctionEvent::State { record, .. } = *event {
                assert_eq!(id, "apple-query");
                assert_eq!(record["operation_id"], "read");
                break;
            }
        }
    }
    drop(runtime);
    server.abort();
}

#[tokio::test]
async fn cloud_login_resolves_fresh_store_account_before_invoking() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = crate::phone_http::PhoneHttpClient::new(
        "127.0.0.1",
        listener.local_addr().unwrap().port(),
        "a".repeat(64),
    )
    .unwrap();
    let (commands, rx) = mpsc::channel(8);
    let (events, mut states) = mpsc::channel(32);
    let actor = tokio::spawn(worker(client, rx, events));
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = TestStream::new(socket);
        socket = reconnect(&listener, socket).await;
        let (_, query) = packet(&mut socket).await;
        assert_eq!(query["method"], "queryAccountState");
        reply(&mut socket, json!({
            "operation_id":"account-read", "status":"completed", "result": {
                "app_store":{"status":"signed_in","account":{"apple_id":"fresh@test","authenticated":true}},
                "icloud":{"status":"not_signed_in","account":null},
                "identity":{"status":"signed_in","account":{"apple_id":"unrelated@test"}}
            }
        })).await;
        socket = reconnect(&listener, socket).await;
        let (_, invoke) = packet(&mut socket).await;
        assert_eq!(invoke["method"], "enableICloud");
        assert_eq!(
            invoke["params"],
            json!({
                "apple_id":"fresh@test", "password":"test-password", "device_passcode":"001234"
            })
        );
        reply(
            &mut socket,
            json!({
                "operation_id":"cloud-login", "method":"enableICloud", "status":"completed",
                "result":{"outcome":"succeeded"}
            }),
        )
        .await;
    });
    assert!(matches!(
        next(&mut states).await,
        FunctionEvent::Available(true)
    ));
    // Missing local configuration must not even start a phone mutation.
    commands
        .send(FunctionCommand::Invoke {
            id: "cloud-login".into(),
            text: "test-password".into(),
        })
        .await
        .unwrap();
    assert!(
        matches!(next(&mut states).await, FunctionEvent::Error(error) if error.contains("锁屏密码"))
    );
    commands
        .send(FunctionCommand::Invoke {
            id: "cloud-login".into(),
            text: "密码=test-password\n设备密码=001234".into(),
        })
        .await
        .unwrap();
    loop {
        match next(&mut states).await {
            FunctionEvent::State { record, .. } if record["status"] == "completed" => break,
            FunctionEvent::Error(error) | FunctionEvent::Uncertain(error) => panic!("{error}"),
            _ => {}
        }
    }
    peer.await.unwrap();
    actor.abort();
}


#[tokio::test]
async fn managed_first_login_persists_candidate_before_ios_change_and_reuses_new_password(){
 let dir=tempfile::tempdir().unwrap();let paths=rv_core::StorePaths::in_dir(dir.path().into());
 let ctx=apple_accounts::Context::at(paths.clone(),Some("device-a".into()));
 let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
 let client=crate::phone_http::PhoneHttpClient::new("127.0.0.1",listener.local_addr().unwrap().port(),"test".into()).unwrap();
 let (commands,rx)=mpsc::channel(8);let (events,mut states)=mpsc::channel(32);
 let actor=tokio::spawn(apple_accounts::run(client,rx,events,ctx));
 assert!(matches!(next(&mut states).await,FunctionEvent::Available(true)));
 commands.send(FunctionCommand::InvokeForAccount{apple_id:"first@test".into(),id:"store-login".into(),text:"First@Test----Original123----https://code.example.test/current".into()}).await.unwrap();
 let (socket,_)=listener.accept().await.unwrap();let mut socket=TestStream::new(socket);
 assert_eq!(packet(&mut socket).await.0,33);reply(&mut socket,json!({"ok":true,"apple_password_change":1})).await;
 let (_,login)=packet(&mut socket).await;assert_eq!(login["method"],"signInAppStore");assert_eq!(login["params"]["password"],"Original123");
 reply(&mut socket,json!({"operation_id":"login-1","method":"signInAppStore","status":"completed","result":{"outcome":"succeeded","authenticated":true}})).await;
 assert_eq!(packet(&mut socket).await.0,33);reply(&mut socket,json!({"ok":true,"apple_password_change":1})).await;
 let (_,change)=packet(&mut socket).await;assert_eq!(change["method"],"changePassword");assert_eq!(change["params"]["password"],"Original123");
 let password=change["params"]["new_password"].as_str().unwrap().to_owned();assert_eq!(password.len(),8);
 // This read is before the simulated iOS mutation responds: durability precedes dispatch.
 let store=rv_core::workspace::WorkspaceStore::open(&paths).unwrap();let pending=store.apple_account("first@test").unwrap().unwrap();
 assert_eq!(pending.pending_password.as_deref(),Some(password.as_str()));assert_eq!(pending.password,"Original123");assert_eq!(pending.original_password,"Original123");
 reply(&mut socket,json!({"operation_id":"password-1","method":"changePassword","status":"completed","result":{"outcome":"succeeded","password_changed":true}})).await;
 assert_eq!(packet(&mut socket).await.0,33);reply(&mut socket,json!({"ok":true,"apple_password_change":1})).await;
 let (_,confirm)=packet(&mut socket).await;assert_eq!(confirm["method"],"signInAppStore");assert_eq!(confirm["params"]["password"],password);
 reply(&mut socket,json!({"operation_id":"login-2","method":"signInAppStore","status":"completed","result":{"outcome":"succeeded","authenticated":true}})).await;
 loop{if let FunctionEvent::State{record,..}=next(&mut states).await {if record["status"]=="completed"{assert_eq!(record["method"],"signInAppStore");assert_eq!(record["result"]["outcome"],"succeeded");break;}}}
 let saved=store.apple_account("first@test").unwrap().unwrap();assert_eq!(saved.password,password);assert_eq!(saved.original_password,"Original123");assert!(saved.pending_password.is_none());assert_eq!(saved.init_status,rv_core::workspace::AppleInitStatus::Ready);
 assert_eq!(saved.device_id.as_deref(),Some("device-a"));assert!(saved.first_login_at.is_some()&&saved.password_changed_at.is_some());
 // Reusing the same pasted original must not overwrite the confirmed password or rotate again.
 commands.send(FunctionCommand::Invoke{id:"store-login".into(),text:"FIRST@test----Original123".into()}).await.unwrap();
 assert_eq!(packet(&mut socket).await.0,33);reply(&mut socket,json!({"ok":true,"apple_password_change":1})).await;
 let (_,again)=packet(&mut socket).await;assert_eq!(again["params"]["password"],password);assert_eq!(again["method"],"signInAppStore");
 reply(&mut socket,json!({"operation_id":"login-3","method":"signInAppStore","status":"completed","result":{"outcome":"succeeded","authenticated":true}})).await;
 loop{if let FunctionEvent::State{record,..}=next(&mut states).await{if record["status"]=="completed"{break;}}}
 assert_eq!(store.apple_account("first@test").unwrap().unwrap().original_password,"Original123");
 drop(commands);actor.await.unwrap();
}

#[tokio::test]
async fn managed_unknown_password_change_preserves_both_passwords_and_does_not_retry(){
 let dir=tempfile::tempdir().unwrap();let paths=rv_core::StorePaths::in_dir(dir.path().into());let ctx=apple_accounts::Context::at(paths.clone(),None);
 let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let client=crate::phone_http::PhoneHttpClient::new("127.0.0.1",listener.local_addr().unwrap().port(),"test".into()).unwrap();
 let (commands,rx)=mpsc::channel(8);let (events,mut states)=mpsc::channel(32);let actor=tokio::spawn(apple_accounts::run(client,rx,events,ctx));
 let _=next(&mut states).await;commands.send(FunctionCommand::Invoke{id:"store-login".into(),text:"unknown@test----Original123".into()}).await.unwrap();
 let (socket,_)=listener.accept().await.unwrap();let mut socket=TestStream::new(socket);
 let _=packet(&mut socket).await;reply(&mut socket,json!({"ok":true,"apple_password_change":1})).await;
 let _=packet(&mut socket).await;reply(&mut socket,json!({"operation_id":"initial","method":"signInAppStore","status":"completed","result":{"outcome":"succeeded","authenticated":true}})).await;
 let _=packet(&mut socket).await;reply(&mut socket,json!({"ok":true,"apple_password_change":1})).await;
 let (_,change)=packet(&mut socket).await;let candidate=change["params"]["new_password"].as_str().unwrap().to_owned();
 reply(&mut socket,json!({"operation_id":"password-unknown","method":"changePassword","status":"completed","error":{"domain":"org.trollvnc.apple","code":6},"result":{"outcome":"unknown"}})).await;
 loop{if let FunctionEvent::State{record,..}=next(&mut states).await{if record["status"]=="completed"{break;}}}
 let store=rv_core::workspace::WorkspaceStore::open(&paths).unwrap();let account=store.apple_account("unknown@test").unwrap().unwrap();
 assert_eq!(account.password,"Original123");assert_eq!(account.pending_password.as_deref(),Some(candidate.as_str()));assert_eq!(account.init_status,rv_core::workspace::AppleInitStatus::Unknown);assert_eq!(account.password_change_operation_id.as_deref(),Some("password-unknown"));
 commands.send(FunctionCommand::Invoke{id:"store-login".into(),text:"unknown@test----Original123".into()}).await.unwrap();
 loop{if matches!(next(&mut states).await,FunctionEvent::Error(_)){break;}}
 let mut no_request=String::new();socket.read_line(&mut no_request).await.unwrap();
 // Recover the original operation; never issue another password mutation.
 commands.send(FunctionCommand::Resume{task:31,operation_id:"password-unknown".into()}).await.unwrap();
 let _=packet(&mut socket).await;reply(&mut socket,json!({"ok":true,"apple_password_change":1})).await;
 let (_,query)=packet(&mut socket).await;assert_eq!(query["action"],"result");assert_eq!(query["operation_id"],"password-unknown");
 reply(&mut socket,json!({"operation_id":"password-unknown","method":"changePassword","status":"completed","error":{"domain":"org.trollvnc.apple","code":11,"reason":"account_locked"},"result":{"outcome":"failed","password_changed":true}})).await;
 loop{if let FunctionEvent::State{record,..}=next(&mut states).await{if record["status"]=="completed"{break;}}}
 let recovered=store.apple_account("unknown@test").unwrap().unwrap();assert_eq!(recovered.password,candidate);assert_eq!(recovered.original_password,"Original123");assert!(recovered.pending_password.is_none());assert_eq!(recovered.account_status,rv_core::workspace::AppleAccountStatus::Locked);
 drop(commands);actor.await.unwrap();
}

#[tokio::test]
async fn managed_manual_change_preserves_ready_and_does_not_switch_other_media_account() {
    let dir=tempfile::tempdir().unwrap();let paths=rv_core::StorePaths::in_dir(dir.path().into());
    let mut store=rv_core::workspace::WorkspaceStore::open(&paths).unwrap();
    let a=store.prepare_apple_login("identity@test","Original123",None,None,&json!({})).unwrap().account;
    let a=store.apple_login_succeeded(&a).unwrap();let a=store.begin_apple_password_change(&a).unwrap();let ready=store.apple_password_change_succeeded(&a).unwrap();
    let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let client=crate::phone_http::PhoneHttpClient::new("127.0.0.1",listener.local_addr().unwrap().port(),"test".into()).unwrap();
    let (commands,rx)=mpsc::channel(8);let (events,mut states)=mpsc::channel(32);
    let actor=tokio::spawn(apple_accounts::run(client,rx,events,apple_accounts::Context::at(paths.clone(),None)));
    let _=next(&mut states).await;
    let mut connection=None;
    for succeed in [false,true] {
        commands.send(FunctionCommand::InvokeForAccount{id:"apple-password-change".into(),text:String::new(),apple_id:"identity@test".into()}).await.unwrap();
        // The first request uses the shared HTTP/2 transport for the read-only preflight.
        if connection.is_none(){let (stream,_)=listener.accept().await.unwrap();connection=Some(TestStream::new(stream));}
        let socket=connection.as_mut().unwrap();
        assert_eq!(packet(socket).await.0,33);reply(socket,json!({"ok":true,"apple_password_change":1})).await;
        let (_,query)=packet(socket).await;assert_eq!(query["method"],"queryAccountState");
        reply(socket,json!({"operation_id":"state","method":"queryAccountState","status":"completed","result":{
            "icloud":{"status":"not_signed_in","account":null},
            "identity":{"status":"signed_in","account":{"apple_id":"identity@test"}},
            "app_store":{"status":"signed_in","account":{"apple_id":"other-media@test","authenticated":true}}
        }})).await;
        assert_eq!(packet(socket).await.0,33);reply(socket,json!({"ok":true,"apple_password_change":1})).await;
        let (_,change)=packet(socket).await;
        assert_eq!(change["method"],"changePassword");assert_eq!(change["params"]["apple_id"],"identity@test");assert_eq!(change["params"]["password"],ready.password);
        let candidate=change["params"]["new_password"].as_str().unwrap().to_owned();
        let saved=store.apple_account("identity@test").unwrap().unwrap();assert_eq!(saved.pending_password.as_deref(),Some(candidate.as_str()));
        let result=if succeed {json!({"operation_id":"manual","method":"changePassword","status":"completed","result":{"outcome":"succeeded","password_changed":true}})} else {json!({"operation_id":"manual","method":"changePassword","status":"completed","error":{"domain":"org.trollvnc.apple","code":3},"result":{"outcome":"failed"}})};
        reply(socket,result).await;
        loop {if let FunctionEvent::State{record,..}=next(&mut states).await {if record["status"]=="completed"{assert_eq!(record["method"],"changePassword");break;}}}
        let saved=store.apple_account("identity@test").unwrap().unwrap();assert_eq!(saved.init_status,rv_core::workspace::AppleInitStatus::Ready);assert!(saved.pending_password.is_none());assert_eq!(saved.original_password,"Original123");
        assert_eq!(saved.password,if succeed {candidate} else {ready.password.clone()});
        // No Store login is requested before or after changing the identity password.
        assert!(tokio::time::timeout(Duration::from_millis(75),packet(socket)).await.is_err());
    }
    drop(commands);actor.await.unwrap();
}
