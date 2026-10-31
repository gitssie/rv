//! App-state snapshots over one HTTP/2 NDJSON response. Never replay an invoke.
use super::*;
use reqwest::{Method, header::HeaderMap};

pub(super) async fn request(
    client: &crate::phone_http::PhoneHttpClient,
    body: &Value,
    events: &mpsc::Sender<FunctionEvent>,
) -> Result<Value, String> {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", "application/json".parse().unwrap());
    let mut response = tokio::time::timeout(
        Duration::from_secs(10),
        client.api().raw(
            Method::POST,
            "/api/app-state/stream",
            headers,
            body.to_string().into_bytes(),
        ),
    )
    .await
    .map_err(|_| "备份请求超时；操作可能已开始，请勿重复执行".to_owned())?
    .map_err(|_| "备份连接中断；操作可能已开始，请勿重复执行".to_owned())?;
    if response.status() != 200
        || response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_none_or(|v| !v.starts_with("application/x-ndjson"))
    {
        return Err("备份流式接口返回异常；请核对原操作结果".into());
    }
    let expected = body["operation_id"].as_str().map(str::to_owned);
    let mut decoder = Decoder {
        operation: expected,
        pending: Vec::new(),
        last: Value::Null,
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
    loop {
        let chunk = tokio::select! {
            _ = events.closed() => return Err("界面已关闭；手机操作继续执行".into()),
            _ = tokio::time::sleep_until(deadline) => return Err("备份等待超时；请查询原操作结果".into()),
            chunk = tokio::time::timeout(Duration::from_secs(30), response.chunk()) =>
                chunk.map_err(|_| "备份进度连接超时；请查询原操作结果".to_owned())?
                    .map_err(|_| "备份进度连接中断；请查询原操作结果".to_owned())?,
        };
        let Some(chunk) = chunk else {
            return Err("备份进度连接提前结束；请查询原操作结果".into());
        };
        // Process each complete line before a later malformed line can hide the accepted ID.
        for fragment in chunk.split_inclusive(|byte| *byte == b'\n') {
            for record in decoder.push(fragment)? {
                if record["status"] == "completed" || record.get("operation_id").is_none() {
                    return Ok(record);
                }
                events
                    .send(FunctionEvent::State { task: 32, record })
                    .await
                    .map_err(|_| "界面已关闭；手机操作继续执行".to_owned())?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn decoder() -> Decoder {
        Decoder {
            operation: None,
            pending: vec![],
            last: Value::Null,
        }
    }
    #[test]
    fn fragmented_unicode_duplicate_heartbeat_and_terminal_are_bounded() {
        let mut d = decoder();
        let state = json!({"event":"state","record":{"operation_id":"one","status":"running","progress":{"stage":"复制"}}}).to_string()+"\n";
        let mut records = vec![];
        for byte in state.as_bytes() {
            records.extend(d.push(&[*byte]).unwrap());
        }
        assert_eq!(records.len(), 1);
        assert!(d.push(state.as_bytes()).unwrap().is_empty());
        assert!(d.push(b"{\"event\":\"heartbeat\"}\n").unwrap().is_empty());
        assert!(d.push(b"{\"event\":\"state\",\"record\":{\"operation_id\":\"other\",\"status\":\"completed\"}}\n").is_err());
        let mut d = decoder();
        assert!(d.push(&vec![b'x'; 1024 * 1024 + 1]).is_err());
    }
    #[tokio::test]
    async fn http2_delivers_progress_before_terminal_without_waiting_for_eof() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (release, ready) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut connection = h2::server::handshake(socket).await.unwrap();
            let (request, mut respond) = connection.accept().await.unwrap().unwrap();
            assert_eq!(request.uri().path(), "/api/app-state/stream");
            let mut body = request.into_body();
            while let Some(bytes) = body.data().await {
                let bytes = bytes.unwrap();
                body.flow_control().release_capacity(bytes.len()).unwrap();
            }
            let mut stream = respond
                .send_response(
                    hyper::Response::builder()
                        .header("content-type", "application/x-ndjson")
                        .body(())
                        .unwrap(),
                    false,
                )
                .unwrap();
            let driver = tokio::spawn(async move {
                while let Some(request) = connection.accept().await {
                    request.unwrap();
                    panic!("request must not replay");
                }
            });
            stream.send_data(hyper::body::Bytes::from_static(b"{\"event\":\"state\",\"record\":{\"operation_id\":\"accepted\",\"status\":\"running\",\"progress\":{\"files\":7,\"bytes\":100}}}\n"),false).unwrap();
            ready.await.unwrap();
            stream.send_data(hyper::body::Bytes::from_static(b"{\"event\":\"state\",\"record\":{\"operation_id\":\"accepted\",\"status\":\"completed\",\"result\":{}}}\n"),false).unwrap();
            tokio::time::sleep(Duration::from_millis(700)).await;
            drop(stream);
            driver.abort();
        });
        let client =
            crate::phone_http::PhoneHttpClient::new("127.0.0.1", port, "test".into()).unwrap();
        let (events, mut received) = mpsc::channel(8);
        let operation = tokio::spawn(async move {
            request(
                &client,
                &json!({"action":"invoke","method":"save","params":{"bundle_id":"test.app"}}),
                &events,
            )
            .await
        });
        let first = tokio::time::timeout(Duration::from_secs(2), received.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(first,FunctionEvent::State {task:32,record} if record["operation_id"]=="accepted" && record["progress"]["files"]==7)
        );
        release.send(()).unwrap();
        let result = tokio::time::timeout(Duration::from_millis(500), operation)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(result["status"], "completed");
        server.await.unwrap();
    }
}

struct Decoder {
    operation: Option<String>,
    pending: Vec<u8>,
    last: Value,
}
impl Decoder {
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<Value>, String> {
        let mut states = Vec::new();
        for &byte in bytes {
            if byte != b'\n' {
                if self.pending.len() >= 1024 * 1024 {
                    return Err("备份进度回复超过限制".into());
                }
                self.pending.push(byte);
                continue;
            }
            let event: Value =
                serde_json::from_slice(&self.pending).map_err(|_| "备份进度格式错误")?;
            self.pending.clear();
            match event["event"].as_str() {
                Some("heartbeat") => continue,
                Some("state") => {}
                _ => return Err("备份进度事件类型错误".into()),
            }
            let record = &event["record"];
            if !record.is_object() {
                return Err("备份进度状态错误".into());
            }
            let Some(id) = record["operation_id"]
                .as_str()
                .filter(|id| !id.is_empty() && id.len() <= 128)
            else {
                if self.operation.is_none() && record.get("error").is_some() {
                    states.push(record.clone());
                    break;
                }
                return Err("备份进度缺少原操作标识".into());
            };
            if self
                .operation
                .as_deref()
                .is_some_and(|expected| expected != id)
            {
                return Err("备份进度操作标识不匹配".into());
            }
            self.operation = Some(id.into());
            if !matches!(record["status"].as_str(), Some("running" | "completed")) {
                return Err("备份进度状态无效".into());
            }
            if record["status"] == "completed"
                && (record.get("result").is_some() == record.get("error").is_some())
            {
                return Err("备份结束回复缺少明确结果".into());
            }
            if record != &self.last {
                states.push(record.clone());
                self.last = record.clone();
            }
            // A terminal record is authoritative; do not wait for EOF or accept late callbacks.
            if record["status"] == "completed" {
                break;
            }
        }
        Ok(states)
    }
}
