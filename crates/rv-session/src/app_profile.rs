//! One explicit phone backup read. Business HTTP tasks do not use this channel.
use crate::{AppCommand, phone_http::PhoneHttpClient};
use rv_core::workspace::AppId;
use serde_json::{Value, json};
#[derive(Clone)]
pub struct AppProfileResponse {
    pub app_id: AppId,
    pub data: Value,
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
            let result = tokio::time::timeout(std::time::Duration::from_secs(300), async move {
                let (status, mut state) = client
                    .json(
                        "/api/app-state/profile",
                        &json!({"bundle_id":bundle,"backup_id":backup}),
                    )
                    .await?;
                if status != 200 || state.get("error").is_some() {
                    return Err("手机未接受账号资料读取，请检查服务或当前备份任务".into());
                }
                let operation = state["operation_id"]
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 128)
                    .ok_or("手机未返回读取任务编号")?
                    .to_string();
                while state["status"] == "running" {
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    let (status, next) = client
                        .json("/api/app-state/result", &json!({"operation_id":operation}))
                        .await?;
                    if status != 200 || next["operation_id"] != operation {
                        return Err("无法确认账号资料读取结果".into());
                    }
                    state = next;
                }
                if state["status"] != "completed" || state.get("error").is_some() {
                    return Err("手机账号资料读取失败，请检查备份完整性".into());
                }
                let data = state["result"].clone();
                if data["bundle_id"] != bundle
                    || data["supported"] != true
                    || AppId::from_idfv(data["idfv"].as_str().unwrap_or(""))
                        .ok()
                        .as_ref()
                        != Some(&app_id)
                {
                    return Err("账号资料与所选 App 身份不匹配".into());
                }
                Ok(AppProfileResponse { app_id, data })
            })
            .await
            .unwrap_or_else(|_| Err("账号资料读取超时，不重复提交".into()));
            let _ = tx.send(result);
        });
        Self {
            command,
            result,
            task,
        }
    }
}
