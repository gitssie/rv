//! Explicit live task probe against an existing RV card and its bound proxy.
//! Never prints tokens or response bodies. No phone App launch or restoration.
use rv_core::{
    StorePaths,
    workspace::{AppId, WorkspaceStore},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let id = AppId::from_idfv(args.first().ok_or("IDFV required")?)?;
    let kind = args.get(1).ok_or("task kind required")?;
    let paths = StorePaths::default_dir()?;
    if !paths.database.is_file() {
        return Err("existing database required".into());
    }
    let mut store = WorkspaceStore::open(&paths)?;
    store.load(&id)?;
    if matches!(kind.as_str(), "refresh-once" | "phone-refresh-once") {
        // Explicit live probes receive private context through stdin, never argv/logs.
        let input: serde_json::Value = serde_json::from_reader(std::io::stdin())?;
        let headers: std::collections::BTreeMap<String, String> = serde_json::from_value(
            if kind == "phone-refresh-once" { input["headers"].clone() } else { input.clone() }
        )?;
        let mut auth = store.request_auth(&id)?.ok_or("missing saved credentials")?;
        let previous = auth.clone();
        if kind == "phone-refresh-once" {
            if input["idfv"].as_str() != Some(id.as_str()) {
                return Err("live phone identity mismatch".into());
            }
            auth.access_token = input["access_token"].as_str().filter(|s| !s.is_empty())
                .ok_or("missing live access token")?.into();
            auth.refresh_token = Some(input["refresh_token"].as_str().filter(|s| !s.is_empty())
                .ok_or("missing live refresh token")?.into());
            auth.expires_at = None;
            auth.extra.remove("access_token_rejected");
            for key in ["expires_in", "expires_at", "refresh_token_expires_at",
                "refresh_token_expires_in", "credentials_received_at"] {
                auth.extra.remove(key);
            }
        }
        for (key, value) in headers {
            if ["Locale", "Accept-Language", "X-Anon-Id", "X-V-Udt", "X-Device-UUID",
                "User-Agent", "Short-Bundle-Version", "X-ICloud-Identifier", "X-Device-Model",
                "X-App-Version"].contains(&key.as_str()) {
                auth.headers.insert(key, value);
            }
        }
        if kind == "refresh-once" {
            assert_eq!(auth.access_token, previous.access_token);
            assert_eq!(auth.refresh_token, previous.refresh_token);
            assert_eq!(auth.headers.get("X-Session-Id"), previous.headers.get("X-Session-Id"));
        }
        store.save_auth(&id, &auth)?;
        let existing = store.tasks(&id)?;
        let result = rv_business::OfflineRunner::new(None)?
            .run_blocking(&paths, &id, rv_business::Operation::Refresh);
        assert_eq!(store.tasks(&id)?.len(),existing.len(),"direct refresh must not create Tasks");
        for task in existing {
            assert_eq!(store.task(&task.id)?.enabled, task.enabled,
                "probe must not change existing task switches");
        }
        println!("direct refresh outcome: {}", result.err().unwrap_or_else(|| "success".into()));
        return Ok(());
    }
    if kind == "sync-products" {
        let tasks = store.tasks(&id)?;
        let result = tokio::runtime::Builder::new_current_thread().enable_all().build()?
            .block_on(rv_business::shop_sync::synchronize(paths.clone(), id.clone(), rv_business::Operation::Products))?;
        assert_eq!(tasks, store.tasks(&id)?, "manual sync must not modify Tasks");
        println!("manual product sync PASS; http_status={:?}; Tasks unchanged", result["http_status"].as_u64());
        return Ok(());
    }
    if kind == "sync-product" {
        let product_id = args.get(2).ok_or("product ID required")?.clone();
        let tasks = store.tasks(&id)?;
        let result = tokio::runtime::Builder::new_current_thread().enable_all().build()?
            .block_on(rv_business::shop_sync::refresh_product(paths.clone(), id.clone(), product_id))?;
        assert_eq!(tasks, store.tasks(&id)?, "manual product refresh must not modify Tasks");
        println!("single product refresh PASS; http_status={:?}; Tasks unchanged", result["http_status"].as_u64());
        return Ok(());
    }
    if kind == "stop-invalid-refresh" {
        for task in store.tasks(&id)? {
            if task.kind == "refresh" && task.error_code.as_deref() == Some("invalid_grant")
                && task.result.as_ref().is_some_and(|value| value["http_status"] == 400) {
                store.set_task_enabled(&task.id, false)?;
                println!("Stopped refresh task {}; invalid_grant result retained", task.id);
            }
        }
        return Ok(());
    }
    if kind == "auth-info" {
        let auth = store.auth(&id)?.ok_or("missing auth")?;
        println!("token expiry metadata: {:?}", auth.token_expirations());
        println!(
            "server expiry field names: {:?}",
            auth.extra
                .keys()
                .filter(|key| key.contains("expir"))
                .collect::<Vec<_>>()
        );
        return Ok(());
    }
    if !matches!(
        kind.as_str(),
        "refresh" | "session" | "products" | "orders" | "chat_inbox" | "notification_stats"
    ) {
        return Err("unsupported probe task".into());
    }
    if store.proxy_binding(&id)?.is_none() {
        return Err("bound proxy required".into());
    }
    let before=store.tasks(&id)?;
    let operation=match kind.as_str() {
        "refresh"=>rv_business::Operation::Refresh,
        "session"=>rv_business::Operation::Session,
        "products"=>rv_business::Operation::Products,
        "orders"=>rv_business::Operation::Orders,
        "chat_inbox"=>rv_business::Operation::Inbox,
        "notification_stats"=>rv_business::Operation::NotificationStats,
        _=>unreachable!(),
    };
    let result=rv_business::OfflineRunner::new(None)?.run_blocking(&paths,&id,operation);
    assert_eq!(store.tasks(&id)?.len(),before.len(),"direct requests must not create Tasks");
    for task in before {
        assert_eq!(store.task(&task.id)?.enabled,task.enabled,"probe must not change existing task switches");
    }
    println!("direct {kind}: {}",result.as_ref().err().map(String::as_str).unwrap_or("success"));
    result.map_err(Into::into)
}
