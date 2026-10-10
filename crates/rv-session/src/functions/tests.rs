use super::*;
use tokio::net::TcpListener;

async fn packet(stream: &mut BufReader<TcpStream>) -> (u8, Value) {
    let mut line = String::new();
    stream.read_line(&mut line).await.unwrap();
    let task = line[..2].parse().unwrap();
    let request = serde_json::from_slice(&STANDARD.decode(line[2..].trim()).unwrap()).unwrap();
    (task, request)
}
async fn reply(stream: &mut BufReader<TcpStream>, value: Value) {
    stream
        .get_mut()
        .write_all(format!("0;;{}\r\n", STANDARD.encode(value.to_string())).as_bytes())
        .await
        .unwrap();
}
async fn reconnect(
    listener: &TcpListener,
    mut previous: BufReader<TcpStream>,
) -> BufReader<TcpStream> {
    let mut line = String::new();
    assert_eq!(
        previous.read_line(&mut line).await.unwrap(),
        0,
        "client must close after each reply"
    );
    drop(previous);
    let (socket, _) = listener.accept().await.unwrap();
    let mut socket = BufReader::new(socket);
    let (task, auth) = packet(&mut socket).await;
    assert_eq!(task, 33);
    assert_eq!(auth["op"], 0);
    reply(&mut socket, json!({"version":1,"op":0,"id":0,"status":0})).await;
    socket
}
async fn next(events: &mut mpsc::Receiver<FunctionEvent>) -> FunctionEvent {
    tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn challenge_fetches_once_and_never_replays_login() {
    challenge_flow(false).await;
}
#[tokio::test]
async fn automatic_code_rejection_is_reported() {
    challenge_flow(true).await;
}
async fn challenge_flow(reject: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://127.0.0.1:{}/get?id=123",
        http.local_addr().unwrap().port()
    );
    let http_task = tokio::spawn(async move {
        let (mut stream, _) = http.accept().await.unwrap();
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
        version: 1,
        port: listener.local_addr().unwrap().port(),
        token: "a".repeat(64),
    };
    let (commands, rx) = mpsc::channel(8);
    let (events, mut states) = mpsc::channel(32);
    let actor = tokio::spawn(worker("127.0.0.1".into(), endpoint, rx, events));
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = BufReader::new(socket);
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
        socket = reconnect(&listener, socket).await;
        let (_, submit) = packet(&mut socket).await;
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
        version: 1,
        port: listener.local_addr().unwrap().port(),
        token: "b".repeat(64),
    };
    let (commands, rx) = mpsc::channel(8);
    let (events, mut states) = mpsc::channel(32);
    let actor = tokio::spawn(worker("127.0.0.1".into(), endpoint, rx, events));
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = BufReader::new(socket);
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
fn native_unknown_and_partial_shapes_remain_uncertain() {
    assert!(uncertain_result(
        31,
        &json!({"error":{"domain":"org.trollvnc.apple","code":6}})
    ));
    assert!(!uncertain_result(
        31,
        &json!({"error":{"domain":"Accounts","code":6}})
    ));
    assert!(uncertain_result(
        32,
        &json!({"error":{"domain":"TVAppStateServices","code":3}})
    ));
    assert!(uncertain_result(
        32,
        &json!({"error":{"state_may_be_partial":true}})
    ));
    assert!(uncertain_result(
        31,
        &json!({"error":{"code":"missing_result"}})
    ));
    assert!(accepted_code(&json!({"operation_id":"wrong","accepted":true}), "expected").is_err());
}

#[tokio::test]
async fn next_operation_uses_a_fresh_authenticated_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ControlEndpoint {
        version: 1,
        port: listener.local_addr().unwrap().port(),
        token: "a".repeat(64),
    };
    let (commands, rx) = mpsc::channel(8);
    let (events, mut states) = mpsc::channel(32);
    let actor = tokio::spawn(worker("127.0.0.1".into(), endpoint, rx, events));
    let peer = tokio::spawn(async move {
        for id in ["first", "second"] {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = BufReader::new(socket);
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
            // Simulate the phone closing an idle socket after completion.
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
        version: 1,
        port: listener.local_addr().unwrap().port(),
        token: "a".repeat(64),
    };
    let (commands, rx) = mpsc::channel(8);
    let (events, mut states) = mpsc::channel(32);
    let actor = tokio::spawn(worker("127.0.0.1".into(), endpoint, rx, events));
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = BufReader::new(socket);
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
        version: 1,
        port: listener.local_addr().unwrap().port(),
        token: "a".repeat(64),
    };
    let (commands, rx) = mpsc::channel(8);
    let (events, mut states) = mpsc::channel(32);
    let actor = tokio::spawn(worker("127.0.0.1".into(), endpoint, rx, events));
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = BufReader::new(socket);
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
        version: 1,
        port: listener.local_addr().unwrap().port(),
        token: "a".repeat(64),
    };
    let (commands, rx) = mpsc::channel(8);
    let (events, mut states) = mpsc::channel(32);
    let actor = tokio::spawn(worker("127.0.0.1".into(), endpoint, rx, events));
    let peer = tokio::spawn(async move {
        for method in [
            if resumed { "" } else { "queryAccountState" },
            "beginAddingContactEmail",
            "queryAccountState",
            "verifyContactEmail",
        ] {
            if method.is_empty() {
                continue;
            }
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = BufReader::new(socket);
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
            let mut line = String::new();
            assert_eq!(socket.read_line(&mut line).await.unwrap(), 0);
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
        version: 1,
        port: listener.local_addr().unwrap().port(),
        token: "a".repeat(64),
    };
    let (commands, rx) = mpsc::channel(8);
    let (events, mut states) = mpsc::channel(32);
    let actor = tokio::spawn(worker_with_provider(
        "127.0.0.1".into(),
        endpoint,
        rx,
        events,
        &MockMailboxProvider,
    ));
    let peer = tokio::spawn(async move {
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
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = BufReader::new(socket);
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
            let mut line = String::new();
            assert_eq!(socket.read_line(&mut line).await.unwrap(), 0);
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
async fn independent_function_finishes_while_another_socket_waits() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = ControlEndpoint {
        version: 1,
        port: listener.local_addr().unwrap().port(),
        token: "a".repeat(64),
    };
    let (entered, mut waiting) = mpsc::channel(1);
    let server = tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let entered = entered.clone();
            tokio::spawn(async move {
                let mut socket = BufReader::new(socket);
                let (task, _) = packet(&mut socket).await;
                assert_eq!(task, 33);
                reply(&mut socket, json!({"version":1,"op":0,"id":0,"status":0})).await;
                let (_, request) = packet(&mut socket).await;
                if request["method"] == "signInAppStore" {
                    entered.send(()).await.unwrap();
                    std::future::pending::<()>().await;
                } else if request["method"] == "queryAccountState" {
                    reply(&mut socket,json!({"operation_id":"account","method":"queryAccountState","status":"completed","result":{}})).await;
                } else {
                    assert_eq!(request["method"], "queryState");
                    reply(&mut socket,json!({"operation_id":"read","method":"queryState","status":"completed","result":{}})).await;
                }
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
