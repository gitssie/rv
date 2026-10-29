//! Live mobile credentials use the existing authenticated HTTP/2 controller.
use crate::phone_http::PhoneHttpClient;
use rv_core::{ConnectionId, StorePaths, events::*, workspace::*};
use std::{
    collections::HashMap,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) async fn run(paths: StorePaths, device: ConnectionId, client: PhoneHttpClient) {
    let Ok(bus) = workspace_bus(&paths) else {
        return;
    };
    let requests = bus.subscribe::<PhoneTokenRequested>();
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    let mut attempts = HashMap::<String, Instant>::new();
    let mut pending = HashMap::<String, bool>::new();
    loop {
        if pending.is_empty() {
            tokio::select! {
                request = requests.recv() => match request {
                    Ok(r) => { pending.insert(r.app_id.clone(),r.manual); },
                    Err(_) => return,
                },
                _ = interval.tick() => {},
            }
        }
        while let Ok(request) = requests.try_recv() {
            *pending.entry(request.app_id.clone()).or_default() |= request.manual;
        }
        let Ok(store) = WorkspaceStore::open(&paths) else {
            pending.clear();
            continue;
        };
        let Ok(apps) = store.phone_token_apps(device) else {
            pending.clear();
            continue;
        };
        pending.retain(|id, _| {
            apps.iter()
                .any(|app| !app.archived && app.id.as_str() == id)
        });
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        for app in apps.into_iter().filter(|app| !app.archived) {
            let requested = pending.remove(app.id.as_str());
            let mut manual = requested == Some(true);
            let needs_token = store.auth(&app.id).ok().flatten().is_none_or(|auth| {
                auth.access_token_rejected()
                    || auth.access_token.is_empty()
                    || auth
                        .token_expirations()
                        .access
                        .is_some_and(|expiry| expiry <= now)
            });
            if !(manual || needs_token) {
                continue;
            }
            // Coalesce requests from many tasks/401s. One loop owns device reads;
            // manual requests remain available when the user has just opened Vinted.
            if !manual
                && attempts
                    .get(app.id.as_str())
                    .is_some_and(|t| t.elapsed() < Duration::from_secs(10))
            {
                continue;
            }
            attempts.insert(app.id.as_str().into(), Instant::now());
            let result = read_and_save(&paths, device, &client, &app).await;
            // Requests arriving during this read share its result, including a
            // manual click while automatic recovery is already in flight.
            while let Ok(request) = requests.try_recv() {
                if request.app_id == app.id.as_str() {
                    manual |= request.manual;
                } else {
                    *pending.entry(request.app_id.clone()).or_default() |= request.manual;
                }
            }
            if let Err(error) = &result {
                tracing::debug!(app_id=%app.id.as_str(),device_id=%device,%error,"Phone access token unavailable");
            }
            bus.post(PhoneTokenResult {
                app_id: app.id.as_str().into(),
                manual,
                result,
            });
        }
    }
}
async fn read_and_save(
    paths: &StorePaths,
    device: ConnectionId,
    client: &PhoneHttpClient,
    app: &AppInstance,
) -> Result<(), String> {
    let (status, value) = client
        .json(
            "/api/apps/access-token",
            &serde_json::json!({"bundle_id":app.bundle_id}),
        )
        .await.map_err(|error|format!("读取手机 Token 失败：{}",error.replace("；结果可能未知","")))?;
    if status == 404 {
        return Err("手机尚未支持实时 Token 获取，请更新 TrollVNC".into());
    }
    if status != 200 || value.get("error").is_some() {
        let code=value["error"].as_str().unwrap_or("request_failed").chars().take(64).collect::<String>();
        let message=value["message"].as_str().unwrap_or("手机 Token 读取失败").chars().take(512).collect::<String>();
        return Err(format!("读取手机 Token 失败（HTTP {status}，{code}）：{message}"));
    }
    let token: PhoneAccessToken =
        serde_json::from_value(value).map_err(|_| "手机 Token 响应不完整")?;
    if token.bundle_id != app.bundle_id
        || AppId::from_idfv(&token.idfv).map_err(|e| e.to_string())? != app.id
    {
        return Err("手机当前环境不是这个店铺，等待对应 IDFV 环境".into());
    }
    let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    store
        .accept_phone_token(device, &token)
        .map_err(|e| e.to_string())?;
    if store
        .auth(&app.id)
        .map_err(|e| e.to_string())?
        .is_some_and(|auth| auth.access_token_rejected())
    {
        return Err("手机尚未产生新 Token，任务继续等待".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
