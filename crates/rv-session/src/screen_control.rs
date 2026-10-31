//! One HTTP operation per screen action, with bounded NDJSON progress and no replay.
use crate::{AppCommand, AppEvent, ScreenState, phone_http::PhoneHttpClient};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;

#[derive(Clone, PartialEq, Eq)]
pub struct ScreenPasscode(pub String);
impl Drop for ScreenPasscode {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.0.zeroize();
    }
}
impl std::fmt::Debug for ScreenPasscode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ScreenPasscode([redacted])")
    }
}
#[derive(Debug, Clone, Deserialize)]
pub struct ScreenProgress {
    pub phase: String,
    pub state: Option<ScreenState>,
    pub error: Option<String>,
}
impl ScreenProgress {
    fn terminal(&self) -> bool {
        matches!(self.phase.as_str(), "unlocked" | "locked" | "failed")
    }
}
pub(crate) struct ScreenOperation {
    pub attempt: u32,
    pub events: mpsc::Receiver<AppEvent>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for ScreenOperation {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl ScreenOperation {
    pub fn start(client: PhoneHttpClient, command: AppCommand) -> Result<Self, String> {
        let attempt = command.unlock_attempt().unwrap();
        let locking = matches!(command, AppCommand::HttpLock(_));
        let body = match command {
            AppCommand::HttpUnlock(_, code) => json!({"passcode":code.0}).to_string().into_bytes(),
            _ => b"{}".to_vec(),
        };
        let (sender, events) = mpsc::channel(16);
        let task = tokio::spawn(async move {
            let result = tokio::time::timeout(std::time::Duration::from_secs(18), async {
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert("content-type", "application/json".parse().unwrap());
                let mut response = client
                    .api()
                    .raw(
                        reqwest::Method::POST,
                        if locking {
                            "/api/device/lock"
                        } else {
                            "/api/device/unlock"
                        },
                        headers,
                        body,
                    )
                    .await
                    .map_err(|_| "屏幕操作请求中断；结果未知，请检查手机")?;
                if response.status() != reqwest::StatusCode::OK {
                    return Err("手机拒绝屏幕操作请求");
                }
                if !response
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| v.starts_with("application/x-ndjson"))
                {
                    return Err("屏幕操作回复格式错误");
                }
                let mut buffer = Vec::new();
                let mut total = 0;
                while let Some(chunk) = response
                    .chunk()
                    .await
                    .map_err(|_| "屏幕操作回复中断；结果未知，请检查手机")?
                {
                    total += chunk.len();
                    if total > 64 * 1024 || buffer.len() + chunk.len() > 16 * 1024 {
                        return Err("屏幕操作回复超过限制");
                    }
                    buffer.extend_from_slice(&chunk);
                    while let Some(end) = buffer.iter().position(|&b| b == b'\n') {
                        let progress: ScreenProgress = serde_json::from_slice(&buffer[..end])
                            .map_err(|_| "屏幕操作回复格式错误")?;
                        buffer.drain(..=end);
                        if !matches!(
                            progress.phase.as_str(),
                            "preparing"
                                | "input_ready"
                                | "entering"
                                | "checking"
                                | "locking"
                                | "locked"
                                | "unlocked"
                                | "failed"
                        ) {
                            return Err("未知屏幕操作状态");
                        }
                        let terminal = progress.terminal();
                        if terminal && progress.phase != "failed" {
                            let expected = if locking { "locked" } else { "unlocked" };
                            if progress.phase != expected
                                || progress.state.as_ref().is_none_or(|s| s.locked != locking)
                            {
                                return Err("屏幕操作最终状态不一致");
                            }
                        }
                        sender
                            .send(AppEvent::ScreenProgress(attempt, progress))
                            .await
                            .map_err(|_| "屏幕操作已取消")?;
                        if terminal {
                            return Ok(());
                        }
                    }
                }
                Err("屏幕操作回复提前结束；结果未知，请检查手机")
            })
            .await
            .unwrap_or(Err("屏幕操作超时；结果未知，请检查手机"));
            if let Err(message) = result {
                // Keep the passcode out of retained events.
                let command = if locking {
                    AppCommand::HttpLock(attempt)
                } else {
                    AppCommand::HttpUnlock(attempt, ScreenPasscode(String::new()))
                };
                let _ = sender
                    .send(AppEvent::Failed {
                        command,
                        message: message.into(),
                    })
                    .await;
            }
        });
        Ok(Self {
            attempt,
            events,
            task,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::sync::oneshot;
    async fn fixture(
        mode: &'static str,
    ) -> (
        ScreenOperation,
        tokio::task::JoinHandle<()>,
        Arc<AtomicUsize>,
        oneshot::Receiver<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        let (cancelled, reset) = oneshot::channel();
        let mut cancelled = Some(cancelled);
        let peer = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut connection = h2::server::handshake(socket).await.unwrap();
            while let Some(request) = connection.accept().await {
                let (request, mut respond) = request.unwrap();
                seen.fetch_add(1, Ordering::SeqCst);
                let cancelled = cancelled.take();
                tokio::spawn(async move {
                    assert_eq!(
                        request.uri().path(),
                        if mode == "lock-success" {
                            "/api/device/lock"
                        } else {
                            "/api/device/unlock"
                        }
                    );
                    assert_eq!(request.headers()["authorization"], "Bearer fixture-token");
                    let mut body = request.into_body();
                    let mut bytes = Vec::new();
                    while let Some(chunk) = body.data().await {
                        let chunk = chunk.unwrap();
                        body.flow_control().release_capacity(chunk.len()).unwrap();
                        bytes.extend_from_slice(&chunk);
                    }
                    assert_eq!(
                        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
                        if mode == "lock-success" {
                            json!({})
                        } else {
                            json!({"passcode":"123456"})
                        }
                    );
                    let response = hyper::Response::builder()
                        .header("content-type", "application/x-ndjson")
                        .body(())
                        .unwrap();
                    let mut stream = respond.send_response(response, false).unwrap();
                    // A JSON event spans DATA frames; progress must not wait for EOF.
                    stream
                        .send_data(hyper::body::Bytes::from_static(b"{\"phase\":\"prep"), false)
                        .unwrap();
                    stream
                        .send_data(hyper::body::Bytes::from_static(b"aring\"}\n"), false)
                        .unwrap();
                    if mode == "cancel" {
                        std::future::poll_fn(|cx| stream.poll_reset(cx))
                            .await
                            .unwrap();
                        cancelled.unwrap().send(()).unwrap();
                    } else {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        let data = match mode {
                            "lock-success" => b"{\"phase\":\"locked\",\"state\":{\"locked\":true,\"passcode_required\":true,\"input_ready\":false,\"input_empty\":false}}\n".as_slice(),
                            "success" => b"{\"phase\":\"unlocked\",\"state\":{\"locked\":false,\"passcode_required\":true,\"input_ready\":false,\"input_empty\":false}}\n".as_slice(),
                            "wrong-state" => b"{\"phase\":\"unlocked\",\"state\":{\"locked\":true,\"passcode_required\":true,\"input_ready\":false,\"input_empty\":false}}\n".as_slice(),
                            _ => b"".as_slice(),
                        };
                        stream
                            .send_data(hyper::body::Bytes::copy_from_slice(data), true)
                            .unwrap();
                    }
                });
            }
        });
        let operation = ScreenOperation::start(
            PhoneHttpClient::new("127.0.0.1", port, "fixture-token".into()).unwrap(),
            if mode == "lock-success" {
                AppCommand::HttpLock(7)
            } else {
                AppCommand::HttpUnlock(7, ScreenPasscode("123456".into()))
            },
        )
        .unwrap();
        (operation, peer, count, reset)
    }
    #[tokio::test]
    async fn lock_request_has_no_passcode_and_finishes_with_observed_locked_state() {
        let (mut operation, peer, count, _) = fixture("lock-success").await;
        assert!(matches!(
            operation.events.recv().await.unwrap(),
            AppEvent::ScreenProgress(7, _)
        ));
        assert!(
            matches!(operation.events.recv().await.unwrap(), AppEvent::ScreenProgress(7, p) if p.phase == "locked" && p.state.as_ref().unwrap().locked)
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
        drop(operation);
        peer.abort();
    }
    #[tokio::test]
    async fn fragmented_progress_finishes_only_with_observed_unlocked_state() {
        let (mut operation, peer, count, _) = fixture("success").await;
        assert!(
            matches!(operation.events.recv().await.unwrap(), AppEvent::ScreenProgress(7, p) if p.phase == "preparing")
        );
        assert!(
            matches!(operation.events.recv().await.unwrap(), AppEvent::ScreenProgress(7, p) if p.phase == "unlocked" && !p.state.as_ref().unwrap().locked)
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
        drop(operation);
        peer.abort();
    }
    #[tokio::test]
    async fn disconnect_and_inconsistent_state_are_unknown_without_replay() {
        for mode in ["eof", "wrong-state"] {
            let (mut operation, peer, count, _) = fixture(mode).await;
            assert!(matches!(
                operation.events.recv().await.unwrap(),
                AppEvent::ScreenProgress(7, _)
            ));
            let event = operation.events.recv().await.unwrap();
            assert!(matches!(event, AppEvent::Failed { .. }));
            assert!(!format!("{event:?}").contains("123456"));
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            assert_eq!(count.load(Ordering::SeqCst), 1);
            drop(operation);
            peer.abort();
        }
    }
    #[tokio::test]
    async fn dropping_operation_resets_http_stream() {
        let (mut operation, peer, count, reset) = fixture("cancel").await;
        assert!(matches!(
            operation.events.recv().await.unwrap(),
            AppEvent::ScreenProgress(7, _)
        ));
        drop(operation);
        tokio::time::timeout(std::time::Duration::from_secs(2), reset)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
        peer.abort();
    }
}
