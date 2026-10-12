//! Finite RV requests through the existing raw Karing HTTP proxy.
//! No phone-side routes, JSON envelopes, retries, or inferred write success.
use crate::phone_http::PhoneHttpClient;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KaringRequest {
    pub id: u32,
    pub method: String,
    /// Original Karing path and query, without the proxy prefix.
    pub path: String,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct KaringResponse {
    pub id: u32,
    pub status: u16,
    pub headers: Vec<(String, Vec<u8>)>,
    pub body: Vec<u8>,
}

pub(crate) struct KaringOperation {
    pub command: crate::AppCommand,
    pub result: tokio::sync::oneshot::Receiver<Result<KaringResponse, String>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for KaringOperation {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl KaringOperation {
    pub fn start(client: PhoneHttpClient, request: KaringRequest) -> Result<Self, String> {
        let method = request
            .method
            .parse::<reqwest::Method>()
            .map_err(|_| "请求方法无效")?;
        if !request.path.starts_with('/')
            || request.path.starts_with("//")
            || request.path.contains('#')
        {
            return Err("Karing 请求路径无效".into());
        }
        let command = crate::AppCommand::Karing(request.clone());
        let (tx, result) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let value = tokio::time::timeout(std::time::Duration::from_secs(15), async {
                let mut headers = reqwest::header::HeaderMap::new();
                if !request.body.is_empty() {
                    headers.insert("content-type", "application/json".parse().unwrap());
                }
                let mut response = client
                    .request(
                        method,
                        &format!("/api/karing{}", request.path),
                        headers,
                        request.body,
                    )
                    .await
                    .map_err(|_| "Karing 请求中断；修改结果可能未知".to_string())?;
                let status = response.status().as_u16();
                let headers = response
                    .headers()
                    .iter()
                    .map(|(name, value)| (name.to_string(), value.as_bytes().to_vec()))
                    .collect();
                let mut body = Vec::new();
                while let Some(chunk) = response
                    .chunk()
                    .await
                    .map_err(|_| "Karing 回复中断".to_string())?
                {
                    if body.len() + chunk.len() > 4 * 1024 * 1024 {
                        return Err("Karing 回复超过面板读取上限".into());
                    }
                    body.extend_from_slice(&chunk);
                }
                Ok(KaringResponse {
                    id: request.id,
                    status,
                    headers,
                    body,
                })
            })
            .await
            .unwrap_or_else(|_| Err("Karing 请求超时；不会自动重试".into()));
            let _ = tx.send(value);
        });
        Ok(Self {
            command,
            result,
            task,
        })
    }
}

/// Encode one path component, including slash, percent, spaces and Unicode.
pub fn karing_component(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            use std::fmt::Write;
            write!(&mut out, "%{byte:02X}").unwrap();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn karing_preserves_method_encoded_query_body_and_error_response() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut connection = h2::server::handshake(socket).await.unwrap();
            let (request, mut respond) = connection.accept().await.unwrap().unwrap();
            assert_eq!(request.method(), "PUT");
            assert_eq!(
                request.uri().path_and_query().unwrap().as_str(),
                "/api/karing/providers/proxies/a%2Fb?unknown=x+y&unknown=%25"
            );
            assert_eq!(request.headers()["authorization"], "Bearer fixture-token");
            let handler = tokio::spawn(async move {
                let mut request_body = request.into_body();
                let mut body = vec![];
                while let Some(bytes) = request_body.data().await {
                    let bytes = bytes.unwrap();
                    request_body
                        .flow_control()
                        .release_capacity(bytes.len())
                        .unwrap();
                    body.extend_from_slice(&bytes);
                }
                assert_eq!(body, b"{ \"unknown\": [1,2] }\n");
                let response = hyper::Response::builder()
                    .status(422)
                    .header("x-karing", "original")
                    .body(())
                    .unwrap();
                let mut stream = respond.send_response(response, false).unwrap();
                stream
                    .send_data(
                        hyper::body::Bytes::from_static(b"not-json\0unchanged"),
                        true,
                    )
                    .unwrap();
            });
            while connection.accept().await.is_some() {}
            handler.await.unwrap();
        });
        let mut operation = KaringOperation::start(
            PhoneHttpClient::new("127.0.0.1", port, "fixture-token".into()).unwrap(),
            KaringRequest {
                id: 8,
                method: "PUT".into(),
                path: "/providers/proxies/a%2Fb?unknown=x+y&unknown=%25".into(),
                body: b"{ \"unknown\": [1,2] }\n".to_vec(),
            },
        )
        .unwrap();
        let response = tokio::time::timeout(Duration::from_secs(2), &mut operation.result)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(response.id, 8);
        assert_eq!(response.status, 422);
        assert_eq!(response.body, b"not-json\0unchanged");
        assert!(
            response
                .headers
                .contains(&("x-karing".into(), b"original".to_vec()))
        );
        drop(operation);
        tokio::time::timeout(Duration::from_secs(2), peer)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn karing_cancellation_resets_stream() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (ready_tx, ready) = tokio::sync::oneshot::channel();
        let (reset_tx, reset) = tokio::sync::oneshot::channel();
        let peer = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut connection = h2::server::handshake(socket).await.unwrap();
            let (_, mut respond) = connection.accept().await.unwrap().unwrap();
            let mut stream = respond
                .send_response(hyper::Response::new(()), false)
                .unwrap();
            let handler = tokio::spawn(async move {
                ready_tx.send(()).unwrap();
                let reason = std::future::poll_fn(|cx| stream.poll_reset(cx))
                    .await
                    .unwrap();
                assert_eq!(reason, h2::Reason::CANCEL);
                reset_tx.send(()).unwrap();
            });
            while connection.accept().await.is_some() {}
            handler.await.unwrap();
        });
        let operation = KaringOperation::start(
            PhoneHttpClient::new("127.0.0.1", port, "fixture-token".into()).unwrap(),
            KaringRequest {
                id: 9,
                method: "GET".into(),
                path: "/proxies".into(),
                body: vec![],
            },
        )
        .unwrap();
        tokio::time::timeout(Duration::from_secs(2), ready)
            .await
            .unwrap()
            .unwrap();
        drop(operation);
        tokio::time::timeout(Duration::from_secs(2), reset)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), peer)
            .await
            .unwrap()
            .unwrap();
    }

    #[test]
    fn karing_names_escape_slashes_query_delimiters_and_unicode() {
        assert_eq!(
            karing_component("FR/中文 ?%"),
            "FR%2F%E4%B8%AD%E6%96%87%20%3F%25"
        );
    }
}
