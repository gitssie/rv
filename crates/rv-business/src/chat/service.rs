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
        let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
        let app = store.load(id).map_err(|e| e.to_string())?;
        if app.archived {
            return Err("账号已归档".into());
        }
        store
            .auth(id)
            .map_err(|e| e.to_string())?
            .ok_or("请先读取账号资料")?;
        let plugin = strategy(&app.bundle_id).ok_or("此 App 尚未接入聊天")?;
        let route = crate::network::route(paths, id, &self.client)?;
        let lease = route.acquire().await;
        let app = store.load(id).map_err(|e| e.to_string())?;
        if app.archived {
            return Err("账号已归档".into());
        }
        let auth = store
            .auth(id)
            .map_err(|e| e.to_string())?
            .ok_or("请先读取账号资料")?;
        let runner = Self {
            client: route.client,
        };
        runner
            .chat_with_lease(&mut store, &app, &auth, plugin.as_ref(), action, lease)
            .await
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
        self.chat_with_lease(store, app, auth, plugin, action, None)
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
        let task = store
            .start_task_when(&app.id, action.kind(), |db| {
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
            })
            .map_err(|e| e.to_string())?;
        let fetched = tokio::time::timeout(
            Duration::from_secs(75),
            self.chat_fetch(plugin, app, auth, &action),
        )
        .await
        .unwrap_or_else(|_| Err("聊天请求超时，结果未知".into()));
        drop(lease);
        match fetched {
            Ok(result) => {
                let completed=store.complete_task_with(&app.id,&task,"completed",&json!({"operation":action.kind()}),None,None,|db| {
                    match result {
                        ChatResult::Inbox(rows)=>repository::persist_conversations(db,&app.id,&rows,false)?,
                        ChatResult::Thread(batch)=>repository::persist_detail(db,&app.id,&batch)?,
                        ChatResult::Send(reply)=>{ let remote = match &action {Action::Send{conversation,..} => plugin.sent_message_id(conversation, &reply), _=>None}; repository::finish_send(db,outbox.as_deref().unwrap(),"submitted",remote.as_deref())?; },
                        ChatResult::Read=>{let Action::MarkRead(remote)=&action else{unreachable!()};let key=repository::key(db,&app.id,remote)?; db.execute("UPDATE chat_conversations SET unread=0,data=json_set(data,'$.unread',json('false')) WHERE id=?1",[key])?;},
                    }
                    Ok(())
                });
                if completed.is_err() {
                    let _ = store.business_transaction(|db| {
                        if let Some(outbox) = &outbox {
                            repository::finish_send(db, outbox, "unknown", None)?;
                        }
                        Ok(())
                    });
                    let _ =
                        store.finish_task(&task, "unknown", &json!({"error":"聊天结果保存失败"}));
                    return Err("聊天结果保存失败，请核对状态".into());
                }
                Ok(())
            }
            Err(error) => {
                // HTTP rejection is definite; transport timeout/malformed success is unknown.
                let status = if error
                    .strip_prefix("HTTP ")
                    .and_then(|s| s.split('：').next())
                    .and_then(|s| s.parse::<u16>().ok())
                    .is_some_and(|s| (400..500).contains(&s) && s != 408)
                {
                    "failed"
                } else {
                    "unknown"
                };
                store
                    .complete_task_with(
                        &app.id,
                        &task,
                        status,
                        &json!({"error":error}),
                        None,
                        None,
                        |db| {
                            if let Some(outbox) = outbox {
                                repository::finish_send(db, &outbox, status, None)?;
                            }
                            Ok(())
                        },
                    )
                    .map_err(|e| e.to_string())?;
                Err(error)
            }
        }
    }
    async fn chat_fetch(
        &self,
        plugin: &dyn ChatStrategy,
        app: &AppInstance,
        auth: &AppAuth,
        action: &Action,
    ) -> Result<ChatResult, String> {
        match action {
            Action::Send { conversation, text } => {
                return self
                    .chat_exchange(plugin.reply(conversation, text)?, auth, true)
                    .await
                    .map(ChatResult::Send);
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
    async fn chat_exchange(
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
        let mut response = request.send().await.map_err(|_| "聊天请求中断，结果未知")?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("HTTP {}：聊天请求未成功", status.as_u16()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "聊天回复中断，结果未知")?
        {
            if bytes.len() + chunk.len() > 1024 * 1024 {
                return Err("聊天回复超过限制".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty() && allow_empty {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&bytes).map_err(|_| "聊天回复不是 JSON，结果未知".into())
    }
}
enum ChatResult {
    Inbox(Vec<Conversation>),
    Thread(DetailBatch),
    Send(Value),
    Read,
}
