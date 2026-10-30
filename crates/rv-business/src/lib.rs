//! Offline tasks execute on the host. This crate has no phone/session dependency.
pub mod assets;
pub mod avatar;
pub mod chat;
pub mod data;
mod network;
pub mod notifications;
pub mod presentation;
pub mod proxy;
pub mod shop;
pub mod shop_sync;
mod executor;
pub mod tasks;
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
    Notifications,
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
            Self::Notifications => "notifications",
            Self::Refresh => "refresh",
            Self::Products => "products",
            Self::Orders => "orders",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Session => "刷新资料",
            Self::Inbox => "查询消息",
            Self::NotificationStats => "查询未读数",
            Self::Notifications => "同步通知",
            Self::Refresh => "刷新令牌",
            Self::Products => "同步商品",
            Self::Orders => "同步订单",
        }
    }
}
const CONNECT_FAILURE: &str = "代理连接失败，请检查代理配置后重试";
pub struct RequestPlan {
    pub method: reqwest::Method,
    pub url: String,
    pub body: Option<String>,
}
pub trait BusinessStrategy: Send + Sync {
    fn product_refresh(&self, _: &str) -> Result<Box<dyn BusinessStrategy>, String> {
        Err("此 App 尚未接入单商品刷新".into())
    }
    fn requires_proxy(&self) -> bool {
        false
    }
    fn request(&self, operation: Operation, auth: &AppAuth) -> Result<RequestPlan, String>;
    fn project(&self, operation: Operation, status: u16, response: &Value) -> Value;
    fn is_success(&self,_operation:Operation,status:u16,_result:&Value)->bool {
        (200..300).contains(&status)
    }
    fn image_urls(&self, _operation: Operation, _result: &Value) -> Vec<String> {
        vec![]
    }
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
    credentials: Option<(StorePaths,AppId)>,
    interactive: bool,
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
            credentials: None,
            interactive: false,
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
        self.run_with_strategy(paths,id,operation,plugin.as_ref()).await
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
        queued: Option<&AppTask>,
    ) -> Result<(), String> {
        self.execute_business(paths,id,operation,plugin,queued).await.map(|_|())
    }
    async fn execute_business(
        &self,paths:&StorePaths,id:&AppId,operation:Operation,
        plugin:&dyn BusinessStrategy,queued:Option<&AppTask>,
    )->Result<Value,String> {
        let record_task=queued.is_some();
        let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
        if operation==Operation::Refresh && store.token_source(id).map_err(|e|e.to_string())?!=rv_core::workspace::TokenSource::Rv {
            return Err("Token 由手机提供，RV 不刷新令牌".into());
        }
        let auth = store
            .request_auth(id)
            .map_err(|e| e.to_string())?
            .ok_or("缺少访问令牌，请先获取当前 Token")?;
        let app = store.load(id).map_err(|e| e.to_string())?;
        // Validate prerequisites before creating a running task.
        let direction = plugin
            .directions(operation)
            .first()
            .copied()
            .ok_or("没有可执行的业务请求")?;
        plugin.request_page(operation, &auth, &app, 1, direction)?;
        let (route, lease) = network::acquire_route(paths, id, &self.client, plugin.requires_proxy()).await?;
        // Waiting can outlive a credential rotation or an archive edit.
        let auth = store
            .request_auth(id)
            .map_err(|e| e.to_string())?
            .ok_or("请先读取账号资料")?;
        let app = store.load(id).map_err(|e| e.to_string())?;
        if app.archived {
            return Err("账号已归档".into());
        }
        plugin.request_page(operation, &auth, &app, 1, direction)?;
        let runner = Self {
            client: route.client,
            credentials: Some((paths.clone(),id.clone())),
            interactive: !record_task,
        };
        let task=if let Some(task)=queued {
            Some(store.claim_task_when(task,|db|tasks::admit_access_token(db,id.as_str(),Some(&task.id)))
                .map_err(|e|e.to_string())?)
        } else {
            if operation!=Operation::Refresh && auth.access_token_rejected() {
                return Err("访问令牌已失效，等待更新凭据".into());
            }
            None
        };
        let budget = Duration::from_secs(queued.map_or(75, |task| u64::from(task.timeout_seconds)));
        let deadline = std::time::Instant::now() + budget;
        let exchange: Result<(u16, Value), String> = async {
            let (status, response) =
                tokio::time::timeout(budget, runner.collect(plugin, operation, &auth, &app))
                    .await
                    .unwrap_or_else(|_| Err("业务同步超时，结果未知".into()))?;
            if (200..300).contains(&status) {
                let result = plugin.project(operation, status, &response);
                assets::cache_business_images(
                    paths,
                    id,
                    plugin.image_urls(operation, &result),
                    deadline,
                    !record_task,
                )
                .await
                .map_err(|e| format!("图片缓存失败：{e}"))?;
            }
            Ok((status, response))
        }
        .await;
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
                        if let Some(task)=&task {store.finish_task(task,"unknown",&json!({"error":"刷新回复未提供完整令牌对"})).map_err(|e|e.to_string())?;}
                        return Err("刷新结果不完整，请核对凭据".into());
                    };
                    let mut updated = auth.clone();
                    // Expiry metadata belongs to the new token pair. An opaque
                    // replacement without a server expiry must remain unknown.
                    for key in [
                        "refresh_token_expires_at",
                        "refresh_token_expires_in",
                        "credentials_received_at",
                    ] {
                        updated.extra.remove(key);
                    }
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
                    updated.extra.insert(
                        "credentials_received_at".into(),
                        json!(
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs() as i64
                        ),
                    );
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
                let successful=plugin.is_success(operation,status,&result);
                let task_result = plugin.task_result(operation, &result);
                let mut projection = json!({});
                if operation != Operation::Refresh {
                    for key in [
                        "session_status",
                        "user_id",
                        "login",
                        "market",
                        "unread_count",
                        "notification_unread_count",
                        "shop",
                    ] {
                        if let Some(value) = result.get(key) {
                            projection[key] = value.clone();
                        }
                    }
                }
                if let Some(updated)=&rotated {
                    let adopted=match &task {
                        Some(task)=>store.complete_refresh_task(id,task,&task_result,&auth,updated,&projection),
                        None=>store.complete_refresh_request(id,&auth,updated),
                    }.map_err(|e|e.to_string())?;
                    return if adopted {Ok(task_result)} else {Err("Token 来源或凭据已改变，本次刷新结果未采用".into())};
                }
                let committed=if let Some(task)=&task {
                    if operation == Operation::Refresh && status == 400 && task_result["error_code"] == "invalid_grant" {
                        store.complete_invalid_refresh_task(id, task, &task_result, &auth).map(|_| ())
                    } else {
                    store.complete_task_with_policy(
                        id,task,if successful{"completed"}else{"failed"},
                        &task_result,rotated.as_ref(),Some(&projection),
                        TaskFinishPolicy{retryable:queued.is_some() && operation!=Operation::Refresh && (status==429 || status>=500),stop_on_failure:matches!(status,401|403)},
                        |db|plugin.persist(db,id,operation,&result),
                    )
                    }
                } else {
                    store.commit_business_data(id,&projection,|db|plugin.persist(db,id,operation,&result))
                };
                if let Err(error)=committed {
                    tracing::error!(app_id=%id.as_str(),operation=%operation.key(),%error,"Business result persistence failed");
                    if let Some(task)=&task {let _=store.finish_task(task,"unknown",&json!({"error":"业务结果保存失败，请核对状态"}));}
                    return Err("业务结果保存失败，请核对状态".into());
                }
                if successful {
                    Ok(task_result)
                } else {
                    let message=task_result.get("error").and_then(Value::as_str).map(str::to_owned).unwrap_or_else(||format!("业务接口返回 HTTP {status}"));
                    tracing::error!(app_id=%id.as_str(),task_id=task.as_deref().unwrap_or("manual"),operation=%operation.key(),http_status=status,error_code=task_result["error_code"].as_str().unwrap_or(""),error=%message,"Business request rejected");
                    Err(message)
                }
            }
            Err(message) => {
                let image_failure = message.starts_with("图片缓存失败：");
                let connect_failure = message.starts_with(CONNECT_FAILURE);
                let preflight_failure = message.starts_with("Token 由手机提供") || message.starts_with("账号凭据已更新");
                if let Some(task)=&task {store
                    .finish_task_with_policy(
                        task,
                        if image_failure || connect_failure || preflight_failure {
                            "failed"
                        } else {
                            "unknown"
                        },
                        &json!({"error":message,"not_sent":preflight_failure}),
                        TaskFinishPolicy {
                            retryable: queued.is_some() && !preflight_failure,
                            stop_on_failure: false,
                        },
                    )
                    .map_err(|e| e.to_string())?;}
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
    async fn send_authenticated(&self,request:reqwest::RequestBuilder,auth:&AppAuth,refresh:bool)->Result<reqwest::Response,String> {
        if refresh {
            if let Some((paths,id))=&self.credentials {
                let store=WorkspaceStore::open(paths).map_err(|e|e.to_string())?;
                if store.token_source(id).map_err(|e|e.to_string())? != TokenSource::Rv {
                    return Err("Token 由手机提供，RV 不刷新令牌".into());
                }
                let current=store.auth(id).map_err(|e|e.to_string())?.ok_or("缺少账号凭据")?;
                if current.access_token!=auth.access_token || current.refresh_token!=auth.refresh_token {
                    return Err("账号凭据已更新，本次不使用旧 refresh token，请重试".into());
                }
            }
        }
        if !refresh {
            if let Some((paths,id))=&self.credentials {
                let store=WorkspaceStore::open(paths).map_err(|e|e.to_string())?;
                let current=store.auth(id).map_err(|e|e.to_string())?.ok_or("缺少账号凭据")?;
                if current.access_token_rejected(){store.request_phone_token(id,false).map_err(|e|e.to_string())?;return Err("访问令牌已失效，等待更新凭据".into())}
                if current.access_token!=auth.access_token{return Err("账号凭据已更新，请重试任务".into())}
            }
        }
        let (client,built)=request.build_split();
        let request=built.map_err(|error|self.http_error("unknown","unknown","build",auth,error))?;
        let method=request.method().to_string();
        let mut endpoint=request.url().clone();
        let _=endpoint.set_username("");let _=endpoint.set_password(None);endpoint.set_query(None);endpoint.set_fragment(None);
        let endpoint=endpoint.to_string();
        let response=client.execute(request).await.map_err(|error|self.http_error(&method,&endpoint,"send",auth,error))?;
        if !refresh && response.status()==reqwest::StatusCode::UNAUTHORIZED {
            if let Some((paths,id))=&self.credentials {
                let mut store=WorkspaceStore::open(paths).map_err(|e|e.to_string())?;
                if store.reject_access_token(id,&auth.access_token).map_err(|e|e.to_string())? {
                    store.request_access_token(id).map_err(|e|e.to_string())?;
                }
            }
        }
        Ok(response)
    }
    fn http_error(&self,method:&str,endpoint:&str,phase:&str,auth:&AppAuth,error:reqwest::Error)->String {
        let timeout=error.is_timeout();
        let connect=error.is_connect();
        let prefix=if timeout {"HTTP 请求超时，结果未知"} else if connect {CONNECT_FAILURE} else {"HTTP 请求失败，结果未知"};
        let error=error.without_url();
        let mut detail=error.to_string();
        let mut source=std::error::Error::source(&error);
        while let Some(cause)=source {
            let text=cause.to_string();
            if !text.is_empty() && !detail.ends_with(&text) {detail.push_str("；");detail.push_str(&text);}
            source=cause.source();
        }
        detail=redact_http_detail(detail,auth);
        let app_id=self.credentials.as_ref().map(|(_,id)|id.as_str());
        tracing::error!(app_id,method,endpoint,phase,timeout,connect,error=%detail,"HTTP transport failed");
        format!("{prefix}（{method} {endpoint}；{detail}）")
    }
    async fn exchange(
        &self,
        plan: RequestPlan,
        auth: &AppAuth,
        operation: Operation,
    ) -> Result<(u16, Value), String> {
        let mut request = self
            .client
            .request(plan.method, plan.url);
        request = device_headers(request, auth);
        if let Some(body) = plan.body {
            request = request
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(body);
        }
        if operation != Operation::Refresh {
            request = request.bearer_auth(&auth.access_token);
        }
        let mut response = self.send_authenticated(request,auth,operation==Operation::Refresh).await?;
        let status = response.status().as_u16();
        let mut endpoint=response.url().clone();
        let _=endpoint.set_username("");let _=endpoint.set_password(None);endpoint.set_query(None);endpoint.set_fragment(None);
        let endpoint=endpoint.to_string();
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error|self.http_error("response",&endpoint,"body",auth,error))?
        {
            if body.len() + chunk.len() > 1024 * 1024 {
                return Err("业务回复超过限制".into());
            }
            body.extend_from_slice(&chunk);
        }
        let json: Value = serde_json::from_slice(&body).map_err(|_| "业务回复不是 JSON，不能确认结果")?;
        // Error summaries may quote submitted credentials. Remove known token
        // values before projecting, persisting, displaying or logging errors.
        let json=if status>=400 {
            fn scrub(value:&mut Value,auth:&AppAuth) {
                match value {
                    Value::String(text)=>{
                        if !auth.access_token.is_empty(){*text=text.replace(&auth.access_token,"[redacted]");}
                        if let Some(token)=auth.refresh_token.as_deref().filter(|t|!t.is_empty()){*text=text.replace(token,"[redacted]");}
                    },
                    Value::Array(items)=>for item in items {scrub(item,auth);},
                    Value::Object(fields)=>for (_,value) in fields {scrub(value,auth);},
                    _=>{}
                }
            }
            let mut value=json;scrub(&mut value,auth);value
        }else{json};
        Ok((status, json))
    }
}

pub(crate) fn device_headers(
    mut request: reqwest::RequestBuilder,
    auth: &AppAuth,
) -> reqwest::RequestBuilder {
    request = request.header("Accept", "*/*")
        .header("Accept-Encoding", "gzip, deflate, br");
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

fn redact_http_detail(mut detail:String,auth:&AppAuth)->String {
    for value in std::iter::once(auth.access_token.as_str()).chain(auth.refresh_token.as_deref()) {
        if !value.is_empty(){detail=detail.replace(value,"[redacted]");}
    }
    // Source errors can include URLs. Exclude URL credentials and query strings.
    let mut start=0;
    while let Some(offset)=detail[start..].find("://") {
        let scheme_end=start+offset+3;
        let end=detail[scheme_end..].find(|c:char|c.is_whitespace() || matches!(c,'\''|'"'|')'|']')).map(|n|scheme_end+n).unwrap_or(detail.len());
        let mut url=detail[scheme_end..end].to_string();
        let authority_end=url.find('/').unwrap_or(url.len());
        if let Some(at)=url[..authority_end].rfind('@'){url.replace_range(..=at,"[redacted]@");}
        if let Some(query)=url.find(['?','#']){url.truncate(query);url.push_str("?[redacted]");}
        detail.replace_range(scheme_end..end,&url);start=scheme_end+url.len();
    }
    detail
}
