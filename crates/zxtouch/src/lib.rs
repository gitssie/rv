//! Authenticated ZXTouch HTTP/2 API. Mutations are sent once; polling only observes IDs.
//! Payloads retain the native schema, including `fields` arrays for legacy device tasks.
pub mod transport;
use base64::Engine as _;
use reqwest::{Method, Response, header::HeaderMap};
use serde_json::{Value, json};
use std::time::Duration;
pub use transport::PhoneHttpClient;

#[derive(Clone)]
pub struct Api {
    http: PhoneHttpClient,
}
impl Api {
    pub fn new(http: PhoneHttpClient) -> Self {
        Self { http }
    }
    pub fn control(&self) -> Control<'_> {
        Control(self)
    }
    pub fn apps(&self) -> Apps<'_> {
        Apps(self)
    }
    pub fn device(&self) -> Device<'_> {
        Device(self)
    }
    pub fn scripts(&self) -> Scripts<'_> {
        Scripts(self)
    }
    pub fn karing_native(&self) -> KaringNative<'_> {
        KaringNative(self)
    }
    pub fn apple(&self) -> Apple<'_> {
        Apple(OperationService {
            api: self,
            module: Module::Apple,
        })
    }
    pub fn app_state(&self) -> AppState<'_> {
        AppState(OperationService {
            api: self,
            module: Module::AppState,
        })
    }
    /// Native JSON response, with HTTP and native rejection checked. No retry.
    pub async fn post(&self, path: &str, params: &Value) -> Result<Value, Error> {
        let (status, value) = self
            .http
            .json(path, params)
            .await
            .map_err(Error::Transport)?;
        if !(200..300).contains(&status) || value.get("error").is_some_and(|v| !v.is_null()) {
            return Err(Error::Rejected {
                status,
                body: value,
            });
        }
        Ok(value)
    }
    /// Preserve status/headers/encoded bytes and streaming; caller owns body limits/deadlines.
    pub async fn raw(
        &self,
        method: Method,
        path: &str,
        headers: HeaderMap,
        body: Vec<u8>,
    ) -> Result<Response, Error> {
        if !path.starts_with("/api/") || path.contains('#') {
            return Err(Error::Invalid("invalid API path"));
        }
        self.http
            .request(method, path, headers, body)
            .await
            .map_err(|_| {
                Error::Transport("HTTP request interrupted; outcome may be unknown".into())
            })
    }
    pub async fn stream(&self, path: &str, params: &Value) -> Result<Ndjson, Error> {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", "application/json".parse().unwrap());
        let response = self
            .raw(Method::POST, path, headers, params.to_string().into_bytes())
            .await?;
        if !response.status().is_success() {
            return Err(Error::Http(response.status().as_u16()));
        }
        Ok(Ndjson {
            response,
            pending: Vec::new(),
            ended: false,
        })
    }
    /// Raw Karing proxy: preserves arbitrary methods, query strings and response status.
    pub async fn karing(
        &self,
        method: Method,
        path_and_query: &str,
        headers: HeaderMap,
        body: Vec<u8>,
    ) -> Result<Response, Error> {
        if !path_and_query.starts_with('/') {
            return Err(Error::Invalid("Karing path must start with /"));
        }
        self.raw(
            method,
            &format!("/api/karing{path_and_query}"),
            headers,
            body,
        )
        .await
    }
}

/// Errors intentionally avoid formatting native bodies, which can contain credentials.
pub enum Error {
    Observing {
        operation_id: String,
        source: Box<Error>,
    },
    Transport(String),
    Invalid(&'static str),
    Http(u16),
    Rejected {
        status: u16,
        body: Value,
    },
    Operation {
        record: Value,
    },
    Timeout {
        operation_id: String,
    },
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Observing { operation_id, .. } => write!(
                f,
                "operation {operation_id} observation failed; query without replay"
            ),
            Self::Transport(s) => f.write_str(s),
            Self::Invalid(s) => f.write_str(s),
            Self::Http(s) | Self::Rejected { status: s, .. } => write!(f, "ZXTouch HTTP {s}"),
            Self::Operation { .. } => {
                f.write_str("native operation failed; inspect retained record")
            }
            Self::Timeout { operation_id } => write!(
                f,
                "operation {operation_id} timed out; query without replay"
            ),
        }
    }
}
impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}
impl std::error::Error for Error {}

macro_rules! namespace {
    ($ty:ident,$prefix:literal,[$($name:ident => $route:literal),* $(,)?]) => {
        pub struct $ty<'a>(&'a Api);
        impl $ty<'_> { $(pub async fn $name(&self,params:&Value)->Result<Value,Error> {
            self.0.post(concat!($prefix,$route),params).await
        })* }
    }
}
namespace!(Control,"/api/control/",[authorize=>"authorize",disconnect=>"disconnect"]);
namespace!(Apps,"/api/apps/",[list=>"list",launch=>"launch",stop=>"stop",restart=>"restart",foreground=>"foreground",icon=>"icon",access_token=>"access-token"]);
namespace!(Device,"/api/device/",[home=>"home",clipboard=>"clipboard",info=>"info",touch=>"touch",tap=>"tap",keyboard=>"keyboard",toast=>"toast",vibrate=>"vibrate",alert=>"alert",ocr=>"ocr",color=>"color",colors=>"colors",image_match=>"image-match",foreground_pid=>"foreground-pid",respring=>"respring",sleep=>"sleep"]);
namespace!(Scripts,"/api/scripts/",[shell=>"shell",start=>"start",stop=>"stop"]);
namespace!(KaringNative,"/api/karing-native/",[status=>"status",config=>"config",profiles=>"profiles",reload=>"reload",connection=>"connection",connect=>"connect",disconnect=>"disconnect",select=>"select"]);
impl Control<'_> {
    pub async fn connect(&self, password: &str) -> Result<Value, Error> {
        self.0
            .http
            .connect(password)
            .await
            .map_err(Error::Transport)
    }
}
impl Device<'_> {
    pub async fn screenshot(&self) -> Result<Response, Error> {
        let mut h = HeaderMap::new();
        h.insert("content-type", "application/json".parse().unwrap());
        self.0
            .raw(Method::POST, "/api/device/screenshot", h, b"{}".to_vec())
            .await
    }
    pub async fn lock(&self, params: &Value) -> Result<Ndjson, Error> {
        self.0.stream("/api/device/lock", params).await
    }
    pub async fn unlock(&self, params: &Value) -> Result<Ndjson, Error> {
        self.0.stream("/api/device/unlock", params).await
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Module {
    Apple,
    AppState,
}
impl Module {
    fn prefix(self) -> &'static str {
        match self {
            Self::Apple => "apple",
            Self::AppState => "app-state",
        }
    }
}
#[derive(Clone, Copy)]
pub struct OperationService<'a> {
    api: &'a Api,
    module: Module,
}
impl OperationService<'_> {
    pub async fn invoke(&self, method: &str, params: &Value) -> Result<Value, Error> {
        if !method
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || method.is_empty()
        {
            return Err(Error::Invalid("invalid native method"));
        }
        let (status, record) = self
            .api
            .http
            .json(&format!("/api/{}/{method}", self.module.prefix()), params)
            .await
            .map_err(Error::Transport)?;
        // Apple returns completed error records with HTTP 400. Preserve their operation IDs.
        if record.get("operation_id").is_none() {
            return Err(Error::Rejected {
                status,
                body: record,
            });
        }
        self.validate(
            &record,
            if method == "result" {
                None
            } else {
                Some(method)
            },
            params.get("operation_id").and_then(Value::as_str),
        )?;
        Ok(record)
    }
    fn validate(&self, r: &Value, method: Option<&str>, id: Option<&str>) -> Result<(), Error> {
        let valid = r["operation_id"].as_str().is_some_and(|s| !s.is_empty())
            && r["method"].as_str().is_some()
            && matches!(
                r["status"].as_str(),
                Some("queued" | "running" | "completed")
            )
            || self.module == Module::Apple
                && r["status"] == "requires_verification_code"
                && r["operation_id"].as_str().is_some_and(|s| !s.is_empty())
                && r["method"].as_str().is_some()
                && r["challenge"]["challenge_id"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty());
        if !valid
            || method.is_some_and(|m| r["method"] != m)
            || id.is_some_and(|i| r["operation_id"] != i)
        {
            return Err(Error::Invalid("malformed or mismatched operation record"));
        }
        Ok(())
    }
    pub async fn result(&self, id: &str) -> Result<Value, Error> {
        if id.is_empty() {
            return Err(Error::Invalid("operation_id required"));
        }
        self.invoke("result", &json!({"operation_id":id}))
            .await
            .map_err(|source| Error::Observing {
                operation_id: id.to_owned(),
                source: Box::new(source),
            })
    }
    /// Returns completed record or verification challenge. Errors retain the native record.
    pub async fn wait(&self, mut record: Value, timeout: Duration) -> Result<Value, Error> {
        self.validate(&record, None, None)?;
        let id = record["operation_id"].as_str().unwrap().to_owned();
        let method = record["method"].as_str().unwrap().to_owned();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            self.validate(&record, Some(&method), Some(&id))
                .map_err(|source| Error::Observing {
                    operation_id: id.clone(),
                    source: Box::new(source),
                })?;
            if record["status"] == "requires_verification_code" {
                return Ok(record);
            }
            if record["status"] == "completed" {
                if record.get("error").is_some_and(|v| !v.is_null())
                    || !record["result"].is_object()
                    || matches!(
                        record["result"]["outcome"].as_str(),
                        Some("failed" | "unknown" | "partial")
                    )
                {
                    return Err(Error::Operation { record });
                }
                return Ok(record);
            }
            let next = async {
                tokio::time::sleep(Duration::from_millis(250)).await;
                self.result(&id).await
            };
            record = tokio::time::timeout_at(deadline, next)
                .await
                .map_err(|_| Error::Timeout {
                    operation_id: id.clone(),
                })??;
        }
    }
    pub async fn submit_code(&self, id: &str, challenge: &str, code: &str) -> Result<Value, Error> {
        if self.module != Module::Apple
            || code.len() != 6
            || !code.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(Error::Invalid("six ASCII digits required"));
        }
        self.challenge("submit_code", id, challenge, Some(code))
            .await
    }
    pub async fn cancel_challenge(&self, id: &str, challenge: &str) -> Result<Value, Error> {
        self.challenge("cancel_challenge", id, challenge, None)
            .await
    }
    async fn challenge(
        &self,
        action: &str,
        id: &str,
        challenge: &str,
        code: Option<&str>,
    ) -> Result<Value, Error> {
        if self.module != Module::Apple || id.is_empty() || challenge.is_empty() {
            return Err(Error::Invalid("invalid challenge identity"));
        }
        let mut p = json!({"operation_id":id,"challenge_id":challenge});
        if let Some(c) = code {
            p["code"] = json!(c);
        }
        let r = self.api.post(&format!("/api/apple/{action}"), &p).await?;
        if r["operation_id"] != id || r["accepted"] != true {
            return Err(Error::Invalid("malformed challenge acknowledgement"));
        }
        Ok(r)
    }
    pub async fn stream(&self, params: &Value) -> Result<Ndjson, Error> {
        if self.module != Module::AppState {
            return Err(Error::Invalid("stream requires App-state"));
        }
        self.api
            .stream(&format!("/api/{}/stream", self.module.prefix()), params)
            .await
    }
    pub async fn download(&self, params: &Value) -> Result<Response, Error> {
        if self.module != Module::AppState {
            return Err(Error::Invalid("download requires App-state"));
        }
        let mut h = HeaderMap::new();
        h.insert("content-type", "application/json".parse().unwrap());
        self.api
            .raw(
                Method::POST,
                "/api/app-state/download",
                h,
                params.to_string().into_bytes(),
            )
            .await
    }
    /// One protocol chunk, with bounded input and exact acknowledgement verification.
    pub async fn upload(&self, params: &Value) -> Result<Value, Error> {
        if self.module != Module::AppState {
            return Err(Error::Invalid("upload requires App-state"));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(
                params["data"]
                    .as_str()
                    .ok_or(Error::Invalid("upload data required"))?,
            )
            .map_err(|_| Error::Invalid("invalid upload base64"))?;
        let offset = params["offset"]
            .as_u64()
            .ok_or(Error::Invalid("upload offset required"))?;
        let total = params["total"]
            .as_u64()
            .ok_or(Error::Invalid("upload total required"))?;
        let limit = match params["part"].as_str() {
            Some("state") => 64 * 1024 * 1024,
            Some("manifest") => 16 * 1024 * 1024,
            _ => return Err(Error::Invalid("invalid upload part")),
        };
        let received = offset
            .checked_add(bytes.len() as u64)
            .ok_or(Error::Invalid("upload offset overflow"))?;
        if bytes.is_empty() || bytes.len() > 256 * 1024 || total > limit || received > total {
            return Err(Error::Invalid("upload chunk bounds invalid"));
        }
        let reply = self.api.post("/api/app-state/upload", params).await?;
        if reply["ok"] != true
            || reply["received"].as_u64() != Some(received)
            || reply["complete"].as_bool() != Some(received == total)
        {
            return Err(Error::Invalid(
                "upload acknowledgement mismatch; outcome may be unknown",
            ));
        }
        Ok(reply)
    }
}

/// Incremental NDJSON reader, bounded to 1 MiB per event. EOF is not a terminal operation result.
pub struct Ndjson {
    response: Response,
    pending: Vec<u8>,
    ended: bool,
}
impl Ndjson {
    pub async fn next(&mut self) -> Result<Option<Value>, Error> {
        loop {
            if let Some(i) = self.pending.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.pending.drain(..=i).collect();
                if i > 1024 * 1024 {
                    return Err(Error::Invalid("NDJSON event exceeds limit"));
                }
                if line.iter().all(|b| b.is_ascii_whitespace()) {
                    continue;
                }
                return serde_json::from_slice(&line)
                    .map(Some)
                    .map_err(|_| Error::Invalid("invalid NDJSON event"));
            }
            if self.pending.len() > 1024 * 1024 {
                return Err(Error::Invalid("NDJSON event exceeds limit"));
            }
            if self.ended {
                if self.pending.is_empty() {
                    return Ok(None);
                }
                return serde_json::from_slice(&std::mem::take(&mut self.pending))
                    .map(Some)
                    .map_err(|_| Error::Invalid("truncated NDJSON event"));
            }
            match self.response.chunk().await.map_err(|_| {
                Error::Transport("stream interrupted; outcome may be unknown".into())
            })? {
                Some(c) => self.pending.extend_from_slice(&c),
                None => self.ended = true,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn service(api: &Api) -> Apple<'_> {
        api.apple()
    }
    #[test]
    fn errors_redact_server_bodies_and_credentials() {
        let error = Error::Rejected {
            status: 400,
            body: json!({"password":"SECRET","access_token":"SECRET"}),
        };
        assert!(!format!("{error:?}").contains("SECRET"));
        let error = Error::Operation {
            record: json!({"result":{"access_token":"SECRET"}}),
        };
        assert!(!format!("{error:?}").contains("SECRET"));
    }
    #[tokio::test]
    async fn direct_result_failures_retain_requested_id() {
        for reply in [
            None,
            Some((404, json!({"error":"operation_not_found"}))),
            Some((
                200,
                json!({"operation_id":"wrong", "method":"list", "status":"completed", "result":{}}),
            )),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let mut connection = h2::server::handshake(socket).await.unwrap();
                let (request, mut sender) = connection.accept().await.unwrap().unwrap();
                assert_eq!(request.uri().path(), "/api/app-state/result");
                if let Some((status, value)) = reply {
                    sender
                        .send_response(
                            http::Response::builder().status(status).body(()).unwrap(),
                            false,
                        )
                        .unwrap()
                        .send_data(value.to_string().into(), true)
                        .unwrap();
                    let _ =
                        tokio::time::timeout(Duration::from_millis(100), connection.accept()).await;
                }
            });
            let api = Api::new(PhoneHttpClient::new("127.0.0.1", port, "fixture".into()).unwrap());
            let error = api.app_state().result("retained").await.unwrap_err();
            assert!(
                matches!(error, Error::Observing { operation_id, .. } if operation_id == "retained")
            );
            server.await.unwrap();
        }
    }
    #[tokio::test]
    async fn upload_validates_bounds_and_exact_acknowledgement() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut c = h2::server::handshake(socket).await.unwrap();
            let (request, mut send) = c.accept().await.unwrap().unwrap();
            assert_eq!(request.uri().path(), "/api/app-state/upload");
            send.send_response(
                http::Response::builder().status(200).body(()).unwrap(),
                false,
            )
            .unwrap()
            .send_data(r#"{"ok":true,"received":2,"complete":true}"#.into(), true)
            .unwrap();
            let _ = tokio::time::timeout(Duration::from_millis(200), c.accept()).await;
        });
        let api = Api::new(PhoneHttpClient::new("127.0.0.1", port, "fixture".into()).unwrap());
        let params = json!({"data":"YQ==","part":"state","offset":0,"total":1});
        assert!(api.apple().upload(&params).await.is_err());
        assert!(api.app_state().upload(&params).await.is_err());
        server.await.unwrap();
        assert!(
            api.app_state()
                .upload(&json!({"data":"","part":"state","offset":0,"total":0}))
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn polling_timeout_retains_id_without_replaying_mutation() {
        let api = Api::new(PhoneHttpClient::new("127.0.0.1", 1, "fixture".into()).unwrap());
        let r = json!({"operation_id":"retained","method":"backup","status":"running"});
        assert!(
            matches!(api.app_state().wait(r,Duration::from_millis(1)).await,Err(Error::Timeout{operation_id}) if operation_id=="retained")
        );
    }
    #[tokio::test]
    async fn ndjson_reads_fragmented_events_without_waiting_for_eof() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut c = h2::server::handshake(socket).await.unwrap();
            let (request, mut send) = c.accept().await.unwrap().unwrap();
            assert_eq!(request.headers()["authorization"], "Bearer fixture");
            let mut stream = send
                .send_response(
                    http::Response::builder().status(200).body(()).unwrap(),
                    false,
                )
                .unwrap();
            stream.send_data("{\"phase\":\"lo".into(), false).unwrap();
            stream
                .send_data("cked\"}\n{\"phase\":\"next\"}\n".into(), false)
                .unwrap();
            let _ = tokio::time::timeout(Duration::from_millis(300), c.accept()).await;
        });
        let api = Api::new(PhoneHttpClient::new("127.0.0.1", port, "fixture".into()).unwrap());
        let mut events = api.device().lock(&json!({})).await.unwrap();
        assert_eq!(events.next().await.unwrap().unwrap()["phase"], "locked");
        assert_eq!(events.next().await.unwrap().unwrap()["phase"], "next");
        drop(events);
        server.await.unwrap();
    }
    #[tokio::test]
    async fn operation_preserves_terminal_error_on_http_400() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut c = h2::server::handshake(socket).await.unwrap();
            let (request, mut send) = c.accept().await.unwrap().unwrap();
            assert_eq!(request.uri().path(), "/api/apple/signInAppStore");
            let record = json!({"operation_id":"op","method":"signInAppStore","status":"completed","result":{"outcome":"unknown"},"error":{"message":"fixture"}});
            send.send_response(
                http::Response::builder().status(400).body(()).unwrap(),
                false,
            )
            .unwrap()
            .send_data(record.to_string().into(), true)
            .unwrap();
            let _ = tokio::time::timeout(Duration::from_millis(200), c.accept()).await;
        });
        let api = Api::new(PhoneHttpClient::new("127.0.0.1", port, "fixture".into()).unwrap());
        let record = api.apple().sign_in_app_store(&json!({})).await.unwrap();
        assert_eq!(record["operation_id"], "op");
        assert!(matches!(
            api.apple().wait(record, Duration::from_secs(1)).await,
            Err(Error::Operation { .. })
        ));
        server.await.unwrap();
    }
    #[tokio::test]
    async fn wait_returns_challenge_without_polling_and_rejects_bad_identity() {
        let api = Api::new(PhoneHttpClient::new("127.0.0.1", 1, "fixture".into()).unwrap());
        let record = json!({"operation_id":"op","method":"signInAppStore","status":"requires_verification_code","challenge":{"challenge_id":"challenge"}});
        assert_eq!(
            service(&api)
                .wait(record.clone(), Duration::from_secs(1))
                .await
                .unwrap(),
            record
        );
        assert!(
            service(&api)
                .validate(&record, Some("queryState"), None)
                .is_err()
        );
        assert!(api.app_state().validate(&record, None, None).is_err());
        assert!(
            service(&api)
                .submit_code("op", "challenge", "12345")
                .await
                .is_err()
        );
    }
}

pub struct Apple<'a>(OperationService<'a>);
impl<'a> std::ops::Deref for Apple<'a> {
    type Target = OperationService<'a>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<'a> Apple<'a> {
    pub fn operations(&self) -> OperationService<'a> {
        self.0
    }

    pub async fn change_password(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("changePassword", params).await
    }
    pub async fn query_state(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("queryState", params).await
    }
    pub async fn query_account_state(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("queryAccountState", params).await
    }
    pub async fn sign_in_app_store(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("signInAppStore", params).await
    }
    pub async fn sign_out_app_store(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("signOutAppStore", params).await
    }
    pub async fn enable_icloud(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("enableICloud", params).await
    }
    pub async fn sign_out_apple_id(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("signOutAppleID", params).await
    }
    pub async fn sign_out_icloud(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("signOutICloud", params).await
    }
    pub async fn disable_find_my(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("disableFindMy", params).await
    }
    pub async fn query_name(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("queryName", params).await
    }
    pub async fn query_email(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("queryEmail", params).await
    }
    pub async fn set_name(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("setName", params).await
    }
    pub async fn begin_adding_contact_email(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("beginAddingContactEmail", params).await
    }
    pub async fn verify_contact_email(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("verifyContactEmail", params).await
    }
    pub async fn set_forwarding_email(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("setForwardingEmail", params).await
    }
    pub async fn query_sign_in_with_apple_apps(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("querySignInWithAppleApps", params).await
    }
    pub async fn stop_using_apple_id(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("stopUsingAppleID", params).await
    }
    pub async fn query_icloud_devices(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("queryICloudDevices", params).await
    }
    pub async fn remove_other_icloud_devices(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("removeOtherICloudDevices", params).await
    }
}

pub struct AppState<'a>(OperationService<'a>);
impl<'a> std::ops::Deref for AppState<'a> {
    type Target = OperationService<'a>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<'a> AppState<'a> {
    pub fn operations(&self) -> OperationService<'a> {
        self.0
    }

    pub async fn list(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("list", params).await
    }
    pub async fn edit(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("edit", params).await
    }
    pub async fn backup(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("backup", params).await
    }
    pub async fn restore(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("restore", params).await
    }
    pub async fn delete(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("delete", params).await
    }
    pub async fn verify(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("verify", params).await
    }
    pub async fn launch(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("launch", params).await
    }
    pub async fn stop(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("stop", params).await
    }
    pub async fn status(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("status", params).await
    }
    pub async fn claim(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("claim", params).await
    }
    pub async fn release(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("release", params).await
    }
    pub async fn ownership(&self, params: &Value) -> Result<Value, Error> {
        self.0.invoke("ownership", params).await
    }
}

/// Current native route inventory, including the unrestricted Karing proxy.
pub const ROUTES: &[&str] = &[
    "/api/control/connect",
    "/api/control/disconnect",
    "/api/control/authorize",
    "/api/apps/list",
    "/api/apps/launch",
    "/api/apps/stop",
    "/api/apps/restart",
    "/api/apps/foreground",
    "/api/apps/icon",
    "/api/apps/access-token",
    "/api/device/home",
    "/api/device/clipboard",
    "/api/device/info",
    "/api/device/touch",
    "/api/device/tap",
    "/api/device/screenshot",
    "/api/device/keyboard",
    "/api/device/toast",
    "/api/device/vibrate",
    "/api/device/alert",
    "/api/device/ocr",
    "/api/device/color",
    "/api/device/colors",
    "/api/device/image-match",
    "/api/device/foreground-pid",
    "/api/device/respring",
    "/api/device/sleep",
    "/api/device/lock",
    "/api/device/unlock",
    "/api/scripts/shell",
    "/api/scripts/start",
    "/api/scripts/stop",
    "/api/apple/changePassword",
    "/api/apple/queryState",
    "/api/apple/queryAccountState",
    "/api/apple/signInAppStore",
    "/api/apple/signOutAppStore",
    "/api/apple/enableICloud",
    "/api/apple/signOutAppleID",
    "/api/apple/signOutICloud",
    "/api/apple/disableFindMy",
    "/api/apple/queryName",
    "/api/apple/queryEmail",
    "/api/apple/setName",
    "/api/apple/beginAddingContactEmail",
    "/api/apple/verifyContactEmail",
    "/api/apple/setForwardingEmail",
    "/api/apple/querySignInWithAppleApps",
    "/api/apple/stopUsingAppleID",
    "/api/apple/queryICloudDevices",
    "/api/apple/removeOtherICloudDevices",
    "/api/apple/result",
    "/api/apple/submit_code",
    "/api/apple/cancel_challenge",
    "/api/app-state/list",
    "/api/app-state/edit",
    "/api/app-state/backup",
    "/api/app-state/restore",
    "/api/app-state/delete",
    "/api/app-state/verify",
    "/api/app-state/launch",
    "/api/app-state/stop",
    "/api/app-state/status",
    "/api/app-state/claim",
    "/api/app-state/release",
    "/api/app-state/ownership",
    "/api/app-state/result",
    "/api/app-state/stream",
    "/api/app-state/download",
    "/api/app-state/upload",
    "/api/karing-native/status",
    "/api/karing-native/config",
    "/api/karing-native/profiles",
    "/api/karing-native/reload",
    "/api/karing-native/connection",
    "/api/karing-native/connect",
    "/api/karing-native/disconnect",
    "/api/karing-native/select",
    "/api/karing/*",
];
