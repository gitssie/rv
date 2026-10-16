//! Inspect an existing Photos deletion prompt; --confirm permits one targeted tap.
//! Reuses the saved VNC credential and never starts another deletion.
use rv_core::{AddressBook, StorePaths};
use rv_session::phone_http::PhoneHttpClient;
use serde_json::{Value, json};
use std::time::Duration;
async fn post(
    client: &PhoneHttpClient,
    path: &str,
    body: Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let response = client
        .request(
            reqwest::Method::POST,
            path,
            Default::default(),
            serde_json::to_vec(&body)?,
        )
        .await?;
    let status = response.status();
    let value: Value = serde_json::from_slice(&response.bytes().await?)?;
    if !status.is_success() {
        return Err(format!("request rejected: {status}").into());
    }
    Ok(value)
}
async fn ocr(client: &PhoneHttpClient) -> Result<Value, Box<dyn std::error::Error>> {
    post(
        client,
        "/api/device/ocr",
        json!({"fields":["1","0,,0,,0,,0","","0.008","0","en-US","1",""]}),
    )
    .await
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = std::env::args().nth(1).ok_or("HOST [--confirm]")?;
    let paths = StorePaths::default_dir()?;
    let book = AddressBook::load(paths.clone())?;
    let phone = book
        .connections()
        .iter()
        .find(|phone| phone.host == host)
        .ok_or("phone is not saved")?;
    let password = paths
        .load_password(phone.id)?
        .ok_or("saved VNC password unavailable")?;
    let port = phone.zxtouch_port.unwrap_or(6000);
    let anonymous = PhoneHttpClient::new(&host, port, String::new())?;
    let login = post(
        &anonymous,
        "/api/control/connect",
        json!({"password":password.clone()}),
    )
    .await?;
    let token = login["access_token"]
        .as_str()
        .ok_or("control authentication failed")?;
    let client = PhoneHttpClient::new(&host, port, token.to_owned())?;
    println!("ZXTouch authenticated");
    let response = client
        .request(
            reqwest::Method::POST,
            "/api/device/screenshot",
            Default::default(),
            b"{}".to_vec(),
        )
        .await?;
    if !response.status().is_success() {
        return Err("screenshot failed".into());
    }
    std::fs::write("/tmp/rv-photo-delete-prompt.jpg", response.bytes().await?)?;
    let recognized = ocr(&client).await?;
    let text = recognized["value"].as_str().ok_or("missing OCR reply")?;
    let lower = text.to_lowercase();
    let owner = lower.contains("trollvncserver");
    let deletion = lower.contains("delete") && lower.contains("photo");
    let denial = lower.contains("allow") && (lower.contains("don't") || lower.contains("don’t"));
    let candidates: Vec<_> = text
        .split(";;")
        .filter_map(|line| {
            let parts: Vec<_> = line.split(",,").collect();
            if parts.len() != 5 || !parts[0].trim().eq_ignore_ascii_case("Delete") {
                return None;
            }
            let xywh: Option<Vec<f64>> = parts[1..].iter().map(|v| v.parse().ok()).collect();
            let v = xywh?;
            Some((v[0] + v[2] / 2., v[1] + v[3] / 2.))
        })
        .collect();
    println!(
        "Prompt match: requester={owner}, photo_deletion={deletion}, deny_button={denial}, delete_buttons={}",
        candidates.len()
    );
    println!("Screenshot: /tmp/rv-photo-delete-prompt.jpg");
    if !std::env::args().any(|arg| arg == "--confirm") {
        return Ok(());
    }
    if !(owner && deletion && denial && candidates.len() == 1) {
        return Err("existing prompt did not match; no touch sent".into());
    }
    let (x, y) = candidates[0];
    let caps = post(&client, "/api/control/authorize", json!({})).await?;
    if caps["device_tap"] != 1 {
        return Err("update phone for HTTP/2 one-shot tap".into());
    }
    let reply = post(
        &client,
        "/api/device/tap",
        json!({"x":x,"y":y,"frame":recognized["frame"]}),
    )
    .await?;
    if reply["ok"] != true {
        return Err("tap rejected".into());
    }
    println!("One Delete tap sent at ({x}, {y})");
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let after = ocr(&client).await?;
    let remains = after["value"]
        .as_str()
        .unwrap_or("")
        .to_lowercase()
        .contains("trollvncserver");
    println!("Requester prompt still detected: {remains}");
    if remains {
        return Err("prompt remains after tap; no repeated tap sent".into());
    }
    Ok(())
}
