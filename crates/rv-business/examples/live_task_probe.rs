//! Explicit live task probe against an existing RV card and its bound proxy.
//! Never prints tokens or response bodies. No phone App launch or restoration.
use rv_core::{
    StorePaths,
    workspace::{AppId, TaskSpec, WorkspaceStore, task_cron_at},
};
use std::time::{SystemTime, UNIX_EPOCH};
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
    if kind == "sync-products" {
        let tasks = store.tasks(&id)?;
        let result = tokio::runtime::Builder::new_current_thread().enable_all().build()?
            .block_on(rv_business::shop_sync::synchronize(paths.clone(), id.clone(), rv_business::Operation::Products))?;
        assert_eq!(tasks, store.tasks(&id)?, "manual sync must not modify Tasks");
        println!("manual product sync PASS; http_status={:?}; Tasks unchanged", result["http_status"].as_u64());
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
    let time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
    let task = if let Some(task) = store
        .configured_tasks(&id)?
        .into_iter()
        .find(|t| t.kind == *kind)
    {
        store.run_task_now(&task.id)?
    } else {
        rv_business::tasks::enqueue(&paths, &id, &TaskSpec::new(kind, task_cron_at(time)?))?
    };
    let result = rv_business::OfflineRunner::new(None)?.run_task_blocking(&paths, &task);
    let saved = store.task(&task.id)?;
    let http = saved
        .result
        .as_ref()
        .and_then(|v| v["http_status"].as_u64());
    println!(
        "kind={} status={} last_result={} http_status={:?}",
        kind,
        saved.status,
        saved.last_run_status.as_deref().unwrap_or("none"),
        http
    );
    if args.iter().any(|arg| arg == "--stop-after") {
        store.set_task_enabled(&task.id, false)?;
        println!("Probe task stopped; credentials retained");
    }
    if let Err(message) = result {
        return Err(message.into());
    }
    Ok(())
}
