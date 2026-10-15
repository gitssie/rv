//! Independent control connection. Never replay a submitted mutation on VNC.
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use vnc::device::{DeviceReply, DeviceRequest};

#[derive(Clone, Deserialize)]
pub struct ControlEndpoint {
    pub version: u8,
    pub port: u16,
    pub token: String,
}
impl std::fmt::Debug for ControlEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlEndpoint")
            .field("version", &self.version)
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}
pub(crate) enum ControlEvent {
    Ready,
    Unavailable,
    Reply(DeviceReply),
}
pub(crate) struct ControlChannel {
    pub requests: mpsc::Sender<DeviceRequest>,
    pub events: mpsc::Receiver<ControlEvent>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for ControlChannel {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl ControlChannel {
    #[cfg(test)]
    pub fn start(host: String, endpoint: ControlEndpoint) -> Self {
        Self::start_with_client(
            crate::phone_http::PhoneHttpClient::new(&host, endpoint.port, endpoint.token).unwrap(),
        )
    }
    pub fn start_with_client(mut client: crate::phone_http::PhoneHttpClient) -> Self {
        let (requests, mut incoming) = mpsc::channel::<DeviceRequest>(8);
        let (outgoing, events) = mpsc::channel(16);
        let task = tokio::spawn(async move {
            loop {
                let mut failed = false;
                let mut delay = Duration::from_millis(500);
                loop {
                    if matches!(
                        client.json("/api/control/authorize", &json!({})).await,
                        Ok((200, _))
                    ) {
                        break;
                    }
                    if !failed {
                        if outgoing.send(ControlEvent::Unavailable).await.is_err() {
                            return;
                        }
                        failed = true;
                    }
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(5));
                }
                if outgoing.send(ControlEvent::Ready).await.is_err() {
                    return;
                }
                loop {
                    let Some(request) = incoming.recv().await else {
                        return;
                    };
                    match tokio::time::timeout(
                        Duration::from_secs(12),
                        exchange(&mut client, &request),
                    )
                    .await
                    {
                        Ok(Ok(reply)) => {
                            if outgoing.send(ControlEvent::Reply(reply)).await.is_err() {
                                return;
                            }
                        }
                        _ => {
                            if outgoing.send(ControlEvent::Reply(DeviceReply {
                                op:request.op, id:request.id, status:1,
                                payload:json!({"error":"Control connection interrupted; operation result unknown. The operation will not be replayed."}).to_string(),
                            })).await.is_err() { return; }
                            // Queued operations have not been submitted. Fail them rather
                            // than sending stale intent after the connection recovers.
                            while let Ok(request) = incoming.try_recv() {
                                let _ = outgoing.send(ControlEvent::Reply(DeviceReply {
                                    op:request.op, id:request.id, status:1,
                                    payload:json!({"error":"Control connection interrupted; queued operation was not sent"}).to_string(),
                                })).await;
                            }
                            if outgoing.send(ControlEvent::Unavailable).await.is_err() {
                                return;
                            }
                            break;
                        }
                    }
                }
            }
        });
        Self {
            requests,
            events,
            task,
        }
    }
}

async fn exchange(
    client: &mut crate::phone_http::PhoneHttpClient,
    request: &DeviceRequest,
) -> std::io::Result<DeviceReply> {
    let path = crate::phone_http::app_route(request.op)
        .ok_or_else(|| std::io::Error::other("Unsupported control operation"))?;
    let payload: Value = serde_json::from_str(&request.payload).map_err(std::io::Error::other)?;
    if request.payload.len() > 4096 {
        return Err(std::io::Error::other("Control request too large"));
    }
    let (status, value) = client
        .json(path, &payload)
        .await
        .map_err(std::io::Error::other)?;
    if status == 401 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Control authorization expired",
        ));
    }
    Ok(DeviceReply {
        op: request.op,
        id: request.id,
        status: if status == 200 { 0 } else { 1 },
        payload: value.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn interrupted_mutation_is_not_replayed_but_later_operation_recovers() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = ControlEndpoint {
            version: 2,
            port: listener.local_addr().unwrap().port(),
            token: "a".repeat(64),
        };
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut connection = h2::server::handshake(socket).await.unwrap();
            let (request, mut reply) = connection.accept().await.unwrap().unwrap();
            assert_eq!(request.uri().path(), "/api/control/authorize");
            let mut stream = reply
                .send_response(hyper::Response::new(()), false)
                .unwrap();
            stream
                .send_data(hyper::body::Bytes::from_static(b"{\"ok\":true}"), true)
                .unwrap();
            let (request, _) = connection.accept().await.unwrap().unwrap();
            assert_eq!(request.uri().path(), "/api/apps/launch");
            drop(connection); // Submitted mutation has an unknown result.
            let (socket, _) = listener.accept().await.unwrap();
            let mut connection = h2::server::handshake(socket).await.unwrap();
            let (request, mut reply) = connection.accept().await.unwrap().unwrap();
            assert_eq!(request.uri().path(), "/api/control/authorize");
            let mut stream = reply
                .send_response(hyper::Response::new(()), false)
                .unwrap();
            stream
                .send_data(hyper::body::Bytes::from_static(b"{\"ok\":true}"), true)
                .unwrap();
            let (request, mut reply) = connection.accept().await.unwrap().unwrap();
            assert_eq!(
                request.uri().path(),
                "/api/apps/foreground",
                "must not replay the previous launch"
            );
            let mut stream = reply
                .send_response(hyper::Response::new(()), false)
                .unwrap();
            stream
                .send_data(
                    hyper::body::Bytes::from_static(b"{\"bundle_id\":null}"),
                    true,
                )
                .unwrap();
            while connection.accept().await.is_some() {}
        });
        let mut channel = ControlChannel::start("127.0.0.1".into(), endpoint);
        async fn next(channel: &mut ControlChannel) -> ControlEvent {
            tokio::time::timeout(Duration::from_secs(3), channel.events.recv())
                .await
                .unwrap()
                .unwrap()
        }
        assert!(matches!(next(&mut channel).await, ControlEvent::Ready));
        channel
            .requests
            .send(DeviceRequest {
                op: 2,
                id: 1,
                payload: "{\"bundle_id\":\"com.example.app\"}".into(),
            })
            .await
            .unwrap();
        assert!(
            matches!(next(&mut channel).await, ControlEvent::Reply(r) if r.id==1 && r.status==1 && r.payload.contains("unknown"))
        );
        assert!(matches!(
            next(&mut channel).await,
            ControlEvent::Unavailable
        ));
        assert!(matches!(next(&mut channel).await, ControlEvent::Ready));
        channel
            .requests
            .send(DeviceRequest {
                op: 5,
                id: 2,
                payload: "{}".into(),
            })
            .await
            .unwrap();
        assert!(
            matches!(next(&mut channel).await, ControlEvent::Reply(r) if r.id==2 && r.status==0)
        );
        drop(channel);
        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn expired_jwt_discards_queued_mutations_before_renewal() {
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let renewed = Arc::new(AtomicBool::new(false));
        let paths = Arc::new(Mutex::new(Vec::new()));
        let server_paths = paths.clone();
        let server_renewed = renewed.clone();
        let server = tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let (paths, renewed) = (server_paths.clone(), server_renewed.clone());
                tokio::spawn(async move {
                    let service = hyper::service::service_fn(
                        move |request: hyper::Request<hyper::body::Incoming>| {
                            let (paths, renewed) = (paths.clone(), renewed.clone());
                            async move {
                                let path = request.uri().path();
                                paths.lock().unwrap().push(path.to_owned());
                                let (status, value) = match path {
                                    "/api/control/connect" => {
                                        renewed.store(true, Ordering::SeqCst);
                                        (
                                            200,
                                            json!({"access_token":"header.payload.new","token_type":"Bearer","expires_in":3600,
                                        "capabilities":{"zx_control":{"version":3,"port":port,"token":"header.payload.new"}}}),
                                        )
                                    }
                                    "/api/control/authorize" => (200, json!({"ok":true})),
                                    "/api/apps/launch" => (401, json!({"error":"expired"})),
                                    "/api/apps/foreground" => {
                                        assert!(renewed.load(Ordering::SeqCst));
                                        assert_eq!(
                                            request.headers()["authorization"],
                                            "Bearer header.payload.new"
                                        );
                                        (200, json!({"bundle_id":null}))
                                    }
                                    _ => panic!("stale mutation must not reach server: {path}"),
                                };
                                Ok::<_, std::convert::Infallible>(
                                    hyper::Response::builder()
                                        .status(status)
                                        .body(http_body_util::Full::new(hyper::body::Bytes::from(
                                            value.to_string(),
                                        )))
                                        .unwrap(),
                                )
                            }
                        },
                    );
                    let _ = hyper::server::conn::http2::Builder::new(
                        hyper_util::rt::TokioExecutor::new(),
                    )
                    .serve_connection(hyper_util::rt::TokioIo::new(socket), service)
                    .await;
                });
            }
        });
        let client =
            crate::phone_http::PhoneHttpClient::new("127.0.0.1", port, "header.payload.old".into())
                .unwrap();
        let mut channel = ControlChannel::start_with_client(client.clone());
        async fn next(channel: &mut ControlChannel) -> ControlEvent {
            tokio::time::timeout(Duration::from_secs(3), channel.events.recv())
                .await
                .unwrap()
                .unwrap()
        }
        assert!(matches!(next(&mut channel).await, ControlEvent::Ready));
        channel
            .requests
            .send(DeviceRequest {
                op: 2,
                id: 1,
                payload: json!({"bundle_id":"fr.vinted"}).to_string(),
            })
            .await
            .unwrap();
        channel
            .requests
            .send(DeviceRequest {
                op: 4,
                id: 2,
                payload: json!({"bundle_id":"fr.vinted"}).to_string(),
            })
            .await
            .unwrap();
        assert!(
            matches!(next(&mut channel).await, ControlEvent::Reply(reply) if reply.id==1 && reply.status==1)
        );
        assert!(
            matches!(next(&mut channel).await, ControlEvent::Reply(reply) if reply.id==2 && reply.status==1)
        );
        assert!(matches!(
            next(&mut channel).await,
            ControlEvent::Unavailable
        ));
        client.connect("fixture").await.unwrap();
        assert!(matches!(next(&mut channel).await, ControlEvent::Ready));
        channel
            .requests
            .send(DeviceRequest {
                op: 5,
                id: 3,
                payload: "{}".into(),
            })
            .await
            .unwrap();
        assert!(
            matches!(next(&mut channel).await, ControlEvent::Reply(reply) if reply.id==3 && reply.status==0)
        );
        assert_eq!(
            paths
                .lock()
                .unwrap()
                .iter()
                .filter(|p| *p == "/api/apps/launch")
                .count(),
            1
        );
        assert!(
            !paths
                .lock()
                .unwrap()
                .iter()
                .any(|p| p == "/api/apps/restart")
        );
        drop(channel);
        server.abort();
    }
    #[tokio::test]
    async fn initial_control_failure_recovers_and_operations_reuse_connection() {
        let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = reserved.local_addr().unwrap().port();
        drop(reserved);
        let endpoint = ControlEndpoint {
            version: 2,
            port,
            token: "a".repeat(64),
        };
        let mut channel = ControlChannel::start("127.0.0.1".into(), endpoint);
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), channel.events.recv())
                .await
                .unwrap(),
            Some(ControlEvent::Unavailable)
        ));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .unwrap();
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let connections = count.clone();
        let server = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                connections.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                tokio::spawn(async move {
                    let handler = hyper::service::service_fn(
                        |request: hyper::Request<hyper::body::Incoming>| async move {
                            let body = if request.uri().path() == "/api/apps/foreground" {
                                "{\"bundle_id\":null}"
                            } else {
                                "{\"ok\":true}"
                            };
                            Ok::<_, std::convert::Infallible>(hyper::Response::new(
                                http_body_util::Full::new(hyper::body::Bytes::from(body)),
                            ))
                        },
                    );
                    let _ = hyper::server::conn::http2::Builder::new(
                        hyper_util::rt::TokioExecutor::new(),
                    )
                    .serve_connection(hyper_util::rt::TokioIo::new(stream), handler)
                    .await;
                });
            }
        });
        assert!(
            matches!(
                tokio::time::timeout(Duration::from_secs(6), channel.events.recv())
                    .await
                    .unwrap(),
                Some(ControlEvent::Ready)
            ),
            "first connection failure must not permanently stop control"
        );
        for id in 1..=2 {
            channel
                .requests
                .send(DeviceRequest {
                    op: 5,
                    id,
                    payload: "{}".into(),
                })
                .await
                .unwrap();
            let event = tokio::time::timeout(Duration::from_secs(2), channel.events.recv())
                .await
                .unwrap();
            assert!(
                matches!(event, Some(ControlEvent::Reply(reply)) if reply.id==id && reply.status==0)
            );
        }
        assert_eq!(
            count.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "operations must share the HTTP/2 connection"
        );
        drop(channel);
        server.abort();
    }
}
