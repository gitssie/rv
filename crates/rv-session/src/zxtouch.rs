//! Independent control connection. Never replay a submitted mutation on VNC.
use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
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
    pub fn start(host: String, endpoint: ControlEndpoint) -> Self {
        let (requests, mut incoming) = mpsc::channel::<DeviceRequest>(8);
        let (outgoing, events) = mpsc::channel(16);
        let task = tokio::spawn(async move {
            // The worker persists; TCP connections do not. Probe authorization once,
            // then authenticate a new short connection for every operation.
            if connect(&host, &endpoint).await.is_err() {
                let _ = outgoing.send(ControlEvent::Unavailable).await;
                return;
            }
            if outgoing.send(ControlEvent::Ready).await.is_err() {
                return;
            }
            while let Some(request) = incoming.recv().await {
                let mut stream = match connect(&host, &endpoint).await {
                    Ok(stream) => stream,
                    Err(_) => {
                        let _ = outgoing.send(ControlEvent::Reply(DeviceReply {
                            op: request.op, id: request.id, status: 1,
                            payload: json!({"error":"ZXTouch connection or authorization failed; operation was not sent"}).to_string(),
                        })).await;
                        continue;
                    }
                };
                let result = tokio::time::timeout(
                    Duration::from_secs(12),
                    exchange(&mut stream, &endpoint, &request),
                )
                .await;
                // Release the socket before publishing completion or awaiting the next command.
                drop(stream);
                match result {
                    Ok(Ok(reply)) => {
                        if outgoing.send(ControlEvent::Reply(reply)).await.is_err() {
                            break;
                        }
                    }
                    _ => {
                        let error = |request: DeviceRequest| {
                            ControlEvent::Reply(DeviceReply {
                            op: request.op, id: request.id, status: 1,
                            payload: json!({"error":"Control connection interrupted; operation result unknown. Reconnect before retrying."}).to_string(),
                        })
                        };
                        let _ = outgoing.send(error(request)).await;
                        incoming.close();
                        while let Some(request) = incoming.recv().await {
                            let _ = outgoing.send(error(request)).await;
                        }
                        let _ = outgoing.send(ControlEvent::Unavailable).await;
                        break;
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

async fn connect(host: &str, endpoint: &ControlEndpoint) -> std::io::Result<BufReader<TcpStream>> {
    tokio::time::timeout(Duration::from_secs(3), async {
        let stream = TcpStream::connect((host, endpoint.port)).await?;
        stream.set_nodelay(true)?;
        let mut stream = BufReader::new(stream);
        let hello = DeviceRequest {
            op: 0,
            id: 0,
            payload: "{}".into(),
        };
        if exchange(&mut stream, endpoint, &hello).await?.status != 0 {
            return Err(std::io::Error::other("Control authorization failed"));
        }
        Ok(stream)
    })
    .await
    .map_err(|_| std::io::Error::other("Control connection timed out"))?
}

async fn exchange(
    stream: &mut BufReader<TcpStream>,
    endpoint: &ControlEndpoint,
    request: &DeviceRequest,
) -> std::io::Result<DeviceReply> {
    let payload: Value = serde_json::from_str(&request.payload).map_err(std::io::Error::other)?;
    if request.payload.len() > 4096 {
        return Err(std::io::Error::other("Control request too large"));
    }
    let json = json!({"version":1,"token":endpoint.token,"op":request.op,"id":request.id,"payload":payload});
    let wire = format!("33{}\r\n", STANDARD.encode(json.to_string()));
    stream.get_mut().write_all(wire.as_bytes()).await?;
    let mut line = Vec::new();
    // Bound each read before appending; read_until alone can allocate without
    // bound when a peer never terminates a response.
    loop {
        let available = stream.fill_buf().await?;
        if available.is_empty() {
            return Err(std::io::Error::other("Control connection closed"));
        }
        let count = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |i| i + 1);
        if line.len() + count > 1_400_000 {
            return Err(std::io::Error::other("Control reply too large"));
        }
        line.extend_from_slice(&available[..count]);
        stream.consume(count);
        if line.last() == Some(&b'\n') {
            break;
        }
    }
    let encoded = line
        .strip_prefix(b"0;;")
        .and_then(|line| line.strip_suffix(b"\r\n"))
        .ok_or_else(|| std::io::Error::other("Invalid control reply"))?;
    let bytes = STANDARD.decode(encoded).map_err(std::io::Error::other)?;
    if bytes.len() > 1024 * 1024 {
        return Err(std::io::Error::other("Control reply too large"));
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
    if value["version"].as_u64() != Some(1)
        || value["id"].as_u64() != Some(request.id.into())
        || value["op"].as_u64() != Some(request.op.into())
        || !matches!(value["status"].as_u64(), Some(0 | 1))
        || !value["payload"].is_object()
    {
        return Err(std::io::Error::other("Mismatched control reply"));
    }
    Ok(DeviceReply {
        op: request.op,
        id: request.id,
        status: value["status"].as_u64().unwrap() as u8,
        payload: value["payload"].to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn disconnected_control_fails_once_and_disables_capabilities() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = ControlEndpoint {
            version: 1,
            port: listener.local_addr().unwrap().port(),
            token: "a".repeat(64),
        };
        let peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            let mut line = String::new();
            stream.read_line(&mut line).await.unwrap();
            let hello: Value = serde_json::from_slice(
                &STANDARD
                    .decode(line.trim().strip_prefix("33").unwrap())
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(hello["op"], 0);
            assert_eq!(hello["token"], "a".repeat(64));
            let reply = json!({"version":1,"op":0,"id":0,"status":0,"payload":{"ok":true}});
            stream
                .get_mut()
                .write_all(format!("0;;{}\r\n", STANDARD.encode(reply.to_string())).as_bytes())
                .await
                .unwrap();
            line.clear();
            assert_eq!(stream.read_line(&mut line).await.unwrap(), 0);
            let (socket, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(socket);
            stream.read_line(&mut line).await.unwrap();
            let hello: Value = serde_json::from_slice(
                &STANDARD
                    .decode(line.trim().strip_prefix("33").unwrap())
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(hello["op"], 0);
            stream
                .get_mut()
                .write_all(format!("0;;{}\r\n", STANDARD.encode(reply.to_string())).as_bytes())
                .await
                .unwrap();
            line.clear();
            stream.read_line(&mut line).await.unwrap();
            let request: Value = serde_json::from_slice(
                &STANDARD
                    .decode(line.trim().strip_prefix("33").unwrap())
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(request["op"], 12);
            // Disconnect after receiving the mutation. It must never be replayed.
        });
        let mut channel = ControlChannel::start("127.0.0.1".into(), endpoint);
        assert!(matches!(
            channel.events.recv().await,
            Some(ControlEvent::Ready)
        ));
        channel
            .requests
            .send(DeviceRequest {
                op: 12,
                id: 1,
                payload: "{}".into(),
            })
            .await
            .unwrap();
        assert!(matches!(
            channel.events.recv().await,
            Some(ControlEvent::Reply(DeviceReply {
                op: 12,
                id: 1,
                status: 1,
                ..
            }))
        ));
        assert!(matches!(
            channel.events.recv().await,
            Some(ControlEvent::Unavailable)
        ));
        assert!(
            channel
                .requests
                .send(DeviceRequest {
                    op: 12,
                    id: 2,
                    payload: "{}".into()
                })
                .await
                .is_err()
        );
        peer.await.unwrap();
    }

    #[tokio::test]
    async fn mismatched_authorization_reply_never_enables_control() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = ControlEndpoint {
            version: 1,
            port: listener.local_addr().unwrap().port(),
            token: "b".repeat(64),
        };
        let peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            let mut line = String::new();
            stream.read_line(&mut line).await.unwrap();
            let reply = json!({"version":1,"op":0,"id":99,"status":0,"payload":{}});
            stream
                .get_mut()
                .write_all(format!("0;;{}\r\n", STANDARD.encode(reply.to_string())).as_bytes())
                .await
                .unwrap();
        });
        let mut channel = ControlChannel::start("127.0.0.1".into(), endpoint);
        assert!(matches!(
            channel.events.recv().await,
            Some(ControlEvent::Unavailable)
        ));
        peer.await.unwrap();
    }
}
