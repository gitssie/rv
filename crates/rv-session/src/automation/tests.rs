use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

use http_body_util::{BodyExt, Full};
use hyper::{
    Request, Response,
    body::{Bytes, Incoming},
    service::service_fn,
};
use hyper_util::rt::{TokioExecutor, TokioIo};
use rv_automation::{
    Action, FailureReason, InputValues, RunControl, RunState, Runner, Step, Target, TextMatch,
    TextSelector, Workflow,
};

use super::*;

const APP: &str = "local.fixture.app";
fn ocr(text: &str) -> Value {
    json!({"value":format!("{text},,100,,200,,200,,40"),"frame":{"width":1000,"height":2000,"orientation":0}})
}
pub(super) struct Expected {
    pub(super) path: &'static str,
    pub(super) status: u16,
    pub(super) body: Option<Value>,
    pub(super) reply: Value,
    pub(super) pause: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
}
pub(super) fn response(path: &'static str, reply: Value) -> Expected {
    Expected {
        path,
        status: 200,
        body: None,
        reply,
        pause: None,
    }
}
fn observe(text: &str) -> Vec<Expected> {
    vec![
        response("/api/apps/foreground", json!({"bundle_id":APP})),
        response("/api/device/ocr", ocr(text)),
        response("/api/apps/foreground", json!({"bundle_id":APP})),
    ]
}
pub(super) fn post_paste_replies(bundle: &str) -> Vec<Expected> {
    vec![
        response("/api/apps/foreground", json!({"bundle_id":bundle})),
        response("/api/device/tap", json!({"ok":true})),
        response("/api/apps/foreground", json!({"bundle_id":bundle})),
        response("/api/device/tap", json!({"ok":true})),
    ]
}

type Calls = Arc<Mutex<Vec<String>>>;
type Pending = Arc<Mutex<VecDeque<Expected>>>;
pub(super) async fn fixture(
    expected: Vec<Expected>,
) -> (ZxTouchDevice, Calls, Pending, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let pending = Arc::new(Mutex::new(VecDeque::from(expected)));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let received = calls.clone();
    let replies = pending.clone();
    let server = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let received = received.clone();
            let replies = replies.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request: Request<Incoming>| {
                    let received = received.clone();
                    let replies = replies.clone();
                    async move {
                        assert_eq!(request.version(), hyper::Version::HTTP_2);
                        assert_eq!(request.method(), hyper::Method::POST);
                        assert_eq!(
                            request.headers()["authorization"],
                            "Bearer fixture.token.value"
                        );
                        let path = request.uri().path().to_owned();
                        received.lock().unwrap().push(path.clone());
                        let bytes = request.into_body().collect().await.unwrap().to_bytes();
                        let payload: Value = serde_json::from_slice(&bytes).unwrap();
                        if let Some(hold) = payload.get("hold_ms").and_then(Value::as_u64) {
                            assert!((900..=1100).contains(&hold) || (100..=120).contains(&hold));
                            assert!(payload["x"].as_u64().is_some_and(|x| x < 1000));
                            assert!(payload["y"].as_u64().is_some_and(|y| y < 2000));
                        }
                        let next = replies
                            .lock()
                            .unwrap()
                            .pop_front()
                            .expect("unexpected replay/request");
                        assert_eq!(path, next.path);
                        if let Some(body) = next.body {
                            assert_eq!(payload, body);
                        }
                        if let Some((started, resume)) = next.pause {
                            started.notify_one();
                            resume.notified().await;
                        }
                        Ok::<_, std::convert::Infallible>(
                            Response::builder()
                                .status(next.status)
                                .body(Full::new(Bytes::from(next.reply.to_string())))
                                .unwrap(),
                        )
                    }
                });
                let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                    .serve_connection(TokioIo::new(socket), service)
                    .await;
            });
        }
    });
    let client = PhoneHttpClient::new("127.0.0.1", port, "fixture.token.value".into()).unwrap();
    (ZxTouchDevice::new(client), calls, pending, server)
}
fn selector(text: &str) -> TextSelector {
    TextSelector {
        text: text.into(),
        mode: TextMatch::Exact,
        case_sensitive: true,
        region: None,
    }
}
fn flow(actions: Vec<Action>) -> Workflow {
    Workflow {
        version: 1,
        bundle_id: APP.into(),
        steps: actions
            .into_iter()
            .enumerate()
            .map(|(i, action)| Step {
                id: format!("step_{i}"),
                timeout_ms: 2000,
                action,
            })
            .collect(),
    }
}
fn tap() -> Action {
    Action::Tap {
        target: Target::Text {
            selector: selector("Email"),
            offset: Point { x: 0., y: 0. },
        },
    }
}

#[tokio::test]
async fn complete_engine_uses_authenticated_http2_and_observable_postcondition() {
    let mut replies = observe("Email");
    replies.extend(observe("Email"));
    replies.push(Expected {
        body: Some(
            json!({"x":200.0,"y":220.0,"frame":{"width":1000,"height":2000,"orientation":0}}),
        ),
        ..response("/api/device/tap", json!({"ok":true}))
    });
    replies.extend(observe("Email"));
    replies.push(response(
        "/api/control/authorize",
        json!({"keyboard_literal":1,"device_tap_hold":1}),
    ));
    replies.push(response("/api/apps/foreground", json!({"bundle_id":APP})));
    replies.push(Expected {
        body: Some(json!({"fields":["8","fixture@example.invalid"]})),
        ..response("/api/device/keyboard", json!({"value":""}))
    });
    replies.extend(post_paste_replies(APP));
    replies.extend(observe("Complete"));
    let (device, calls, pending, server) = fixture(replies).await;
    let mut inputs = InputValues::default();
    inputs.insert("email", "fixture@example.invalid".into());
    let result = Runner::new(device)
        .run(
            &flow(vec![
                Action::WaitForText {
                    selector: selector("Email"),
                    present: true,
                },
                tap(),
                Action::InputText {
                    input: "email".into(),
                },
                Action::WaitForText {
                    selector: selector("Complete"),
                    present: true,
                },
            ]),
            &inputs,
            &RunControl::default(),
        )
        .await;
    assert_eq!(result.state, RunState::Succeeded);
    assert_eq!(result.completed_steps, 4);
    assert!(pending.lock().unwrap().is_empty());
    assert_eq!(calls.lock().unwrap().len(), 20);
    server.abort();
}

#[tokio::test]
async fn stale_frame_is_not_retried_and_stops_the_workflow() {
    let mut replies = observe("Email");
    replies.push(response(
        "/api/device/tap",
        json!({"error":"frame_changed"}),
    ));
    let (device, calls, pending, server) = fixture(replies).await;
    let result = Runner::new(device)
        .run(
            &flow(vec![tap(), tap()]),
            &InputValues::default(),
            &RunControl::default(),
        )
        .await;
    assert_eq!(result.state, RunState::Failed);
    assert_eq!(
        result.failure.unwrap().reason,
        FailureReason::DeviceRejected
    );
    assert!(pending.lock().unwrap().is_empty());
    assert_eq!(calls.lock().unwrap().len(), 4);
    server.abort();
}

#[tokio::test]
async fn app_change_during_ocr_prevents_tap() {
    let mut replies = observe("Email");
    replies[2].reply = json!({"bundle_id":"other.app"});
    let (device, calls, _, server) = fixture(replies).await;
    let result = Runner::new(device)
        .run(
            &flow(vec![tap()]),
            &InputValues::default(),
            &RunControl::default(),
        )
        .await;
    assert_eq!(result.state, RunState::Failed);
    assert_eq!(calls.lock().unwrap().len(), 3);
    server.abort();
}

#[tokio::test]
async fn malformed_keyboard_ack_is_unknown_and_not_replayed() {
    let mut replies = observe("Email");
    replies.push(response(
        "/api/control/authorize",
        json!({"keyboard_literal":1,"device_tap_hold":1}),
    ));
    replies.push(response(
        "/api/device/keyboard",
        json!({"unexpected":"response"}),
    ));
    let (device, calls, _, server) = fixture(replies).await;
    let mut inputs = InputValues::default();
    inputs.insert("email", "fixture@example.invalid".into());
    let result = Runner::new(device)
        .run(
            &flow(vec![
                Action::InputText {
                    input: "email".into(),
                },
                tap(),
            ]),
            &inputs,
            &RunControl::default(),
        )
        .await;
    assert_eq!(result.state, RunState::OutcomeUnknown);
    assert_eq!(calls.lock().unwrap().len(), 5);
    server.abort();
}

#[tokio::test]
async fn supports_both_daemon_and_foreground_adapter_keyboard_acknowledgements() {
    for acknowledgement in [json!({"value":""}), json!({"ok":true})] {
        let (mut device, calls, _, server) = fixture(vec![
            response("/api/control/authorize", json!({"keyboard_literal":1})),
            response("/api/device/keyboard", acknowledgement),
        ])
        .await;
        assert_eq!(device.input_text("fixture").await, Ok(()));
        assert_eq!(calls.lock().unwrap().len(), 2);
        server.abort();
    }
}

#[tokio::test]
async fn keyboard_http400_after_dispatch_remains_unknown_and_stops_later_actions() {
    let mut replies = observe("Email");
    replies.push(response(
        "/api/control/authorize",
        json!({"keyboard_literal":1,"device_tap_hold":1}),
    ));
    replies.push(Expected {
        status: 400,
        ..response(
            "/api/device/keyboard",
            json!({"error":"App adapter timed out"}),
        )
    });
    let (device, calls, _, server) = fixture(replies).await;
    let mut inputs = InputValues::default();
    inputs.insert("email", "fixture@example.invalid".into());
    let result = Runner::new(device)
        .run(
            &flow(vec![
                Action::InputText {
                    input: "email".into(),
                },
                tap(),
            ]),
            &inputs,
            &RunControl::default(),
        )
        .await;
    assert_eq!(result.state, RunState::OutcomeUnknown);
    assert!(result.failure.unwrap().mutation_may_have_executed);
    assert_eq!(calls.lock().unwrap().len(), 5);
    server.abort();
}

#[tokio::test]
async fn interrupted_keyboard_request_never_reconnects_to_replay() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut connection = h2::server::handshake(socket).await.unwrap();
        let (request, mut reply) = connection.accept().await.unwrap().unwrap();
        assert_eq!(request.uri().path(), "/api/control/authorize");
        let mut stream = reply.send_response(Response::new(()), false).unwrap();
        stream
            .send_data(Bytes::from_static(b"{\"keyboard_literal\":1}"), true)
            .unwrap();
        let (request, _) = connection.accept().await.unwrap().unwrap();
        assert_eq!(request.uri().path(), "/api/device/keyboard");
        drop(connection);
        assert!(
            tokio::time::timeout(Duration::from_millis(200), listener.accept())
                .await
                .is_err()
        );
    });
    let mut device = ZxTouchDevice::new(
        PhoneHttpClient::new("127.0.0.1", port, "fixture.token.value".into()).unwrap(),
    );
    assert_eq!(
        device.input_text("fixture").await,
        Err(DeviceError::OutcomeUnknown)
    );
    server.await.unwrap();
}

#[tokio::test]
async fn missing_literal_capability_never_dispatches_physical_or_clipboard_fallback() {
    let (mut device, calls, _, server) = fixture(vec![response(
        "/api/control/authorize",
        json!({"device_tap":1}),
    )])
    .await;
    assert_eq!(
        device.input_text("fixture").await,
        Err(DeviceError::Rejected)
    );
    assert_eq!(*calls.lock().unwrap(), vec!["/api/control/authorize"]);
    server.abort();
}

#[test]
fn validates_ocr_coordinates_and_requires_frame_metadata() {
    let valid = ocr("Email");
    let observed = parse_observation(&valid, Some(APP.into())).unwrap();
    assert_eq!(
        observed.elements[0].bounds.center(),
        Point { x: 200., y: 220. }
    );
    for invalid in [
        json!({"value":"Email,,1,,2,,3,,4"}),
        json!({"value":"Email,,NaN,,2,,3,,4","frame":valid["frame"]}),
        json!({"value":"Email,,-1,,2,,3,,4","frame":valid["frame"]}),
        json!({"value":"Email,,900,,2,,300,,4","frame":valid["frame"]}),
        json!({"value":"Email;;unescaped,,1,,2,,3,,4","frame":valid["frame"]}),
        json!({"value":"Email,,1,,2,,3,,4","frame":{"width":1000,"height":2000,"orientation":4}}),
    ] {
        assert!(matches!(
            parse_observation(&invalid, None),
            Err(DeviceError::InvalidResponse)
        ));
    }
    let empty = json!({"value":"","frame":valid["frame"]});
    assert!(parse_observation(&empty, None).unwrap().elements.is_empty());
}

#[tokio::test]
async fn cancellation_during_long_hold_does_not_send_short_tap_or_replay_text() {
    let started = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let (mut device, calls, pending, server) = fixture(vec![
        response(
            "/api/control/authorize",
            json!({"keyboard_literal":1,"device_tap_hold":1}),
        ),
        response("/api/apps/foreground", json!({"bundle_id":APP})),
        response("/api/device/keyboard", json!({"value":""})),
        response("/api/apps/foreground", json!({"bundle_id":APP})),
        Expected {
            pause: Some((started.clone(), resume.clone())),
            ..response("/api/device/tap", json!({"ok":true}))
        },
    ])
    .await;
    device.last_tap = Some((
        Frame {
            width: 1000,
            height: 2000,
            orientation: 0,
        },
        Point { x: 200., y: 220. },
    ));
    {
        let input = device.input_text("fixture");
        tokio::pin!(input);
        tokio::select! {
            _ = &mut input => panic!("long hold must remain pending"),
            _ = started.notified() => {}
        }
        // Dropping the in-flight input is how Runner cancels a mutation.
    }
    resume.notify_one();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(pending.lock().unwrap().is_empty());
    let calls = calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|p| p.as_str() == "/api/device/keyboard")
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|p| p.as_str() == "/api/device/tap")
            .count(),
        1
    );
    server.abort();
}

#[tokio::test]
async fn missing_timed_tap_capability_rejects_before_sending_text() {
    let (mut device, calls, _, server) = fixture(vec![response(
        "/api/control/authorize",
        json!({"keyboard_literal":1}),
    )])
    .await;
    device.last_tap = Some((
        Frame {
            width: 1000,
            height: 2000,
            orientation: 0,
        },
        Point { x: 200., y: 220. },
    ));
    assert_eq!(
        device.input_text("fixture").await,
        Err(DeviceError::Rejected)
    );
    assert_eq!(*calls.lock().unwrap(), vec!["/api/control/authorize"]);
    server.abort();
}
