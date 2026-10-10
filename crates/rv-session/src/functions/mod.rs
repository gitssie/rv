//! Bounded ZXTouch native-service operations, separate from the RFB frame loop.
mod account;
mod code_url;
mod mailbox;
mod spec;
use code_url::fetch_code;
pub use code_url::{extract_verification_code, validate_code_url};
#[cfg(test)]
mod tests;
pub use spec::{OPERATIONS, OperationSpec, PreparedOperation, parse_operation, valid_code};

use crate::zxtouch::ControlEndpoint;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    sync::mpsc,
};

#[derive(Clone)]
pub enum FunctionCommand {
    RefreshAccountState,
    Scoped {
        id: String,
        command: Box<FunctionCommand>,
    },
    Invoke {
        id: String,
        text: String,
    },
    Resume {
        task: u8,
        operation_id: String,
    },
    SubmitCode {
        operation_id: String,
        challenge_id: String,
        code: String,
    },
    FetchCode,
    ReadEmailCode {
        text: String,
    },
    RebindEmail {
        text: String,
    },
    CancelChallenge {
        operation_id: String,
        challenge_id: String,
    },
}
impl std::fmt::Debug for FunctionCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FunctionCommand { contents: [redacted] }")
    }
}
#[derive(Clone)]
pub enum FunctionEvent {
    AccountStateLoading,
    AccountState(Result<Value, String>),
    ForOperation {
        id: String,
        event: Box<FunctionEvent>,
    },
    Available(bool),
    Progress(String),
    EmailCodePending(Value),
    VerificationCodeReceived(String),
    EmailCodeFetched(String),
    State {
        task: u8,
        record: Value,
    },
    Error(String),
    Uncertain(String),
}
impl std::fmt::Debug for FunctionEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AccountStateLoading => f.write_str("AccountStateLoading"),
            Self::AccountState(_) => f.write_str("AccountState([redacted])"),
            Self::ForOperation { id, .. } => f.debug_tuple("ForOperation").field(id).finish(),
            Self::EmailCodeFetched(_) => f.write_str("EmailCodeFetched([redacted])"),
            Self::VerificationCodeReceived(_) => {
                f.write_str("VerificationCodeReceived([redacted])")
            }
            Self::Available(v) => f.debug_tuple("Available").field(v).finish(),
            Self::Progress(v) => f.debug_tuple("Progress").field(v).finish(),
            Self::EmailCodePending(v) => f.debug_tuple("EmailCodePending").field(v).finish(),
            Self::State { task, record } => f
                .debug_struct("State")
                .field("task", task)
                .field("record", record)
                .finish(),
            Self::Error(v) => f.debug_tuple("Error").field(v).finish(),
            Self::Uncertain(v) => f.debug_tuple("Uncertain").field(v).finish(),
        }
    }
}
pub(crate) struct FunctionRuntime {
    host: String,
    port: Option<u16>,
    allowed: bool,
    endpoint: Option<ControlEndpoint>,
    events: mpsc::Sender<FunctionEvent>,
    rx: mpsc::Receiver<FunctionEvent>,
    workers: std::collections::HashMap<String, mpsc::Sender<FunctionCommand>>,
    routes: std::collections::HashMap<String, String>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    last_family: String,
    announce: bool,
    account_refresh_pending: bool,
    account_refresh_again: bool,
}
impl Drop for FunctionRuntime {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
fn operation_family(id: &str) -> &str {
    if id == "email-verify" {
        "email-add"
    } else {
        id
    }
}
impl FunctionRuntime {
    pub fn new(host: String, port: Option<u16>, allowed: bool) -> Self {
        let (events, rx) = mpsc::channel(64);
        Self {
            host,
            port,
            allowed,
            endpoint: None,
            events,
            rx,
            workers: Default::default(),
            routes: Default::default(),
            tasks: vec![],
            last_family: String::new(),
            announce: false,
            account_refresh_pending: false,
            account_refresh_again: false,
        }
    }
    pub fn configure(&mut self, caps: &crate::AppCapabilities) {
        if self.endpoint.is_some() || !self.allowed || !caps.control {
            return;
        }
        let Some(mut endpoint) = caps.zx_control.clone() else {
            return;
        };
        if endpoint.version != 1
            || endpoint.port == 0
            || endpoint.token.len() != 64
            || !endpoint.token.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return;
        }
        if let Some(port) = self.port {
            endpoint.port = port;
        }
        self.endpoint = Some(endpoint);
        self.announce = true;
        self.refresh_account_state();
    }
    fn refresh_account_state(&mut self) {
        if self.account_refresh_pending {
            self.account_refresh_again = true;
            return;
        }
        let Some(endpoint) = self.endpoint.clone() else { return; };
        self.account_refresh_pending = true;
        let _ = self.events.try_send(FunctionEvent::AccountStateLoading);
        let host = self.host.clone();
        let events = self.events.clone();
        self.tasks.retain(|task| !task.is_finished());
        self.tasks.push(tokio::spawn(async move {
            let state = tokio::time::timeout(Duration::from_secs(20), account::query_account_state(&host, &endpoint))
                .await.unwrap_or_else(|_| Err("账户状态读取超时".into()));
            let _ = events.send(FunctionEvent::AccountState(state)).await;
        }));
    }
    pub fn command(&mut self, command: FunctionCommand, send: &impl Fn(FunctionEvent)) {
        let (scope, command) = match command {
            FunctionCommand::Scoped { id, command } => (Some(id), *command),
            c => (None, c),
        };
        if matches!(command, FunctionCommand::RefreshAccountState) {
            self.refresh_account_state();
            return;
        }
        let family = scope.unwrap_or_else(|| match &command {
            FunctionCommand::Invoke { id, .. } => id.clone(),
            FunctionCommand::ReadEmailCode { .. } | FunctionCommand::RebindEmail { .. } => {
                "email-add".into()
            }
            FunctionCommand::Resume { operation_id, .. }
            | FunctionCommand::SubmitCode { operation_id, .. }
            | FunctionCommand::CancelChallenge { operation_id, .. } => self
                .routes
                .get(operation_id)
                .cloned()
                .unwrap_or_else(|| self.last_family.clone()),
            _ => self.last_family.clone(),
        });
        let family = operation_family(&family).to_owned();
        let Some(endpoint) = self.endpoint.clone() else {
            send(FunctionEvent::Error(
                "功能连接不可用，请检查 ZXTouch 端口并重新连接".into(),
            ));
            return;
        };
        if self
            .workers
            .get(&family)
            .is_none_or(|sender| sender.is_closed())
        {
            let (tx, commands) = mpsc::channel(8);
            let (events, mut rx) = mpsc::channel(32);
            let shared = self.events.clone();
            let key = family.clone();
            let host = self.host.clone();
            self.tasks.push(tokio::spawn(async move {
                let forward = async move {
                    while let Some(event) = rx.recv().await {
                        if shared
                            .send(FunctionEvent::ForOperation {
                                id: key.clone(),
                                event: Box::new(event),
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                };
                tokio::join!(worker(host, endpoint, commands, events), forward);
            }));
            self.workers.insert(family.clone(), tx);
        }
        self.last_family = family.clone();
        if self.workers[&family].try_send(command).is_err() {
            send(FunctionEvent::ForOperation {
                id: family,
                event: Box::new(FunctionEvent::Error("此功能请求繁忙，请稍后重试".into())),
            });
        }
    }
    pub fn advance(&mut self, send: &impl Fn(FunctionEvent)) {
        if self.announce {
            self.announce = false;
            send(FunctionEvent::Available(true));
        }
        while let Ok(event) = self.rx.try_recv() {
            if matches!(event, FunctionEvent::AccountState(_)) {
                self.account_refresh_pending = false;
                if std::mem::take(&mut self.account_refresh_again) { self.refresh_account_state(); }
            }
            if let FunctionEvent::ForOperation { id, event: inner } = &event {
                let record = match inner.as_ref() {
                    FunctionEvent::State { record, .. }
                    | FunctionEvent::EmailCodePending(record) => Some(record),
                    _ => None,
                };
                if record.is_some_and(|r| r["status"] == "completed" && matches!(r["method"].as_str(),
                    Some("signInAppStore" | "enableICloud" | "signOutAppStore" | "signOutAppleID" | "signOutICloud" | "disableFindMy"))) {
                    self.refresh_account_state();
                }
                if let Some(op) = record.and_then(|r| r["operation_id"].as_str()) {
                    self.routes.insert(op.into(), id.clone());
                }
            }
            send(event);
        }
    }
}

pub fn email_binding_prepared(record: &Value) -> bool {
    record["status"] == "completed"
        && record["method"] == "beginAddingContactEmail"
        && record.get("error").is_none()
        && (record["result"]["outcome"] == "succeeded"
            || (record["result"]["outcome"] == "requires_email_verification"
                && record["result"]["challenge_id"]
                    .as_str()
                    .is_some_and(|id| !id.is_empty() && id.len() <= 128)))
}
pub fn uncertain_result(task: u8, record: &Value) -> bool {
    let error = &record["error"];
    record["result"]["outcome"] == "unknown"
        || matches!(
            error["code"].as_str(),
            Some("outcome_unknown" | "missing_result")
        )
        || (task == 31
            && error["domain"] == "org.trollvnc.apple"
            && error["code"].as_i64() == Some(6))
        || (task == 32
            && error["domain"] == "TVAppStateServices"
            && error["code"].as_i64().is_some_and(|code| code >= 3))
        || error["state_may_be_partial"] == true
        || error["archive_may_be_partial"] == true
}
fn service_error(response: &Value) -> String {
    let code = response["error"]["code"]
        .as_str()
        .filter(|s| s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
        .unwrap_or("unknown");
    format!("原生请求失败：{code}")
}
fn accepted_code(response: &Value, id: &str) -> Result<(), String> {
    if response.get("error").is_some() {
        return Err(service_error(response));
    }
    if response["operation_id"] != id || response["accepted"] != true {
        return Err("验证码提交未确认，请查询原操作结果".into());
    }
    Ok(())
}

struct Wire {
    stream: BufReader<TcpStream>,
}
impl Wire {
    async fn connect(host: &str, endpoint: &ControlEndpoint) -> Result<Self, String> {
        tokio::time::timeout(Duration::from_secs(5), async {
            let stream = TcpStream::connect((host, endpoint.port))
                .await
                .map_err(|_| "ZXTouch 连接失败")?;
            stream
                .set_nodelay(true)
                .map_err(|_| "ZXTouch 连接设置失败")?;
            let mut wire = Self {
                stream: BufReader::new(stream),
            };
            let response = wire
                .request(
                    33,
                    json!({"version":1,"token":endpoint.token,"op":0,"id":0,"payload":{}}),
                )
                .await?;
            if response["version"] != 1
                || response["op"] != 0
                || response["id"] != 0
                || response["status"] != 0
            {
                return Err("ZXTouch 授权失败".into());
            }
            Ok(wire)
        })
        .await
        .map_err(|_| "ZXTouch 连接超时")?
    }
    async fn request(&mut self, task: u8, body: Value) -> Result<Value, String> {
        tokio::time::timeout(Duration::from_secs(10), async {
            let json = body.to_string();
            if json.len() > 32768 {
                return Err("功能请求过大".into());
            }
            let wire = format!("{task}{}\r\n", STANDARD.encode(json));
            self.stream
                .get_mut()
                .write_all(wire.as_bytes())
                .await
                .map_err(|_| "ZXTouch 请求中断；结果可能未知")?;
            let mut line = Vec::new();
            loop {
                let available = self
                    .stream
                    .fill_buf()
                    .await
                    .map_err(|_| "ZXTouch 回复中断；结果可能未知")?;
                if available.is_empty() {
                    return Err("ZXTouch 连接已关闭；结果可能未知".into());
                }
                let count = available
                    .iter()
                    .position(|b| *b == b'\n')
                    .map_or(available.len(), |i| i + 1);
                if line.len() + count > 950000 {
                    return Err("功能回复超过限制".into());
                }
                line.extend_from_slice(&available[..count]);
                self.stream.consume(count);
                if line.last() == Some(&b'\n') {
                    break;
                }
            }
            let encoded = line
                .strip_prefix(b"0;;")
                .and_then(|l| l.strip_suffix(b"\r\n"))
                .ok_or("功能回复格式错误")?;
            let bytes = STANDARD.decode(encoded).map_err(|_| "功能回复编码错误")?;
            if bytes.len() > 700000 {
                return Err("功能回复超过限制".into());
            }
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| "功能回复 JSON 错误")?;
            if !value.is_object() {
                return Err("功能回复不是对象".into());
            }
            Ok(value)
        })
        .await
        .map_err(|_| "功能请求超时；请查询原操作结果，不要重复执行".to_string())?
    }
}
struct Active {
    task: u8,
    started: std::time::Instant,
    id: String,
    url: Option<String>,
    last: Value,
    fetched: std::collections::HashSet<String>,
}
async fn worker(
    host: String,
    endpoint: ControlEndpoint,
    commands: mpsc::Receiver<FunctionCommand>,
    events: mpsc::Sender<FunctionEvent>,
) {
    worker_with_provider(host, endpoint, commands, events, &mailbox::Router).await;
}
async fn worker_with_provider(
    host: String,
    endpoint: ControlEndpoint,
    mut commands: mpsc::Receiver<FunctionCommand>,
    events: mpsc::Sender<FunctionEvent>,
    mailbox_provider: &dyn mailbox::MailboxProvider,
) {
    // Each invoke/query/code submission owns one short authenticated socket.
    let _ = events.send(FunctionEvent::Available(true)).await;
    let mut active: Option<Active> = None;
    // Native email verification is a separate call; retain its opaque target in memory.
    let mut email_pending: Option<(Option<String>, String)> = None;
    let mut email_target: Option<String> = None;
    let mut email_mailbox: Option<mailbox::PreparedMailbox> = None;
    let mut automatic_command = None;
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let command = if let Some(command) = automatic_command.take() {
            Some(command)
        } else {
            tokio::select! {cmd=commands.recv()=>{let Some(cmd)=cmd else{return;};Some(cmd)},_=interval.tick(),if active.is_some()=>None}
        };
        if command.is_none()
            && active
                .as_ref()
                .is_some_and(|a| a.started.elapsed() > Duration::from_secs(600))
        {
            let _ = events
                .send(FunctionEvent::Uncertain(
                    "操作等待超时；保留原操作标识，可继续查询结果".into(),
                ))
                .await;
            active = None;
            continue;
        }
        let request = match command {
            Some(FunctionCommand::Scoped { .. } | FunctionCommand::RefreshAccountState) => continue,
            Some(FunctionCommand::RebindEmail { text }) => {
                if active.is_some() {
                    let _ = events
                        .send(FunctionEvent::Error("上一操作仍在执行，请等待完成".into()))
                        .await;
                    continue;
                }
                automatic_command = Some(FunctionCommand::Invoke {
                    id: "email-add".into(),
                    text,
                });
                continue;
            }
            Some(FunctionCommand::Invoke { id, text }) => {
                if active.is_some() {
                    let _ = events
                        .send(FunctionEvent::Error(
                            "上一操作尚未确认，请查询原操作结果".into(),
                        ))
                        .await;
                    continue;
                }
                let Some(spec) = OPERATIONS.iter().find(|s| s.id == id) else {
                    continue;
                };
                let mut parsed = match parse_operation(spec, &text) {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = events.send(FunctionEvent::Error(e)).await;
                        continue;
                    }
                };
                email_target = None;
                email_mailbox = None;
                if account::needs_current_account(spec) {
                    if spec.method == "verifyContactEmail" && email_pending.is_none() {
                        let _ = events
                            .send(FunctionEvent::Error(
                                "请先添加联系邮箱，再输入收到的验证码".into(),
                            ))
                            .await;
                        continue;
                    }
                    let target =
                        match account::query_current_account(&host, &endpoint, spec.method).await {
                            Ok(target) => target,
                            Err(error) => {
                                let _ = events.send(FunctionEvent::Error(error)).await;
                                continue;
                            }
                        };
                    if spec.method == "verifyContactEmail" {
                        let (expected, challenge) = email_pending.as_ref().unwrap();
                        if expected
                            .as_ref()
                            .is_some_and(|id| !id.eq_ignore_ascii_case(&target))
                        {
                            let _ = events
                                .send(FunctionEvent::Error(
                                    "手机账户已改变，不能提交原邮箱验证码".into(),
                                ))
                                .await;
                            continue;
                        }
                        parsed.params["challenge_id"] = json!(challenge);
                    }
                    if spec.method == "beginAddingContactEmail" {
                        email_target = Some(target.clone());
                    }
                    parsed.params["apple_id"] = json!(target);
                }
                if let Some(credentials) = parsed.mailbox.take() {
                    let _ = events
                        .send(FunctionEvent::Progress("正在检查邮箱授权…".into()))
                        .await;
                    match mailbox::PreparedMailbox::prepare(credentials, mailbox_provider).await {
                        Ok(mailbox) => email_mailbox = Some(mailbox),
                        Err(error) => {
                            let _ = events.send(FunctionEvent::Error(error)).await;
                            continue;
                        }
                    }
                    // OAuth preparation may take time. Resolve the phone's current account again.
                    match account::query_current_account(&host, &endpoint, spec.method).await {
                        Ok(target) if Some(&target) == email_target.as_ref() => (),
                        _ => {
                            email_mailbox = None;
                            let _ = events
                                .send(FunctionEvent::Error(
                                    "手机账户状态已改变，未添加邮箱".into(),
                                ))
                                .await;
                            continue;
                        }
                    }
                }
                if let Some(mailbox) = email_mailbox.as_mut()
                    && let Err(error) = mailbox.arm().await
                {
                    email_mailbox = None;
                    let _ = events.send(FunctionEvent::Error(error)).await;
                    continue;
                }
                // Drop input as soon as the native invocation has been serialized.
                Some((
                    spec.task,
                    json!({"action":"invoke","method":spec.method,"params":parsed.params}),
                    parsed.code_url,
                    true,
                ))
            }
            Some(FunctionCommand::Resume { task, operation_id }) => {
                if !matches!(task, 31 | 32) || operation_id.is_empty() || operation_id.len() > 128 {
                    continue;
                }
                if active
                    .as_ref()
                    .is_some_and(|a| a.task != task || a.id != operation_id)
                {
                    let _ = events
                        .send(FunctionEvent::Error("请先处理当前操作".into()))
                        .await;
                    continue;
                }
                if active.is_none() {
                    email_target = None;
                    interval.reset();
                    active = Some(Active {
                        started: std::time::Instant::now(),
                        task,
                        id: operation_id.clone(),
                        url: None,
                        last: Value::Null,
                        fetched: Default::default(),
                    });
                }
                Some((
                    task,
                    json!({"action":"result","operation_id":operation_id}),
                    None,
                    false,
                ))
            }
            Some(FunctionCommand::SubmitCode {
                operation_id,
                challenge_id,
                code,
            }) => {
                let Some(a) = active.as_ref() else {
                    let _ = events
                        .send(FunctionEvent::Error(
                            "请先查询原操作，再提交或取消验证".into(),
                        ))
                        .await;
                    continue;
                };
                if a.task != 31
                    || a.id != operation_id
                    || a.last["status"] != "requires_verification_code"
                    || a.last["challenge"]["challenge_id"] != challenge_id
                    || !valid_code(&code)
                {
                    let _ = events
                        .send(FunctionEvent::Error("验证码或验证任务无效".into()))
                        .await;
                    continue;
                }
                Some((
                    31,
                    json!({"action":"submit_code","operation_id":operation_id,"challenge_id":challenge_id,"code":code}),
                    None,
                    false,
                ))
            }
            Some(FunctionCommand::CancelChallenge {
                operation_id,
                challenge_id,
            }) => {
                let Some(a) = active.as_ref() else {
                    let _ = events
                        .send(FunctionEvent::Error(
                            "请先查询原操作，再提交或取消验证".into(),
                        ))
                        .await;
                    continue;
                };
                if a.task != 31
                    || a.id != operation_id
                    || a.last["challenge"]["challenge_id"] != challenge_id
                {
                    continue;
                }
                Some((
                    31,
                    json!({"action":"cancel_challenge","operation_id":operation_id,"challenge_id":challenge_id}),
                    None,
                    false,
                ))
            }
            Some(FunctionCommand::ReadEmailCode { text }) => {
                // Mailbox-only read: no native invoke, no verification replay, even after unknown.
                let spec = OPERATIONS.iter().find(|s| s.id == "email-add").unwrap();
                let credentials = match parse_operation(spec, &text).and_then(|mut p| {
                    p.mailbox
                        .take()
                        .ok_or_else(|| "原邮箱资料缺少 OAuth 授权，无法自动取码".into())
                }) {
                    Ok(credentials) => credentials,
                    Err(error) => {
                        let _ = events.send(FunctionEvent::Error(error)).await;
                        continue;
                    }
                };
                let _ = events
                    .send(FunctionEvent::Progress(
                        "正在重新读取最新邮件验证码…".into(),
                    ))
                    .await;
                let read = async {
                    let mut mailbox =
                        mailbox::PreparedMailbox::prepare(credentials, mailbox_provider).await?;
                    mailbox.read_latest().await
                };
                let result =
                    tokio::select! { result = read => result, _ = events.closed() => return };
                match result {
                    Ok(code) => {
                        let _ = events.send(FunctionEvent::EmailCodeFetched(code)).await;
                    }
                    Err(error) => {
                        let _ = events.send(FunctionEvent::Error(error)).await;
                    }
                }
                continue;
            }
            Some(FunctionCommand::FetchCode) => {
                if let Some(a) = &mut active {
                    if a.url.is_some() && a.last["status"] == "requires_verification_code" {
                        a.fetched.clear();
                    } else {
                        let _ = events
                            .send(FunctionEvent::Error(
                                "没有可读取的验证码链接，请手动输入验证码".into(),
                            ))
                            .await;
                    }
                } else {
                    let _ = events
                        .send(FunctionEvent::Error("请先查询原操作结果".into()))
                        .await;
                }
                None
            }
            None => {
                let a = active.as_ref().unwrap();
                Some((
                    a.task,
                    json!({"action":"result","operation_id":a.id}),
                    None,
                    false,
                ))
            }
        };
        if let Some((task, body, url, invocation)) = request {
            let mut wire = match Wire::connect(&host, &endpoint).await {
                Ok(connection) => connection,
                Err(e) => {
                    let _ = events.send(FunctionEvent::Error(e)).await;
                    continue;
                }
            };
            let action = body["action"].as_str().unwrap_or("").to_owned();
            let response = match wire.request(task, body).await {
                Ok(r) => r,
                Err(e) => {
                    let _ = events.send(FunctionEvent::Uncertain(e)).await;
                    let _ = events.send(FunctionEvent::Available(false)).await;
                    return;
                }
            };
            drop(wire);
            if invocation {
                let Some(id) = response["operation_id"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 128)
                else {
                    let message = if response.get("error").is_some() {
                        service_error(&response)
                    } else {
                        "原生操作标识缺失；结果未知，请勿重复执行".into()
                    };
                    let event = if response.get("error").is_none() {
                        FunctionEvent::Uncertain(message)
                    } else {
                        FunctionEvent::Error(message)
                    };
                    let _ = events.send(event).await;
                    continue;
                };
                interval.reset();
                active = Some(Active {
                    started: std::time::Instant::now(),
                    task,
                    id: id.into(),
                    url,
                    last: Value::Null,
                    fetched: Default::default(),
                });
            }

            if matches!(action.as_str(), "submit_code" | "cancel_challenge") {
                let a = active.as_mut().unwrap();
                match accepted_code(&response, &a.id) {
                    Ok(()) => {
                        a.last["status"] = json!("running");
                        a.last.as_object_mut().unwrap().remove("challenge");
                        let _ = events
                            .send(FunctionEvent::State {
                                task,
                                record: a.last.clone(),
                            })
                            .await;
                    }
                    Err(e) => {
                        let _ = events.send(FunctionEvent::Error(e)).await;
                    }
                }
                continue;
            }
            if response.get("operation_id").is_some() {
                let a = active.as_mut().unwrap();
                if response["operation_id"] != a.id {
                    let _ = events
                        .send(FunctionEvent::Error("原生操作回复标识不匹配".into()))
                        .await;
                    active = None;
                    continue;
                }
                if !matches!(
                    response["status"].as_str(),
                    Some("running" | "completed" | "requires_verification_code")
                ) {
                    let _ = events
                        .send(FunctionEvent::Uncertain(
                            "原生操作状态无效；请查询原操作结果".into(),
                        ))
                        .await;
                    active = None;
                    continue;
                }
                let automatic_email = response["status"] == "completed"
                    && response["method"] == "beginAddingContactEmail"
                    && response.get("error").is_none()
                    && response["result"]["outcome"] == "requires_email_verification"
                    && response["result"]["challenge_id"].is_string()
                    && email_mailbox.is_some();
                if response != a.last {
                    a.last = response.clone();
                    let event = if automatic_email {
                        FunctionEvent::EmailCodePending(response.clone())
                    } else {
                        FunctionEvent::State {
                            task,
                            record: response.clone(),
                        }
                    };
                    let _ = events.send(event).await;
                }
                if response["status"] == "completed" {
                    if response["method"] == "beginAddingContactEmail"
                        && response.get("error").is_none()
                        && response["result"]["outcome"] == "requires_email_verification"
                        && let Some(challenge) = response["result"]["challenge_id"].as_str()
                    {
                        // A resumed result has no client-side target. The native service
                        // still binds this challenge to its original Apple ID and validates
                        // the freshly resolved ID before accepting verification.
                        email_pending = Some((email_target.take(), challenge.into()));
                        if let Some(mut mailbox) = email_mailbox.take() {
                            let _ = events
                                .send(FunctionEvent::Progress(
                                    "等待邮件送达，随后读取验证码…".into(),
                                ))
                                .await;
                            let code = tokio::select! {
                                code = mailbox.wait_code() => code,
                                _ = events.closed() => return,
                            };
                            match code {
                                Ok(code) if !commands.is_closed() => {
                                    if events
                                        .send(FunctionEvent::VerificationCodeReceived(code.clone()))
                                        .await
                                        .is_err()
                                    {
                                        return;
                                    }
                                    // A separate short ZXTouch call, with a fresh account check.
                                    automatic_command = Some(FunctionCommand::Invoke {
                                        id: "email-verify".into(),
                                        text: code,
                                    });
                                }
                                Ok(_) => return,
                                Err(error) => {
                                    let _ = events.send(FunctionEvent::Error(error)).await;
                                }
                            }
                        }
                    }
                    if response["method"] == "verifyContactEmail"
                        && response.get("error").is_none()
                        && response["result"]["outcome"] == "succeeded"
                    {
                        email_pending = None;
                    }
                    email_mailbox = None;
                    active = None;
                    continue;
                }
            } else if response.get("error").is_some() {
                let _ = events
                    .send(FunctionEvent::Uncertain(service_error(&response)))
                    .await;
                // A missing result never means the prior mutation did not execute.
                active = None;
                continue;
            }
        }
        if let Some(a) = &mut active
            && a.task == 31
            && a.last["status"] == "requires_verification_code"
            && let Some(challenge) = a.last["challenge"]["challenge_id"]
                .as_str()
                .map(str::to_owned)
            && let Some(url) = a.url.as_ref()
            && a.fetched.insert(challenge.clone())
        {
            match fetch_code(url).await {
                Ok(code) => {
                    if events
                        .send(FunctionEvent::VerificationCodeReceived(code.clone()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    // Code retrieval does not invoke or replay login.
                    let mut wire = match Wire::connect(&host, &endpoint).await {
                        Ok(wire) => wire,
                        Err(e) => {
                            let _ = events.send(FunctionEvent::Error(e)).await;
                            continue;
                        }
                    };
                    match wire.request(31,json!({"action":"submit_code","operation_id":a.id,"challenge_id":challenge,"code":code})).await {
                        Ok(response)=>match accepted_code(&response,&a.id) {
                            Ok(())=>{a.last["status"]=json!("running");a.last.as_object_mut().unwrap().remove("challenge");let _=events.send(FunctionEvent::State{task:31,record:a.last.clone()}).await;},
                            Err(e)=>{let _=events.send(FunctionEvent::Error(e)).await;},
                        },
                        Err(_)=>{let _=events.send(FunctionEvent::Uncertain("验证码提交结果未知；请查询原登录结果".into())).await;let _=events.send(FunctionEvent::Available(false)).await;return;},
                    }
                }
                Err(e) => {
                    let _ = events.send(FunctionEvent::Error(e)).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod code_tests {
    use super::*;
    #[test]
    fn code_is_explicit_and_preserves_zeroes() {
        assert_eq!(
            extract_verification_code("{\"data\":{\"code\":\"012345\"}}").unwrap(),
            "012345"
        );
        assert_eq!(extract_verification_code("012345").unwrap(), "012345");
        assert!(extract_verification_code("{\"id\":123456}").is_err());
        assert!(extract_verification_code("{\"code\":\"123456\",\"otp\":\"654321\"}").is_err());
        assert!(validate_code_url("file:///etc/passwd").is_err());
        assert!(validate_code_url("https://u:p@example.test/").is_err());
    }
}
