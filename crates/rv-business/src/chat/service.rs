use super::*;
use rv_core::{StorePaths, workspace::*};
use serde_json::json;
use std::{
    collections::HashSet,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
#[derive(Clone)]
pub enum Action {
    Inbox,
    Thread(String),
    Send { conversation: String, text: String },
    MarkRead(String),
}
pub(super) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
impl Action {
    fn kind(&self) -> &'static str {
        match self {
            Self::Inbox => "chat_inbox",
            Self::Thread(_) => "chat_thread",
            Self::Send { .. } => "chat_send",
            Self::MarkRead(_) => "chat_read",
        }
    }
}
impl crate::OfflineRunner {
    pub fn chat_blocking(
        &self,
        paths: &StorePaths,
        id: &AppId,
        action: Action,
    ) -> Result<(), String> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "无法启动聊天执行器")?
            .block_on(self.chat(paths, id, action))
    }
    pub async fn chat(&self, paths: &StorePaths, id: &AppId, action: Action) -> Result<(), String> {
        self.chat_internal(paths, id, action, None, false).await
    }
    pub(crate) async fn chat_interactive(&self,paths:&StorePaths,id:&AppId,action:Action)->Result<(),String> {
        self.chat_internal(paths,id,action,None,false).await
    }
    pub(crate) async fn chat_queued(
        &self,
        paths: &StorePaths,
        id: &AppId,
        action: Action,
        task: &AppTask,
    ) -> Result<(), String> {
        self.chat_internal(paths, id, action, Some(task), true).await
    }
    async fn chat_internal(
        &self,
        paths: &StorePaths,
        id: &AppId,
        action: Action,
        queued: Option<&AppTask>,
        record_task: bool,
    ) -> Result<(), String> {
        let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
        let app = store.load(id).map_err(|e| e.to_string())?;
        if app.archived {
            return Err("账号已归档".into());
        }
        store
            .request_auth(id)
            .map_err(|e| e.to_string())?
            .ok_or("请先读取账号资料")?;
        let plugin = strategy(&app.bundle_id).ok_or("此 App 尚未接入聊天")?;
        let (route, lease) = crate::network::acquire_route(paths, id, &self.client, plugin.requires_proxy()).await?;
        let app = store.load(id).map_err(|e| e.to_string())?;
        if app.archived {
            return Err("账号已归档".into());
        }
        let auth = store
            .request_auth(id)
            .map_err(|e| e.to_string())?
            .ok_or("请先读取账号资料")?;
        let runner = Self {
            client: route.client.clone(),
            credentials: Some((paths.clone(),id.clone())),
            interactive: !record_task,
        };
        runner
            .chat_with_lease(
                &mut store,
                &app,
                &auth,
                plugin.as_ref(),
                action,
                lease,
                queued,
                Some((paths, route)),
            )
            .await
    }
    pub(super) async fn chat_interactive_with(&self,paths:&StorePaths,id:&AppId,action:Action,plugin:&dyn ChatStrategy)->Result<(),String> {
        let mut store=WorkspaceStore::open(paths).map_err(|e|e.to_string())?;
        let app=store.load(id).map_err(|e|e.to_string())?;
        if app.archived{return Err("账号已归档".into())}
        let (route, lease) = crate::network::acquire_route(paths,id,&self.client,plugin.requires_proxy()).await?;
        let app=store.load(id).map_err(|e|e.to_string())?;
        if app.archived{return Err("账号已归档".into())}
        let auth=store.request_auth(id).map_err(|e|e.to_string())?.ok_or("缺少账号凭据")?;
        let runner=Self{client:route.client.clone(),credentials:Some((paths.clone(),id.clone())),interactive:true};
        runner.chat_with_lease(&mut store,&app,&auth,plugin,action,lease,None,Some((paths,route))).await
    }
    #[cfg(test)]
    pub(super) async fn chat_with(
        &self,
        store: &mut WorkspaceStore,
        app: &AppInstance,
        auth: &AppAuth,
        plugin: &dyn ChatStrategy,
        action: Action,
    ) -> Result<(), String> {
        self.chat_with_lease(store, app, auth, plugin, action, None, None, None)
            .await
    }
    async fn chat_with_lease(
        &self,
        store: &mut WorkspaceStore,
        app: &AppInstance,
        auth: &AppAuth,
        plugin: &dyn ChatStrategy,
        action: Action,
        lease: Option<crate::network::Lease>,
        queued: Option<&AppTask>,
        cache_paths: Option<(&StorePaths, crate::network::Route)>,
    ) -> Result<(), String> {
        let mut outbox = None;
        if let Action::Send { conversation, text } = &action {
            if !plugin.capabilities().text {
                return Err("此 App 不支持文字回复".into());
            }
            plugin.reply(conversation, text)?;
        }
        if matches!(action, Action::MarkRead(_)) && !plugin.capabilities().mark_read {
            return Err("此 App 不支持标为已读".into());
        }
        // Resolve the account-scoped conversation before any HTTP call.
        let admit = |db: &rusqlite::Connection| {
            crate::tasks::admit_access_token(
                db,
                app.id.as_str(),
                queued.map(|task| task.id.as_str()),
            )?;
            repository::schema(db)?;
            match &action {
                Action::Inbox => {}
                Action::Send { conversation, text } => {
                    outbox = Some(repository::begin_send(db, &app.id, conversation, text)?)
                }
                Action::Thread(remote) | Action::MarkRead(remote) => {
                    repository::key(db, &app.id, remote)?;
                }
            }
            Ok(())
        };
        let task=if self.interactive {
store.business_transaction(admit).map_err(|e|e.to_string())?;
            None
        } else {
            Some(match queued {
                Some(task)=>store.claim_task_when(task,admit),
                None=>store.start_task_when(&app.id,action.kind(),admit),
            }.map_err(|e|e.to_string())?)
        };
        let budget = Duration::from_secs(queued.map_or(75, |task| u64::from(task.timeout_seconds)));
        let deadline = std::time::Instant::now() + budget;
        let fetched: Result<ChatResult, String> = async {
            let result = tokio::time::timeout(budget, self.chat_fetch(plugin, app, auth, &action))
                .await
                .unwrap_or_else(|_| Err("聊天请求超时，结果未知".into()))?;
            let mut urls = Vec::new();
            let mut conversation_images = |conversation: &Conversation| {
                if let Some(image) = &conversation.avatar {
                    urls.push(image.url.clone());
                }
                urls.extend(
                    conversation
                        .items
                        .iter()
                        .filter_map(|item| item.image.as_ref().map(|image| image.url.clone())),
                );
            };
            match &result {
                ChatResult::Inbox(rows) => {
                    for row in rows {
                        conversation_images(row);
                    }
                }
                ChatResult::Thread(batch) => {
                    conversation_images(&batch.conversation);
                    for message in &batch.messages {
                        urls.extend(message.images.iter().map(|image| image.url.clone()));
                    }
                    urls.extend(
                        batch
                            .participants
                            .iter()
                            .filter_map(|p| p.avatar.as_ref().map(|image| image.url.clone())),
                    );
                }
                _ => {}
            }
            if let Some((paths, route)) = cache_paths {
                crate::assets::cache_business_images_on_route(paths, &app.id, urls, deadline, &route,self.interactive)
                    .await
                    .map_err(|e| format!("图片缓存失败：{e}"))?;
            }
            Ok(result)
        }
        .await;
        drop(lease);
        match fetched {
            Ok(result) => {
                let persist=|db:&rusqlite::Connection| {

                    match result {
                        ChatResult::Inbox(rows)=>{ repository::persist_conversations(db,&app.id,&rows,false)?; if task.is_some(){super::queue::wake(db,&app.id,"chat_threads")?;} },
                        ChatResult::Thread(batch)=>repository::persist_detail(db,&app.id,&batch)?,
                        ChatResult::Send(reply)=>{ let remote = match &action {Action::Send{conversation,..} => plugin.sent_message_id(conversation, &reply), _=>None}; repository::finish_send(db,outbox.as_deref().unwrap(),"submitted",remote.as_deref())?; },
                        ChatResult::Read=>{let Action::MarkRead(remote)=&action else{unreachable!()};let key=repository::key(db,&app.id,remote)?; db.execute("UPDATE chat_conversations SET unread=0,data=json_set(data,'$.unread',json('false')) WHERE id=?1",[key])?;if task.is_some(){super::queue::wake(db,&app.id,"notifications")?;}},
                    }
                    Ok(())
                };
                let completed=if let Some(task)=&task {
                    store.complete_task_with(&app.id,task,"completed",&json!({"operation":action.kind()}),None,None,persist)
                } else {store.commit_business_data(&app.id,&json!({}),persist)};
                if completed.is_err() {
                    let _ = store.business_transaction(|db| {
                        if let Some(outbox) = &outbox {
                            repository::finish_send(db, outbox, "unknown", None)?;
                        }
                        Ok(())
                    });
                    if let Some(task)=&task {let _=store.finish_task(task,"unknown",&json!({"error":"聊天结果保存失败"}));}
                    return Err("聊天结果保存失败，请核对状态".into());
                }
                Ok(())
            }
            Err(error) => {
                // HTTP rejection is definite; transport timeout/malformed success is unknown.
                let image_failure = error.starts_with("图片缓存失败：");
                let status = if image_failure
                    || error
                        .strip_prefix("HTTP ")
                        .and_then(|s| s.split('：').next())
                        .and_then(|s| s.parse::<u16>().ok())
                        .is_some_and(|s| (400..500).contains(&s) && s != 408)
                {
                    "failed"
                } else {
                    "unknown"
                };
                let persist=|db:&rusqlite::Connection| {

                            if let Some(outbox) = outbox {
                                repository::finish_send(db, &outbox, status, None)?;
                            }
                            Ok(())
                };
                if let Some(task)=&task {
                    store.complete_task_with_policy(&app.id,task,status,&json!({"error":error}),None,None,
                        TaskFinishPolicy{retryable:queued.is_some()&&matches!(action,Action::Inbox|Action::Thread(_))&&(status=="unknown"||image_failure),stop_on_failure:status=="failed"&&!image_failure},persist)
                        .map_err(|e|e.to_string())?;
                } else {store.commit_business_data(&app.id,&json!({}),persist).map_err(|e|e.to_string())?;}
                Err(error)
            }
        }
    }
    pub(super) async fn chat_fetch(
        &self,
        plugin: &dyn ChatStrategy,
        app: &AppInstance,
        auth: &AppAuth,
        action: &Action,
    ) -> Result<ChatResult, String> {
        match action {
            Action::Send { conversation, text } => {
                let reply = self
                    .chat_exchange(plugin.reply(conversation, text)?, auth, true)
                    .await?;
                if plugin.sent_message_id(conversation, &reply).is_none() {
                    return Err("消息回执缺少标识，结果未知，请核对是否送达".into());
                }
                return Ok(ChatResult::Send(reply));
            }
            Action::MarkRead(remote) => {
                self.chat_exchange(plugin.mark_read(remote)?, auth, true)
                    .await?;
                return Ok(ChatResult::Read);
            }
            _ => {}
        }
        let mut cursor = None;
        let mut seen = HashSet::new();
        let mut rows = Vec::new();
        let mut detail: Option<DetailBatch> = None;
        let mut bytes = 0;
        for _ in 0..200 {
            let plan = match action {
                Action::Inbox => plugin.inbox(cursor.as_deref())?,
                Action::Thread(remote) => plugin.detail(remote, cursor.as_deref())?,
                _ => unreachable!(),
            };
            let reply = self.chat_exchange(plan, auth, false).await?;
            bytes += reply.to_string().len();
            if bytes > 8 * 1024 * 1024 {
                return Err("聊天同步数据超过限制".into());
            }
            cursor = match action {
                Action::Inbox => {
                    let batch = plugin.parse_inbox(app, &reply)?;
                    rows.extend(batch.rows);
                    batch.next
                }
                Action::Thread(remote) => {
                    let batch = plugin.parse_detail(app, remote, &reply)?;
                    let next = batch.next.clone();
                    if let Some(first) = &mut detail {
                        first.messages.extend(batch.messages);
                        first.participants.extend(batch.participants);
                    } else {
                        detail = Some(batch);
                    }
                    next
                }
                _ => unreachable!(),
            };
            if cursor.is_none() {
                return Ok(match action {
                    Action::Inbox => ChatResult::Inbox(rows),
                    _ => ChatResult::Thread(detail.unwrap()),
                });
            }
            if !seen.insert(cursor.clone().unwrap()) {
                return Err("消息分页游标重复".into());
            }
        }
        Err("消息分页超过限制，未提交不完整数据".into())
    }
    pub(super) async fn chat_exchange(
        &self,
        plan: ChatRequest,
        auth: &AppAuth,
        allow_empty: bool,
    ) -> Result<Value, String> {
        let mut request = crate::device_headers(self.client.request(plan.method, plan.url), auth)
            .bearer_auth(&auth.access_token)
            .header("Accept", "application/json");
        if let Some(body) = plan.body {
            request = request.json(&body);
        }
        self.chat_response(request, auth, allow_empty).await
    }
    pub(super) async fn chat_upload_image(
        &self,
        plugin: &dyn ChatStrategy,
        auth: &AppAuth,
        jpeg: Vec<u8>,
    ) -> Result<String, String> {
        let plan = plugin.image_upload()?;
        let mut form = reqwest::multipart::Form::new();
        for (key, value) in plan.fields {
            form = form.text(key, value);
        }
        form = form.part(
            plan.file_field,
            reqwest::multipart::Part::bytes(jpeg)
                .file_name("image.jpg")
                .mime_str("image/jpeg")
                .map_err(|_| "图片编码错误")?,
        );
        let request = crate::device_headers(self.client.post(plan.url), auth)
            .bearer_auth(&auth.access_token)
            .header("Accept", "application/json")
            .multipart(form);
        let reply = self.chat_response(request, auth, false).await?;
        plugin.image_reference(&reply)
    }
    async fn chat_response(
        &self,
        request: reqwest::RequestBuilder,
        auth: &AppAuth,
        allow_empty: bool,
    ) -> Result<Value, String> {
        let mut response = self.send_authenticated(request,auth,false).await?;
        let endpoint=format!("{}://{}{}",response.url().scheme(),response.url().host_str().unwrap_or("unknown"),response.url().path());
        let status = response.status();
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error|self.http_error("response",&endpoint,"body",auth,error))?
        {
            if bytes.len() + chunk.len() > 1024 * 1024 {
                return Err("聊天回复超过限制".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            let body:Value=serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            let code=body["error_code"].as_str().or_else(||body["error"].as_str()).or_else(||body["error"]["code"].as_str()).map(str::to_owned)
                .or_else(||body["code"].as_i64().map(|code|code.to_string()));
            let description=body["error_description"].as_str().or_else(||body["message"].as_str()).or_else(||body["error"]["message"].as_str()).unwrap_or("聊天请求未成功");
            let message=crate::redact_http_detail(format!("HTTP {}：{}{}",status.as_u16(),code.map(|c|format!("{c} · ")).unwrap_or_default(),description),auth);
            tracing::error!(endpoint,http_status=status.as_u16(),error=%message,"Chat API rejected request");
            return Err(message);
        }
        if bytes.is_empty() && allow_empty {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&bytes).map_err(|error| {
            tracing::error!(endpoint,http_status=status.as_u16(),response_bytes=bytes.len(),error=%error,"Chat response JSON parsing failed");
            format!("聊天回复 JSON 解析失败（HTTP {}，{} 字节，{}）",status.as_u16(),bytes.len(),error)
        })
    }
}
pub(super) enum ChatResult {
    Inbox(Vec<Conversation>),
    Thread(DetailBatch),
    Send(Value),
    Read,
}
