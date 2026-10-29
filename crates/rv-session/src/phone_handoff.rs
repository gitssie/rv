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
            return Err(format!(
                "备份 {part} 回写未确认（HTTP {status}），RV 任务保持停止，可重试交接"
            ));
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
    let expected_data = expected_fields
        .get("archiveInfo")
        .ok_or("本地备份缺少数据归属")?;
    let data_sha = expected_data
        .as_dictionary()
        .and_then(|value| value.get("sha256"))
        .and_then(plist::Value::as_string)
        .ok_or("本地备份数据哈希无效")?;
    let original = crate::app_profile::download(
        client,
        json!({"bundle_id":bundle,"backup_id":backup,"part":"manifest"}),
        MAX_MANIFEST_BYTES,
    )
    .await?;
    let original = plist::Value::from_reader(std::io::Cursor::new(original))
        .map_err(|_| "手机备份清单无效")?;
    let original = original.as_dictionary().ok_or("手机备份清单无效")?;
    if original.get("archiveInfo") != Some(expected_data)
        || original.get("bundleID").and_then(plist::Value::as_string) != Some(bundle)
        || original.get("idfv").and_then(plist::Value::as_string) != Some(id.as_str())
    {
        return Err("手机数据备份与本地来源不一致，未回写 state".into());
    }
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
        local_manifest,
    )
    .await?;
    let manifest = crate::app_profile::download(
        client,
        json!({"bundle_id":bundle,"backup_id":backup,"part":"manifest"}),
        MAX_MANIFEST_BYTES,
    )
    .await?;
    if manifest != local_manifest {
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
        let state = crate::read_shadow_state(client, app_id, bundle, backup).await?;
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
    write_shadow_backup(client, app_id, bundle, operation_id, &state).await?;
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
