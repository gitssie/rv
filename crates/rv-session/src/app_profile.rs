//! Read the selected complete Shadow state. All archive parsing lives in Rust.
use crate::{AppCommand, phone_http::PhoneHttpClient};
use rv_core::workspace::{
    AppDeviceInfo, AppId, MAX_MANIFEST_BYTES, MAX_STATE_BYTES, ShadowAppState,
};
use serde_json::{Value, json};
#[derive(Clone)]
pub struct AppProfileResponse {
    pub app_id: AppId,
    pub state: ShadowAppState,
}
impl std::fmt::Debug for AppProfileResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppProfileResponse")
            .field("app_id", &self.app_id)
            .finish_non_exhaustive()
    }
}
pub(crate) struct ProfileOperation {
    pub command: AppCommand,
    pub result: tokio::sync::oneshot::Receiver<Result<AppProfileResponse, String>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for ProfileOperation {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl ProfileOperation {
    pub fn start(client: PhoneHttpClient, app_id: AppId, bundle: String, backup: String) -> Self {
        let command = AppCommand::ReadProfile {
            app_id: app_id.clone(),
            bundle: bundle.clone(),
            backup: backup.clone(),
        };
        let (tx, result) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let response = read_shadow_state(&client, &app_id, &bundle, &backup)
                .await
                .map(|state| AppProfileResponse { app_id, state });
            let _ = tx.send(response);
        });
        Self {
            command,
            result,
            task,
        }
    }
}
pub(crate) async fn download(
    client: &PhoneHttpClient,
    body: Value,
    maximum: usize,
) -> Result<Vec<u8>, String> {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut response = client
            .api()
            .app_state()
            .download(&body)
            .await
            .map_err(|_| "state 下载连接中断")?;
        if response.status() != 200 {
            let status = response.status().as_u16();
            tracing::error!(
                http_status = status,
                part = body["part"].as_str().unwrap_or(""),
                "Shadow state download rejected by phone"
            );
            return Err(format!(
                "Shadow state 读取失败（HTTP {status}），请检查备份或更新手机服务"
            ));
        }
        let mut data = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "state 下载中断")? {
            if data.len() + chunk.len() > maximum {
                return Err("state 下载超过大小限制".into());
            }
            data.extend_from_slice(&chunk);
        }
        Ok(data)
    })
    .await
    .map_err(|_| "state 下载超时".to_string())?
}
pub async fn read_shadow_state(
    client: &PhoneHttpClient,
    id: &AppId,
    bundle: &str,
    backup: &str,
) -> Result<ShadowAppState, String> {
    read_shadow_state_for(client, id, bundle, backup, "phone").await
}
pub(crate) async fn read_shadow_state_as_rv(
    client: &PhoneHttpClient,
    id: &AppId,
    bundle: &str,
    backup: &str,
) -> Result<ShadowAppState, String> {
    read_shadow_state_for(client, id, bundle, backup, "rv").await
}
async fn read_shadow_state_for(
    client: &PhoneHttpClient,
    id: &AppId,
    bundle: &str,
    backup: &str,
    requester: &str,
) -> Result<ShadowAppState, String> {
    if backup != format!("snapshot:{}", id.as_str()) || !crate::valid_bundle_id(bundle) {
        return Err("Shadow 备份身份不匹配".into());
    }
    let manifest = download(
        client,
        json!({"bundle_id":bundle,"backup_id":backup,"requester":requester,"part":"manifest"}),
        MAX_MANIFEST_BYTES,
    )
    .await?;
    let value = plist::Value::from_reader(std::io::Cursor::new(&manifest))
        .map_err(|_| "Shadow 清单格式无效")?;
    let fields = value.as_dictionary().ok_or("Shadow 清单不是字典")?;
    if fields.get("format").and_then(plist::Value::as_string) != Some("ShadowAppState2")
        || fields.get("bundleID").and_then(plist::Value::as_string) != Some(bundle)
        || fields.get("idfv").and_then(plist::Value::as_string) != Some(id.as_str())
    {
        return Err("Shadow 清单与所选 App 不匹配".into());
    }
    let sha = fields
        .get("stateArchiveInfo")
        .and_then(plist::Value::as_dictionary)
        .and_then(|d| d.get("sha256"))
        .and_then(plist::Value::as_string)
        .ok_or("Shadow 清单缺少 state 哈希")?;
    let archive = download(
        client,
        json!({"bundle_id":bundle,"backup_id":backup,"requester":requester,"part":"state","state_sha256":sha}),
        MAX_STATE_BYTES,
    )
    .await?;
    let info = client
        .api()
        .device()
        .info(&json!({}))
        .await
        .map_err(|_| "读取请求上下文设备信息失败")?;
    let device = AppDeviceInfo {
        model: info["model"].as_str().unwrap_or("").into(),
        os_version: info["os_version"].as_str().unwrap_or("").into(),
    };
    let id = id.clone();
    let bundle = bundle.to_string();
    tokio::task::spawn_blocking(move || {
        ShadowAppState::decode(&id, &bundle, manifest, archive, device).map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "state 解析已停止".to_string())?
}
