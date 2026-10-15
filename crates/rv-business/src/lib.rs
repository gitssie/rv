//! Offline tasks execute on the host. This crate has no phone/session dependency.
pub mod assets;
pub mod chat;
pub mod data;
mod network;
pub mod presentation;
pub mod schedules;
#[cfg(test)]
mod tests;
mod vinted;
use rv_core::{StorePaths, workspace::*};
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Operation {
    Session,
    Inbox,
    NotificationStats,
    Refresh,
    Products,
    Orders,
}
impl Operation {
    pub fn key(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Inbox => "inbox",
            Self::NotificationStats => "notification_stats",
            Self::Refresh => "refresh",
            Self::Products => "products",
            Self::Orders => "orders",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Session => "检测会话",
            Self::Inbox => "查询消息",
            Self::NotificationStats => "查询未读统计",
            Self::Refresh => "刷新令牌",
            Self::Products => "同步商品",
            Self::Orders => "同步订单",
        }
    }
}
pub struct RequestPlan {
    pub method: reqwest::Method,
    pub url: String,
    pub body: Option<String>,
}
pub trait BusinessStrategy: Send + Sync {
    fn request(&self, operation: Operation, auth: &AppAuth) -> Result<RequestPlan, String>;
    fn project(&self, operation: Operation, status: u16, response: &Value) -> Value;
    fn directions(&self, _: Operation) -> Vec<Option<&'static str>> {
        vec![None]
    }
    fn request_page(
        &self,
        operation: Operation,
        auth: &AppAuth,
        _: &AppInstance,
        _: u32,
        _: Option<&str>,
    ) -> Result<RequestPlan, String> {
        self.request(operation, auth)
    }
    fn collect_page(
        &self,
        _: Operation,
        response: Value,
        combined: &mut Value,
        _: u32,
        _: Option<&str>,
    ) -> Result<bool, String> {
        *combined = response;
        Ok(false)
    }
    fn task_result(&self, _: Operation, result: &Value) -> Value {
        result.clone()
    }
    fn persist(
        &self,
        _: &rusqlite::Connection,
        _: &AppId,
        _: Operation,
        _: &Value,
    ) -> Result<(), rv_core::StoreError> {
        Ok(())
    }
    fn read_data(
        &self,
        _: &rusqlite::Connection,
        _: &AppId,
        _: data::Dataset,
        _: &str,
        _: usize,
    ) -> Result<data::BusinessPage, rv_core::StoreError> {
        Err(data::unsupported_data())
    }
}
pub fn strategy(bundle: &str) -> Option<Box<dyn BusinessStrategy>> {
    match bundle {
        "lt.manodrabuziai.fr" => Some(Box::new(vinted::Vinted)),
        _ => None,
    }
}
pub struct OfflineRunner {
    client: reqwest::Client,
}
impl OfflineRunner {
    pub fn new(proxy: Option<&str>) -> Result<Self, String> {
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(40));
        if let Some(proxy) = proxy {
            builder = builder.proxy(reqwest::Proxy::all(proxy).map_err(|_| "代理配置无效")?);
        }
        Ok(Self {
            client: builder.build().map_err(|_| "无法初始化业务 HTTP 客户端")?,
        })
    }
    pub fn run_blocking(
        &self,
        paths: &StorePaths,
        id: &AppId,
        operation: Operation,
    ) -> Result<(), String> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "无法启动任务执行器")?
            .block_on(self.run(paths, id, operation))
    }
    pub fn run_scheduled_blocking(
        &self,
        paths: &StorePaths,
        job: &schedules::ScheduledJob,
    ) -> Result<(), String> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "无法启动任务执行器")?
            .block_on(async {
                let store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
                let app = store.load(&job.app_id).map_err(|e| e.to_string())?;
                let plugin = strategy(&app.bundle_id).ok_or("此 App 尚未接入脱机任务")?;
                self.run_admitted(
                    paths,
                    &job.app_id,
                    job.operation,
                    plugin.as_ref(),
                    Some(job),
                )
                .await
            })
    }
    pub async fn run(
        &self,
        paths: &StorePaths,
        id: &AppId,
        operation: Operation,
    ) -> Result<(), String> {
        if operation == Operation::Inbox
            && chat::strategy(
                &WorkspaceStore::open(paths)
                    .map_err(|e| e.to_string())?
                    .load(id)
                    .map_err(|e| e.to_string())?
                    .bundle_id,
            )
            .is_some()
        {
            return self.chat(paths, id, chat::Action::Inbox).await;
        }
        let store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
        let app = store.load(id).map_err(|e| e.to_string())?;
        let plugin = strategy(&app.bundle_id).ok_or("此 App 尚未接入脱机任务")?;
        self.run_with_strategy(paths, id, operation, plugin.as_ref())
            .await
    }
    async fn run_with_strategy(
        &self,
        paths: &StorePaths,
        id: &AppId,
        operation: Operation,
        plugin: &dyn BusinessStrategy,
    ) -> Result<(), String> {
        self.run_admitted(paths, id, operation, plugin, None).await
    }
    async fn run_admitted(
        &self,
        paths: &StorePaths,
        id: &AppId,
        operation: Operation,
        plugin: &dyn BusinessStrategy,
        scheduled: Option<&schedules::ScheduledJob>,
    ) -> Result<(), String> {
        let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
        let auth = store
            .auth(id)
            .map_err(|e| e.to_string())?
            .ok_or("请先从手机备份读取账号资料")?;
        let app = store.load(id).map_err(|e| e.to_string())?;
        // Validate prerequisites before creating a running task.
        let direction = plugin
            .directions(operation)
            .first()
            .copied()
            .ok_or("没有可执行的业务请求")?;
        plugin.request_page(operation, &auth, &app, 1, direction)?;
        let route = network::route(paths, id, &self.client)?;
        let lease = route.acquire().await;
        // Waiting can outlive a credential rotation or an archive edit.
        let auth = store
            .auth(id)
            .map_err(|e| e.to_string())?
            .ok_or("请先读取账号资料")?;
        let app = store.load(id).map_err(|e| e.to_string())?;
        if app.archived {
            return Err("账号已归档".into());
        }
        plugin.request_page(operation, &auth, &app, 1, direction)?;
        let runner = Self {
            client: route.client,
        };
        let task = store
            .start_task_when(id, operation.key(), |db| {
                if let Some(job) = scheduled {
                    schedules::authorize(db, job)?;
                }
                Ok(())
            })
            .map_err(|e| e.to_string())?;
        let exchange = tokio::time::timeout(
            Duration::from_secs(75),
            runner.collect(plugin, operation, &auth, &app),
        )
        .await
        .unwrap_or_else(|_| Err("业务同步超时，结果未知".into()));
        drop(lease);
        match exchange {
            Ok((status, response)) => {
                let mut rotated = None;
                if operation == Operation::Refresh && (200..300).contains(&status) {
                    let pair = response["access_token"]
                        .as_str()
                        .zip(response["refresh_token"].as_str())
                        .filter(|(a, r)| !a.is_empty() && !r.is_empty());
                    let Some((access, refresh)) = pair else {
                        store
                            .finish_task(
                                &task,
                                "unknown",
                                &json!({"error":"刷新回复未提供完整令牌对"}),
                            )
                            .map_err(|e| e.to_string())?;
                        return Err("刷新结果不完整，请核对凭据".into());
                    };
                    let mut updated = auth.clone();
                    updated.access_token = access.into();
                    updated.refresh_token = Some(refresh.into());
                    updated.expires_at = response["expires_at"].as_i64().or_else(|| {
                        response["expires_in"]
                            .as_i64()
                            .filter(|v| *v >= 0)
                            .and_then(|v| {
                                (std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs() as i64)
                                    .checked_add(v)
                            })
                    });
                    if let Some(fields) = response.as_object() {
                        for (key, value) in fields {
                            if !["access_token", "refresh_token", "headers", "expires_at"]
                                .contains(&key.as_str())
                            {
                                updated.extra.insert(key.clone(), value.clone());
                            }
                        }
                    }
                    rotated = Some(updated);
                }
                let result = plugin.project(operation, status, &response);
                let task_result = plugin.task_result(operation, &result);
                let mut projection = json!({});
                if operation != Operation::Refresh {
                    for key in [
                        "session_status",
                        "user_id",
                        "login",
                        "market",
                        "unread_count",
                    ] {
                        if let Some(value) = result.get(key) {
                            projection[key] = value.clone();
                        }
                    }
                    projection["last_task"] = json!({"kind":operation.key(),"status":status});
                }
                if store
                    .complete_task_with(
                        id,
                        &task,
                        if (200..300).contains(&status) {
                            "completed"
                        } else {
                            "failed"
                        },
                        &task_result,
                        rotated.as_ref(),
                        Some(&projection),
                        |db| plugin.persist(db, id, operation, &result),
                    )
                    .is_err()
                {
                    let _ = store.finish_task(
                        &task,
                        "unknown",
                        &json!({"error":"业务结果保存失败，请核对状态"}),
                    );
                    return Err("业务结果保存失败，请核对状态".into());
                }
                if (200..300).contains(&status) {
                    Ok(())
                } else {
                    Err(format!("业务接口返回 HTTP {status}，任务未成功"))
                }
            }
            Err(message) => {
                store
                    .finish_task(&task, "unknown", &json!({"error":message}))
                    .map_err(|e| e.to_string())?;
                Err(message)
            }
        }
    }
    async fn collect(
        &self,
        plugin: &dyn BusinessStrategy,
        operation: Operation,
        auth: &AppAuth,
        app: &AppInstance,
    ) -> Result<(u16, Value), String> {
        let mut combined = Value::Null;
        for direction in plugin.directions(operation) {
            for page in 1..=200 {
                let plan = plugin.request_page(operation, auth, app, page, direction)?;
                let (status, response) = self.exchange(plan, auth, operation).await?;
                if status != 200 {
                    return Ok((status, response));
                }
                let more =
                    plugin.collect_page(operation, response, &mut combined, page, direction)?;
                if combined.to_string().len() > 8 * 1024 * 1024 {
                    return Err("业务同步数据超过单次限制".into());
                }
                if !more {
                    break;
                }
                if page == 200 {
                    return Err("业务分页超过限制，未提交不完整数据".into());
                }
            }
        }
        Ok((200, combined))
    }
    async fn exchange(
        &self,
        plan: RequestPlan,
        auth: &AppAuth,
        operation: Operation,
    ) -> Result<(u16, Value), String> {
        let mut request = self
            .client
            .request(plan.method, plan.url)
            .header("Accept", "application/json");
        request = device_headers(request, auth);
        if let Some(body) = plan.body {
            request = request
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(body);
        }
        if operation != Operation::Refresh {
            request = request.bearer_auth(&auth.access_token);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| "业务请求中断，结果未知".to_string())?;
        let status = response.status().as_u16();
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "业务回复中断，结果未知")?
        {
            if body.len() + chunk.len() > 1024 * 1024 {
                return Err("业务回复超过限制".into());
            }
            body.extend_from_slice(&chunk);
        }
        let json = serde_json::from_slice(&body).map_err(|_| "业务回复不是 JSON，不能确认结果")?;
        Ok((status, json))
    }
}

pub(crate) fn device_headers(
    mut request: reqwest::RequestBuilder,
    auth: &AppAuth,
) -> reqwest::RequestBuilder {
    for (key, value) in &auth.headers {
        if [
            "X-Session-Id",
            "Locale",
            "Accept-Language",
            "X-Anon-Id",
            "X-V-Udt",
            "X-Device-UUID",
            "User-Agent",
            "Short-Bundle-Version",
            "X-ICloud-Identifier",
            "X-Device-Model",
            "X-App-Version",
        ]
        .contains(&key.as_str())
        {
            request = request.header(key, value);
        }
    }
    request
}
