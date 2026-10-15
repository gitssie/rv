//! Shared HTTP/2 client for the phone control service, including raw Karing streams.
use reqwest::{Client, Method, Response, header::HeaderMap};
use serde_json::Value;
use std::sync::{Arc, RwLock};
use std::time::Duration;
#[derive(Clone)]
pub struct PhoneHttpClient {
    client: Client,
    origin: String,
    token: Arc<RwLock<String>>,
}
impl PhoneHttpClient {
    pub fn new(host: &str, port: u16, token: String) -> Result<Self, reqwest::Error> {
        let host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host.to_owned()
        };
        Ok(Self {
            client: Client::builder()
                .http2_prior_knowledge()
                .no_proxy()
                .retry(reqwest::retry::never())
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(5))
                .build()?,
            origin: format!("http://{host}:{port}"),
            token: Arc::new(RwLock::new(token)),
        })
    }
    /// Returns the actual status, headers and response body/stream. No JSON schema.
    /// Callers bound finite requests; traffic/log streams have no global deadline.
    pub async fn request(
        &self,
        method: Method,
        path_and_query: &str,
        headers: HeaderMap,
        body: Vec<u8>,
    ) -> Result<Response, reqwest::Error> {
        let token = self.token.read().unwrap().clone();
        self.client
            .request(method, format!("{}{}", self.origin, path_and_query))
            .headers(headers)
            .bearer_auth(token)
            .body(body)
            .send()
            .await
    }
    pub(crate) async fn connect(&self, password: &str) -> Result<crate::AppCapabilities, String> {
        let (status, response) = self
            .json(
                "/api/control/connect",
                &serde_json::json!({"password":password}),
            )
            .await?;
        self.accept_connect(status, response)
    }
    fn accept_connect(
        &self,
        status: u16,
        response: Value,
    ) -> Result<crate::AppCapabilities, String> {
        if status != 200 {
            return Err(if status == 401
                && response["error"] == "Control authorization expired or unavailable"
            {
                "ZXTouch 认证入口被前置授权拦截；请更新 TrollVNC 3.2-320 或更新版本并重启服务"
            } else if status == 401 && response["error"] == "authentication_failed" {
                "ZXTouch 密码验证失败"
            } else {
                "ZXTouch 独立控制接口不可用，请更新手机版"
            }
            .into());
        }
        let token = response["access_token"]
            .as_str()
            .filter(|t| t.len() <= 4096 && t.split('.').count() == 3)
            .ok_or("ZXTouch 未返回 JWT")?;
        if response["token_type"] != "Bearer" {
            return Err("ZXTouch 认证返回不正确".into());
        }
        let caps: crate::AppCapabilities = serde_json::from_value(response["capabilities"].clone())
            .map_err(|_| "ZXTouch 能力信息不正确")?;
        if !caps
            .zx_control
            .as_ref()
            .is_some_and(|e| e.version == 3 && e.token == token)
        {
            return Err("ZXTouch 独立控制版本不正确".into());
        }
        *self.token.write().unwrap() = token.to_owned();
        Ok(caps)
    }
    pub(crate) async fn json(&self, path: &str, value: &Value) -> Result<(u16, Value), String> {
        tokio::time::timeout(Duration::from_secs(10), async {
            let mut headers = HeaderMap::new();
            headers.insert("content-type", "application/json".parse().unwrap());
            let mut response = self
                .request(Method::POST, path, headers, value.to_string().into_bytes())
                .await
                .map_err(|_| "手机 HTTP 请求中断；结果可能未知".to_string())?;
            let status = response.status().as_u16();
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| "手机 HTTP 回复中断；结果可能未知".to_string())?
            {
                if bytes.len() + chunk.len() > 1024 * 1024 {
                    return Err("手机回复超过限制".into());
                }
                bytes.extend_from_slice(&chunk);
            }
            let value: Value =
                serde_json::from_slice(&bytes).map_err(|_| "手机回复 JSON 错误".to_string())?;
            if !value.is_object() {
                return Err("手机回复不是对象".into());
            }
            Ok((status, value))
        })
        .await
        .map_err(|_| "手机请求超时；请查询原操作结果，不要重复执行".to_string())?
    }
}
pub(crate) fn app_route(op: u8) -> Option<&'static str> {
    Some(match op {
        0 => "/api/control/authorize",
        1 => "/api/apps/list",
        2 => "/api/apps/launch",
        3 => "/api/apps/stop",
        4 => "/api/apps/restart",
        5 => "/api/apps/foreground",
        6 => "/api/apps/icon",
        12 => "/api/device/home",
        16 => "/api/device/clipboard",
        25 => "/api/device/info",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn refused_stream_never_replays_a_mutation() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut connection = h2::server::handshake(socket).await.unwrap();
            let (request, mut response) = connection.accept().await.unwrap().unwrap();
            assert_eq!(request.uri().path(), "/api/apple/signInAppStore");
            response.send_reset(h2::Reason::REFUSED_STREAM);
            assert!(
                tokio::time::timeout(Duration::from_millis(200), connection.accept())
                    .await
                    .is_err(),
                "mutation must not be resent on the connection"
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(200), listener.accept())
                    .await
                    .is_err(),
                "mutation must not open a retry connection"
            );
        });
        let client = PhoneHttpClient::new("127.0.0.1", port, "fixture-token".into()).unwrap();
        assert!(
            client
                .json(
                    "/api/apple/signInAppStore",
                    &serde_json::json!({"apple_id":"fixture", "password":"fixture"})
                )
                .await
                .is_err()
        );
        peer.await.unwrap();
    }
}
