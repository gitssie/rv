//! Read-only real-device test of ZXTouch Vinted backup metadata.
//! Never saves/restores/deletes backups or exports keychain records.
//! Reuses RV's local saved credentials; never prints tokens or configuration secrets.
use rv_core::{ConnectRequest, Connection, StorePaths};
use rv_session::{AppEvent, SessionEvent, SessionHandle, phone_http::PhoneHttpClient};
use serde_json::Value;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = std::env::args()
        .nth(1)
        .ok_or("usage: live_app_state_backup_probe HOST")?;
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
        let apps = post(&client, "/api/apps/list", serde_json::json!({})).await?;
        let candidates: Vec<&Value> = apps["apps"].as_array().ok_or("missing apps array")?
            .iter().filter(|app| app["name"].as_str().unwrap_or("").to_lowercase().contains("vinted")
                || app["bundle_id"].as_str().unwrap_or("").to_lowercase().contains("vinted")).collect();
        if candidates.len() != 1 { return Err(format!("expected one installed Vinted app, found {}", candidates.len()).into()); }
        let bundle = candidates[0]["bundle_id"].as_str().ok_or("missing bundle id")?;
        println!("Installed Vinted: {bundle}");
        let mut operation = post(&client, "/api/app-state/list", serde_json::json!({"bundle_id":bundle})).await?;
        let identifier = operation["operation_id"].as_str().ok_or_else(|| format!("list rejected: {}", operation.get("error").unwrap_or(&Value::Null)))?.to_owned();
        let deadline = Instant::now() + Duration::from_secs(60);
        while operation["status"].as_str() != Some("completed") {
            if Instant::now() >= deadline { return Err("backup list timed out; no operation replayed".into()); }
            tokio::time::sleep(Duration::from_millis(250)).await;
            operation = post(&client, "/api/app-state/result", serde_json::json!({"operation_id":identifier})).await?;
            if operation.get("status").is_none() { return Err(format!("result rejected: {}", operation.get("error").unwrap_or(&Value::Null)).into()); }
        }
        if let Some(error) = operation.get("error") { return Err(format!("backup list failed: {error}").into()); }
        let result = operation.get("result").ok_or("missing result")?;
        if result["bundle_id"].as_str() != Some(bundle) { return Err("wrong bundle in result".into()); }
        let backups = result["backups"].as_array().ok_or("missing backups array")?;
        println!("Backup list PASS: count={}, current={}, restore_pending={}", backups.len(), result["current_backup_id"], result["restore_pending"]);
        for item in backups {
            let id = item["backup_id"].as_str().ok_or("missing backup id")?;
            if !id.starts_with("snapshot:") && !id.starts_with("baseline:") { return Err("invalid backup id".into()); }
            println!("{}", serde_json::json!({"backup_id":id,"kind":item["kind"],"display_name":item["display_name"],"date":item["date"],"version":item["version"],"build":item["build"]}));
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    }.await;
    session.close();
    result
}

async fn post(
    client: &PhoneHttpClient,
    path: &str,
    body: Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("content-type", "application/json".parse()?);
        let mut response = client
            .request(
                reqwest::Method::POST,
                path,
                headers,
                serde_json::to_vec(&body)?,
            )
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("{path}: HTTP {status}").into());
        }
        if response.version() != reqwest::Version::HTTP_2 {
            return Err("expected HTTP/2".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 1024 * 1024 {
                return Err("response exceeds limit".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes)?;
        if !value.is_object() {
            return Err("expected JSON object".into());
        }
        Ok::<_, Box<dyn std::error::Error>>(value)
    })
    .await
    .map_err(|_| "HTTP request timed out")?
}
