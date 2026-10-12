//! Read-only probe of the installed phone's HTTP/2 Karing proxy.
//! Reuses RV's local saved credentials; never prints tokens or configuration secrets.
use rv_core::{ConnectRequest, Connection, StorePaths};
use rv_session::{AppEvent, SessionEvent, SessionHandle, phone_http::PhoneHttpClient};
use serde_json::Value;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = std::env::args()
        .nth(1)
        .ok_or("usage: live_karing_config_probe HOST")?;
    let paths = StorePaths::default_dir()?;
    let book: Value = serde_json::from_slice(&std::fs::read(&paths.address_book)?)?;
    let saved = book["connections"]
        .as_array()
        .ok_or("missing address book")?
        .iter()
        .find(|c| c["host"].as_str() == Some(&host))
        .ok_or("save this phone in RV first")?;
    let connection: Connection = serde_json::from_value(saved.clone())?;
    let mut request =
        ConnectRequest::from_connection(&connection, paths.load_password(connection.id)?);
    // A shared probe must not displace the user's existing viewer.
    request.shared = true;
    request.view_only = false;
    request.zxtouch_port = Some(6000);
    let session = SessionHandle::spawn(request);
    let result = async {
        let deadline = Instant::now() + Duration::from_secs(15);
        let endpoint = loop {
            if Instant::now() >= deadline { return Err("control discovery timed out".into()); }
            match session.try_recv() {
                Some(SessionEvent::Apps(AppEvent::Capabilities(caps))) => {
                    if let Some(endpoint) = caps.zx_control { break endpoint; }
                }
                Some(SessionEvent::Error(_)) => return Err("VNC authentication/connection failed".into()),
                _ => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        };
        let client = PhoneHttpClient::new(&host, 6000, endpoint.token)?;
        for path in ["/configs", "/providers/proxies", "/proxies"] {
            let response = tokio::time::timeout(Duration::from_secs(10), client.request(
                reqwest::Method::GET, &format!("/api/karing{path}"), Default::default(), vec![],
            )).await??;
            let status = response.status();
            let version = response.version();
            let value: Value = serde_json::from_slice(&response.bytes().await?)?;
            if !status.is_success() { return Err(format!("{path}: HTTP {status}").into()); }
            let summary = match path {
                "/configs" => serde_json::json!({
                    "keys":value.as_object().map(|v| v.keys().collect::<Vec<_>>()),
                    "hasOutbounds":value.get("outbounds").is_some(),
                    "hasRoute":value.get("route").is_some(),
                    "mode":value.get("mode"),
                }),
                "/providers/proxies" => serde_json::json!({"providers":value["providers"].as_object().map(|v|v.len())}),
                _ => serde_json::json!({"proxies":value["proxies"].as_object().map(|v|v.len()),"current":value["proxies"]["GLOBAL"]["now"]}),
            };
            println!("GET {path}: HTTP {status}, {version:?}, {summary}");
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    }.await;
    session.close();
    result
}
