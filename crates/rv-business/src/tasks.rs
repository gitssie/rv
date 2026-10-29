//! Persistent task dispatch. One tasks row is claimed and completed by the
//! existing business/chat runner; no separate execution log or phone dependency.
use super::*;
enum Action {
    Avatar(String),
    Business(Operation),
    Chat(chat::Action),
    ChatWorkflow(chat::workflow::Workflow),
}
fn action(kind: &str, payload: &Value) -> Result<Action, String> {
    let fields = payload.as_object().ok_or("任务参数必须为 JSON 对象")?;
    if kind == "set_avatar" {
        return fields
            .get("image_id")
            .and_then(Value::as_str)
            .filter(|id| fields.len() == 1 && uuid::Uuid::parse_str(id).is_ok())
            .map(|id| Action::Avatar(id.into()))
            .ok_or_else(|| "头像任务缺少图片".into());
    }
    let operation = match kind {
        "session" => Some(Operation::Session),
        "notification_stats" => Some(Operation::NotificationStats),
        "notifications" => Some(Operation::Notifications),
        "products" => Some(Operation::Products),
        "orders" => Some(Operation::Orders),
        "refresh" => Some(Operation::Refresh),
        _ => None,
    };
    if let Some(operation) = operation {
        if !fields.is_empty() {
            return Err("此操作不接受额外参数".into());
        }
        return Ok(Action::Business(operation));
    }
    if matches!(kind, "chat_send" | "chat_threads" | "chat_read") && fields.is_empty() {
        return Ok(Action::ChatWorkflow(match kind {
            "chat_send" => chat::workflow::Workflow::Send,
            "chat_read" => chat::workflow::Workflow::Read,
            _ => chat::workflow::Workflow::Threads,
        }));
    }
    if matches!(kind, "inbox" | "chat_inbox") && fields.is_empty() {
        return Ok(Action::Chat(chat::Action::Inbox));
    }
    Err("聊天任务为固定处理器，不接受单条消息或会话参数".into())
}

#[cfg(test)]
mod tests;
pub const KEEPALIVE_KEY: &str = "keepalive";

/// Refresh scheduling is independent of tasks using the current access token.
pub fn set_task_enabled(paths: &StorePaths, task_id: &str, enabled: bool) -> Result<AppTask, rv_core::StoreError> {
    WorkspaceStore::open(paths)?.set_task_enabled(task_id, enabled)
}
pub fn run_task_now(paths: &StorePaths, task_id: &str) -> Result<AppTask, rv_core::StoreError> {
    WorkspaceStore::open(paths)?.run_task_now(task_id)
}

pub(crate) fn admit_refresh(db: &rusqlite::Connection, app_id: &str) -> Result<(),rv_core::StoreError> {
    if WorkspaceStore::token_source_in(db, app_id)? != rv_core::workspace::TokenSource::Rv {
        return Err(rv_core::StoreError::Invalid("Token 由手机提供，RV 不刷新令牌".into()));
    }
    Ok(())
}

/// Check shared credential state when a task is actually claimed.
pub(crate) fn admit_access_token(db: &rusqlite::Connection, app_id: &str, task_id: Option<&str>) -> Result<(),rv_core::StoreError> {
    if let Some(task)=task_id {
        let kind:String=db.query_row("SELECT kind FROM tasks WHERE id=?1",[task],|r|r.get(0))?;
        if kind=="refresh" {
            return admit_refresh(db, app_id);
        }
    }
    WorkspaceStore::require_access_token(db,app_id)
}

pub fn enqueue(paths: &StorePaths, id: &AppId, spec: &TaskSpec) -> Result<AppTask, String> {
    let command = action(&spec.kind, &spec.payload)?;
    if spec.max_attempts > 1
        && matches!(
            command,
            Action::Avatar(_)
                | Action::Chat(chat::Action::Send { .. } | chat::Action::MarkRead(_))
                | Action::ChatWorkflow(
                    chat::workflow::Workflow::Send | chat::workflow::Workflow::Read
                )
        )
    {
        return Err("写操作不自动重试，max_attempts 必须为 1".into());
    }
    let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    let app = store.load(id).map_err(|e| e.to_string())?;
    if strategy(&app.bundle_id).is_none() {
        return Err("此 App 尚未接入业务任务".into());
    }
    let fixed = match spec.kind.as_str() {
        "inbox" | "chat_inbox" => Some("inbox"),
        "chat_threads" => Some("chat_threads"),
        "chat_send" => Some("chat_send"),
        "chat_read" => Some("chat_read"),
        _ => None,
    };
    if let Some(key) = fixed {
        drop(store);
        initialize(paths, id)?;
        let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
        let current = store
            .configured_tasks(id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|t| t.task_key.as_deref() == Some(key))
            .ok_or("尚未初始化消息任务")?;
        let timestamp = timestamp();
        store.business_transaction(|db|{db.execute("UPDATE tasks SET status='pending',next_run_at=CASE WHEN status='pending' THEN MIN(next_run_at,?2) ELSE ?2 END,updated_at=?2 WHERE id=?1 AND enabled=1 AND status IN ('pending','completed','failed')",rusqlite::params![current.id,timestamp])?;Ok(())}).map_err(|e|e.to_string())?;
        return store.task(&current.id).map_err(|e| e.to_string());
    }
    let mut spec = spec.clone();
    if spec.kind == "inbox" {
        spec.kind = "chat_inbox".into();
    }
    let existing=store.business_transaction(|db| {
        use rusqlite::OptionalExtension;
        db.query_row("SELECT id FROM tasks WHERE app_id=?1 AND kind=?2 AND payload=?3 AND status IN ('pending','running') ORDER BY created_at LIMIT 1",rusqlite::params![id.as_str(),spec.kind,spec.payload.to_string()],|r|r.get::<_,String>(0)).optional().map_err(Into::into)
    }).map_err(|e|e.to_string())?;
    if let Some(existing) = existing {
        let current = store.task(&existing).map_err(|e| e.to_string())?;
        return if current.status == "pending" {
            store.run_task_now(&existing)
        } else {
            Ok(current)
        }
        .map_err(|e| e.to_string());
    }
    store.enqueue_task(id, &spec).map_err(|e| e.to_string())
}
pub fn due(paths: &StorePaths, timestamp: i64) -> Result<Vec<AppTask>, String> {
    let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    let tasks = store.due_tasks(timestamp).map_err(|e| e.to_string())?;
    let mut ready = Vec::new();
    for task in tasks {
        let id = AppId::from_idfv(&task.app_id).map_err(|e| e.to_string())?;
        let token_rejected = task.kind != "refresh" && store.auth(&id).map_err(|e|e.to_string())?.is_some_and(|auth|auth.access_token_rejected());
        let source = store.token_source(&id).map_err(|e|e.to_string())?;
        if token_rejected || store.auth(&id).map_err(|e|e.to_string())?.is_none() {
            store.request_phone_token(&id, false).map_err(|e|e.to_string())?;
        }
        let blocker = if task.kind == "refresh" && matches!(source, rv_core::workspace::TokenSource::Phone(_)) {
            Some(("token_source", "Token 由手机提供，RV 不刷新令牌"))
        } else if token_rejected {
            Some(("access_token", "访问令牌已失效，等待更新凭据"))
        } else if store.auth(&id).map_err(|e| e.to_string())?.is_none() {
            if matches!(source, rv_core::workspace::TokenSource::Phone(_)) {
                Some(("access_token", "等待手机 Token"))
            } else { Some(("credentials", "等待获取账号凭据")) }
        } else if store
            .proxy_binding(&id)
            .map_err(|e| e.to_string())?
            .is_none()
        {
            Some(("proxy", "等待关联虚拟 App 的代理"))
        } else {
            None
        };
        if let Some((code, message)) = blocker {
            store
                .task_blocker(&task, Some(code), Some(message))
                .map_err(|e| e.to_string())?;
        } else {
            if matches!(
                task.error_code.as_deref(),
                Some("credentials" | "proxy" | "keepalive" | "access_token" | "token_source")
            ) {
                store
                    .task_blocker(&task, None, None)
                    .map_err(|e| e.to_string())?;
                ready.push(store.task(&task.id).map_err(|e| e.to_string())?);
            } else {
                ready.push(task);
            }
        }
    }
    Ok(ready)
}
impl OfflineRunner {
    pub fn run_task_blocking(&self, paths: &StorePaths, task: &AppTask) -> Result<(), String> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "无法启动任务执行器")?
            .block_on(self.run_task(paths, task))
    }
    pub async fn run_task(&self, paths: &StorePaths, task: &AppTask) -> Result<(), String> {
        use tracing::Instrument;
        self.run_task_inner(paths,task).instrument(tracing::info_span!("task",task_id=%task.id,app_id=%task.app_id,kind=%task.kind)).await
    }
    async fn run_task_inner(&self, paths: &StorePaths, task: &AppTask) -> Result<(), String> {
        let id = AppId::from_idfv(&task.app_id).map_err(|e| e.to_string())?;
        let result = async {
            match action(&task.kind, &task.payload)? {
                Action::Avatar(image) => self.avatar_task(paths, &id, task, &image).await,
                Action::Chat(action) => self.chat_queued(paths, &id, action, task).await,
                Action::ChatWorkflow(workflow) => {
                    self.chat_workflow(paths, &id, task, workflow).await
                }
                Action::Business(operation) => {
                    let store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
                    let app = store.load(&id).map_err(|e| e.to_string())?;
                    let plugin = strategy(&app.bundle_id).ok_or("未注册此 App 的业务处理器")?;
                    self.run_admitted(paths, &id, operation, plugin.as_ref(), Some(task))
                        .await
                }
            }
        }
        .await;
        // A preflight failure has not claimed/sent this task. Do not leave it
        // endlessly due, or change a cancelled/edited/busy task.
        if let Err(message) = &result {
            tracing::error!(task_id=%task.id,app_id=%task.app_id,kind=%task.kind,error=%message,"Task execution failed");
            let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
            match store.fail_pending_task(task, message) {
                Ok(()) | Err(rv_core::StoreError::Conflict) => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        let mut store = WorkspaceStore::open(paths).map_err(|e|e.to_string())?;
        if store.auth(&id).map_err(|e|e.to_string())?.is_some_and(|auth|auth.access_token_rejected()) {
            store.wait_for_access_token(&id).map_err(|e|e.to_string())?;
            store.request_phone_token(&id,false).map_err(|e|e.to_string())?;
        }
        result
    }
}

/// Provision a durable workflow once. All scheduling/state lives in tasks.
/// Cancelling a task is respected; provisioning never re-enables it.
pub fn initialize(paths: &StorePaths, id: &AppId) -> Result<(), String> {
    let timestamp = timestamp();
    let mut store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    let app = store.load(id).map_err(|e| e.to_string())?;
    if app.archived
        || app.bundle_id != "lt.manodrabuziai.fr"
    {
        return Ok(());
    }
    // Import has already validated/extracted state. Never create an API workflow
    // for a catalogue-only entry without credentials.
    if store.auth(id).map_err(|e| e.to_string())?.is_none() {
        return Ok(());
    }
    chat::recover_queue(&mut store)?;
    let result = (|| {
        let keepalive_cron = "0 0 * * * *";
        // Automatic refresh has its own schedule; reads can use imported credentials.
        let first = rv_core::workspace::next_task_time(keepalive_cron, timestamp)
            .map_err(|e| e.to_string())?.ok_or("会话保持计划没有下次执行时间")?;
        let mut keepalive_spec = TaskSpec::new("refresh", keepalive_cron);
        keepalive_spec.max_attempts = 3;
        let keepalive = store
            .ensure_task(id, KEEPALIVE_KEY, &keepalive_spec, None, first)
            .map_err(|e| e.to_string())?;
        let mut spec = TaskSpec::new(
            "session",
            rv_core::workspace::task_cron_at(timestamp + 10).map_err(|e| e.to_string())?,
        );
        spec.max_attempts = 3;
        let profile = store
            .ensure_task(id, "profile", &spec, None, timestamp + 10)
            .map_err(|e| e.to_string())?;
        store.business_transaction(|db| {
            db.execute("UPDATE tasks SET depends_on=NULL WHERE id=?1 AND depends_on=?2",rusqlite::params![profile.id,keepalive.id])?;
            Ok(())
        }).map_err(|e|e.to_string())?;
        for (key, kind, cron) in [
            ("products", "products", "0 */15 * * * *"),
            ("orders", "orders", "0 */15 * * * *"),
            ("inbox", "chat_inbox", "0 */5 * * * *"),
            ("chat_threads", "chat_threads", "0 */5 * * * *"),
            ("chat_send", "chat_send", "*/2 * * * * *"),
            ("chat_read", "chat_read", "*/5 * * * * *"),
            ("notifications", "notification_stats", "0 */10 * * * *"),
            ("notification_list", "notifications", "0 */5 * * * *"),
        ] {
            let mut spec = TaskSpec::new(kind, cron);
            spec.max_attempts = if matches!(kind, "chat_send" | "chat_read") {
                1
            } else {
                3
            };
            let task = store
                .ensure_task(id, key, &spec, Some(&profile.id), timestamp + 10)
                .map_err(|e| e.to_string())?;
            let _ = task;
        }
        Ok(())
    })();
    result
}
fn timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
/// Host-owned executor. UI only receives change notifications and reads SQLite.
pub struct TaskWorker {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl TaskWorker {
    pub fn spawn(paths: StorePaths) -> Self {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        std::thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(v) => v,
                Err(_) => {
                    tracing::error!("任务执行器启动失败");
                    return;
                }
            };
            runtime.block_on(async move {
                let changes=match rv_core::events::workspace_bus(&paths) {
                    Ok(bus)=>bus.subscribe::<rv_core::events::WorkspaceChanged>(),
                    Err(error)=>{tracing::error!(%error,"任务事件订阅失败");return;}
                };
                let mut running = std::collections::HashSet::new();
                let mut jobs = tokio::task::JoinSet::new();
                loop {
                    if stopped.load(Ordering::Relaxed) {
                        jobs.abort_all();
                        break;
                    }
                    while let Some(result) = jobs.try_join_next() {
                        match result {
                            Ok((id, _result)) => {
                                running.remove(&id);
                            }
                            Err(error) => tracing::error!(error=%error,"Task worker interrupted"),
                        }
                    }
                    let tasks = (|| {
                        let store = WorkspaceStore::open(&paths).map_err(|e| e.to_string())?;
                        let apps = store
                            .list(&rv_core::workspace::AppQuery::default())
                            .map_err(|e| e.to_string())?;
                        drop(store);
                        for app in apps {
                            initialize(&paths, &app.id)?;
                        }
                        due(&paths, timestamp())
                    })();
                    match tasks {
                        Ok(tasks) => {
                            for task in tasks {
                                if running.insert(task.app_id.clone()) {
                                    let paths = paths.clone();
                                    jobs.spawn(async move {
                                        let result = match OfflineRunner::new(None) {
                                            Ok(runner) => runner.run_task(&paths, &task).await,
                                            Err(e) => Err(e),
                                        };
                                        (task.app_id, result)
                                    });
                                }
                            }
                        }
                        Err(error) => {
                            tracing::error!(error=%error,"Task scheduler failed");
                        }
                    }
                    let delay=tokio::time::sleep(std::time::Duration::from_secs(1));
                    tokio::pin!(delay);
                    loop {
                        tokio::select! {
                            _=&mut delay=>break,
                            event=changes.recv()=>match event {
                                Ok(event) if event.changes.iter().any(|c|c.domain==rv_core::events::ChangeDomain::Credentials)=>break,
                                Ok(_)=>{},
                                Err(_)=>break,
                            }
                        }
                    }
                }
            });
        });
        Self { stop }
    }
}
impl Drop for TaskWorker {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
