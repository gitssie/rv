//! Freeze RV work, sequentially publish state/manifest, then verify both by readback.
use crate::{AppCommand, phone_http::PhoneHttpClient};
use base64::{Engine, engine::general_purpose::STANDARD};
use rv_core::{StoreError, StorePaths, workspace::*};
use serde_json::json;

pub(crate) struct HandoffOperation {
    pub command: AppCommand,
    pub result: tokio::sync::oneshot::Receiver<Result<(), String>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for HandoffOperation {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl HandoffOperation {
    pub fn start(client: PhoneHttpClient, command: AppCommand) -> Self {
        let (sender, result) = tokio::sync::oneshot::channel();
        let input = command.clone();
        let task = tokio::spawn(async move {
            let _ = sender.send(execute(&client, &input).await);
        });
        Self {
            command,
            result,
            task,
        }
    }
}

fn phone_service_failure(label: &str, status: u16, response: &serde_json::Value) -> String {
    let detail=crate::functions::service_error_message(response);
    let unknown=crate::functions::uncertain_result(32,response) || response["outcome_unknown"]==true;
    let suffix=if unknown { "，原操作结果未知，请先查询确认，勿自动重试" } else { "" };
    format!("{label}失败（HTTP {status}）：{detail}{suffix}")
}

async fn app_state_result(client:&PhoneHttpClient,bundle:&str,method:&str)->Result<serde_json::Value,String> {
    let (label,timeout)=match method {"stop"=>("关闭手机 App",15),"list"=>("读取手机备份目录",30),_=>return Err("不支持的手机交接操作".into())};
    tokio::time::timeout(std::time::Duration::from_secs(timeout),async {
        let (status,mut record)=client.json(&format!("/api/app-state/{method}"),&json!({"bundle_id":bundle})).await?;
        if status!=200 {return Err(phone_service_failure(label,status,&record));}
        let operation=record["operation_id"].as_str().filter(|id|!id.is_empty())
            .ok_or_else(||format!("{label}未返回操作标识"))?.to_string();
        loop {
            if record["operation_id"].as_str()!=Some(operation.as_str()) || record["method"]!=method {
                return Err(format!("{label}回执不匹配"));
            }
            if record.get("error").is_some_and(|error|!error.is_null()) {
                let message=phone_service_failure(label,status,&record);
                tracing::error!(method,operation_id=%operation,error=%message,"Phone handoff operation failed");
                return Err(message);
            }
            match record["status"].as_str() {
                Some("completed")=>return record.get("result").filter(|value|value.is_object()).cloned()
                    .ok_or_else(||format!("{label}缺少结果")),
                Some("running"|"pending")=>{},
                _=>return Err(format!("{label}状态无效")),
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let (status,next)=client.json("/api/app-state/result",&json!({"operation_id":operation})).await?;
            if status!=200 {return Err(phone_service_failure(label,status,&next));}
            record=next;
        }
    }).await.unwrap_or_else(|_|Err(format!("{label}超时，未确认完成")))
}

/// A stop ACK alone is insufficient: native completion must confirm that both
/// the main app and its extensions have exited.
async fn stop_phone_app(client:&PhoneHttpClient,bundle:&str)->Result<(),String> {
    let result=app_state_result(client,bundle,"stop").await?;
    if result["bundle_id"]==bundle && result["running"]==false
        && result["main_running"]==false && result["extension_process_count"]==0 {
        Ok(())
    } else {Err("手机 App 进程尚未确认退出，未回写备份".into())}
}

/// Catalog current_idfv is read directly from Shadow preferences by the phone;
/// active detection never reads or depends on access/refresh tokens.
async fn require_inactive_backup(client:&PhoneHttpClient,id:&AppId,bundle:&str)->Result<(),String> {
    let catalog=app_state_result(client,bundle,"list").await?;
    if catalog["bundle_id"]!=bundle || !catalog["backups"].is_array() {
        return Err("手机备份目录与所选 App 不匹配，未交接给 RV".into());
    }
    let value=catalog.get("current_idfv").ok_or("手机服务尚未提供独立的 active 查询，请更新 TrollVNC 至 4.0-16 或以上")?;
    if value.is_null() {return Ok(());}
    let current=AppId::from_idfv(value.as_str().unwrap_or(""))
        .map_err(|_|"手机当前环境 IDFV 无效，未交接给 RV")?;
    if current==*id {
        return Err("该备份仍是手机当前 active 环境。请先在 Shadow 保存最新备份并切换到其他环境，再回来交给 RV".into());
    }
    Ok(())
}


async fn upload(
    client: &PhoneHttpClient,
    bundle: &str,
    backup: &str,
    operation: &str,
    part: &str,
    data_sha: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let sha = state_sha256(bytes);
    let mut offset = 0;
    for chunk in bytes.chunks(192 * 1024) {
        let (status, response) = client.json("/api/app-state/upload", &json!({
            "bundle_id":bundle,"backup_id":backup,"transfer_id":operation.to_ascii_uppercase(),
            "part":part,"offset":offset,"total":bytes.len(),"sha256":sha,"data_sha256":data_sha,"data":STANDARD.encode(chunk),
        })).await?;
        offset += chunk.len();
        if status != 200
            || response["ok"] != true
            || response["received"].as_u64() != Some(offset as u64)
            || response["complete"].as_bool() != Some(offset == bytes.len())
        {
            tracing::error!(http_status = status, part, "Phone backup write rejected");
            return Err(phone_service_failure(&format!("备份 {part} 回写未确认"), status, &response));
        }
    }
    Ok(())
}

pub async fn write_shadow_backup(
    client: &PhoneHttpClient,
    id: &AppId,
    bundle: &str,
    operation: &str,
    state: &ShadowAppState,
) -> Result<(), String> {
    write_parts(
        client,
        id,
        bundle,
        operation,
        state.manifest(),
        state.archive(),
    )
    .await
}
async fn write_parts(
    client: &PhoneHttpClient,
    id: &AppId,
    bundle: &str,
    operation: &str,
    local_manifest: &[u8],
    local_archive: &[u8],
) -> Result<(), String> {
    if !crate::valid_bundle_id(bundle) {return Err("交接 App 身份无效".into());}
    stop_phone_app(client,bundle).await?;
    write_stopped_parts(client,id,bundle,operation,local_manifest,local_archive).await
}
async fn write_stopped_parts(
    client:&PhoneHttpClient,id:&AppId,bundle:&str,operation:&str,
    local_manifest:&[u8],local_archive:&[u8],
)->Result<(),String> {
    let backup = format!("snapshot:{}", id.as_str());
    let expected = plist::Value::from_reader(std::io::Cursor::new(local_manifest))
        .map_err(|_| "本地备份清单无效")?;
    let expected_fields = expected.as_dictionary().ok_or("本地备份清单无效")?;
    if expected_fields
        .get("format")
        .and_then(plist::Value::as_string)
        != Some("ShadowAppState2")
        || expected_fields
            .get("bundleID")
            .and_then(plist::Value::as_string)
            != Some(bundle)
        || expected_fields
            .get("idfv")
            .and_then(plist::Value::as_string)
            != Some(id.as_str())
    {
        return Err("本地 state 与交接目标身份不匹配，未回写".into());
    }
    let original = crate::app_profile::download(
        client,
        json!({"bundle_id":bundle,"backup_id":backup,"part":"manifest"}),
        MAX_MANIFEST_BYTES,
    )
    .await?;
    let original = plist::Value::from_reader(std::io::Cursor::new(original))
        .map_err(|_| "手机备份清单无效")?;
    let original = original.as_dictionary().ok_or("手机备份清单无效")?;
    if original.get("bundleID").and_then(plist::Value::as_string) != Some(bundle)
        || original.get("idfv").and_then(plist::Value::as_string) != Some(id.as_str())
    {
        return Err("手机备份 Bundle ID 或 IDFV 与交接目标不匹配，未回写 state".into());
    }
    // Update state against the phone's current data snapshot, not RV's old copy.
    let data_sha = original
        .get("archiveInfo")
        .and_then(plist::Value::as_dictionary)
        .and_then(|info| info.get("sha256"))
        .and_then(plist::Value::as_string)
        .ok_or("手机备份数据哈希无效")?;
    let mut outgoing = original.clone();
    outgoing.insert(
        "stateArchiveInfo".into(),
        expected_fields.get("stateArchiveInfo")
            .ok_or("本地备份缺少 state 信息")?.clone(),
    );
    let mut outgoing_manifest = Vec::new();
    plist::Value::Dictionary(outgoing).to_writer_binary(&mut outgoing_manifest)
        .map_err(|_| "生成回写清单失败")?;
    upload(
        client,
        bundle,
        &backup,
        operation,
        "state",
        data_sha,
        local_archive,
    )
    .await?;
    upload(
        client,
        bundle,
        &backup,
        operation,
        "manifest",
        data_sha,
        &outgoing_manifest,
    )
    .await?;
    let manifest = crate::app_profile::download(
        client,
        json!({"bundle_id":bundle,"backup_id":backup,"part":"manifest"}),
        MAX_MANIFEST_BYTES,
    )
    .await?;
    if manifest != outgoing_manifest {
        return Err("手机备份清单读回不一致，交接尚未完成".into());
    }
    let archive = crate::app_profile::download(client, json!({"bundle_id":bundle,"backup_id":backup,"part":"state","state_sha256":state_sha256(local_archive)}), MAX_STATE_BYTES).await?;
    if archive.len() != local_archive.len() || state_sha256(&archive) != state_sha256(local_archive)
    {
        return Err("手机 state 读回哈希或大小不一致，交接尚未完成".into());
    }
    Ok(())
}
async fn execute(client: &PhoneHttpClient, command: &AppCommand) -> Result<(), String> {
    if let AppCommand::TakeoverBackup {
        app_id,
        bundle,
        backup,
        store_root,
        device_id,
        operation_id,
    } = command
    {
        if !crate::valid_bundle_id(bundle) || *backup != format!("snapshot:{}", app_id.as_str()) {
            return Err("交接备份身份无效".into());
        }
        let paths = StorePaths::in_dir(store_root.clone());
        let paths_before = paths.clone();
        let id = app_id.clone();
        let device = *device_id;
        let operation = operation_id.clone();
        tokio::task::spawn_blocking(move || {
            WorkspaceStore::open(&paths_before)?.validate_rv_takeover(&id, device, &operation)
        })
        .await
        .map_err(|_| "RV 接管检查已停止")?
        .map_err(|error| error.to_string())?;
        require_inactive_backup(client,app_id,bundle).await?;
        let state = crate::read_shadow_state(client, app_id, bundle, backup).await?;
        require_inactive_backup(client,app_id,bundle).await?;
        let id = app_id.clone();
        let device = *device_id;
        let operation = operation_id.clone();
        return tokio::task::spawn_blocking(move || {
            WorkspaceStore::open(&paths)?.complete_rv_takeover(&id, device, &operation, &state)
        })
        .await
        .map_err(|_| "RV 接管保存未确认")?
        .map_err(|error| error.to_string());
    }
    let AppCommand::HandoffBackup {
        app_id,
        bundle,
        backup,
        store_root,
        device_id,
        operation_id,
    } = command
    else {
        return Err("无效的手机交接命令".into());
    };
    if !crate::valid_bundle_id(bundle) || *backup != format!("snapshot:{}", app_id.as_str()) {
        return Err("交接备份身份无效".into());
    }
    let paths = StorePaths::in_dir(store_root.clone());
    let check_paths=paths.clone();let check_id=app_id.clone();let check_operation=operation_id.clone();let check_device=*device_id;
    tokio::task::spawn_blocking(move|| {
        let store=WorkspaceStore::open(&check_paths)?;let execution=store.execution(&check_id)?;
        if execution.mode!=ExecutionMode::ToPhone || execution.operation_id!=check_operation || execution.device_id!=Some(check_device) {
            return Err(StoreError::Invalid("交接状态或目标手机已改变".into()));
        }
        Ok::<_,StoreError>(())
    }).await.map_err(|_|"交接检查已停止")?.map_err(|error|error.to_string())?;
    stop_phone_app(client,bundle).await?;
    let state = loop {
        let paths = paths.clone();
        let id = app_id.clone();
        let operation = operation_id.clone();
        let device = *device_id;
        let prepared = tokio::task::spawn_blocking(move || {
            let mut store = WorkspaceStore::open(&paths)?;
            let execution = store.execution(&id)?;
            if execution.device_id != Some(device) {
                return Err(StoreError::Invalid("交接目标手机已改变".into()));
            }
            store.prepare_phone_handoff(&id, &operation)
        })
        .await
        .map_err(|_| "交接准备已停止")?;
        match prepared {
            Ok(state) => break state,
            Err(StoreError::Conflict) => {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await
            }
            Err(error) => return Err(error.to_string()),
        }
    };
    write_stopped_parts(client,app_id,bundle,operation_id,state.manifest(),state.archive()).await?;
    let sha = state_sha256(state.archive());
    let id = app_id.clone();
    let operation = operation_id.clone();
    let device = *device_id;
    tokio::task::spawn_blocking(move || {
        WorkspaceStore::open(&paths)?.complete_phone_handoff(&id, &operation, device, &sha)
    })
    .await
    .map_err(|_| "手机备份已校验，但本地接管状态保存未确认，请重试交接")?
    .map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "phone_handoff_tests.rs"]
mod tests;
