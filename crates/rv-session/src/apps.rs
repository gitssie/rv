use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::zxtouch::{ControlChannel, ControlEndpoint, ControlEvent};
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};
use vnc::device::{DeviceReply, DeviceRequest};
use vnc::{VncClient, X11Event};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AppCapabilities {
    pub list: bool,
    pub launch: bool,
    pub terminate: bool,
    pub restart: bool,
    pub foreground: bool,
    pub icons: bool,
    pub control: bool,
    pub unlock: bool,
    pub http_screen_control: bool,
    pub lock: bool,
    pub home: bool,
    pub notifications: bool,
    pub clipboard_write: bool,
    pub photo_tasks: bool,
    pub device_info: bool,
    pub app_profile: bool,
    pub zx_control: Option<ControlEndpoint>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemoteApp {
    pub bundle_id: String,
    pub name: String,
    pub can_launch: bool,
    pub can_terminate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppCommand {
    List,
    Karing(crate::KaringRequest),
    CancelKaring,
    Launch(String),
    Terminate(String),
    Restart(String),
    Foreground,
    DeviceInfo,
    ReadProfile {
        app_id: rv_core::workspace::AppId,
        bundle: String,
        backup: String,
    },
    Icon(String),
    ScreenState(u32),
    PrepareUnlock(u32),
    ArmUnlock(u32, u8),
    LockScreen(u32),
    HttpUnlock(u32, crate::ScreenPasscode),
    HttpLock(u32),
    CancelScreen(u32),
    Home,
    SubscribeNotifications(bool),
    Notifications,
    WriteClipboard {
        token: String,
        text: String,
    },
}

impl AppCommand {
    pub fn bundle_id(&self) -> Option<&str> {
        match self {
            Self::Launch(id) | Self::Terminate(id) | Self::Restart(id) | Self::Icon(id) => Some(id),
            _ => None,
        }
    }
    pub fn unlock_attempt(&self) -> Option<u32> {
        match self {
            Self::ScreenState(id)
            | Self::PrepareUnlock(id)
            | Self::ArmUnlock(id, _)
            | Self::LockScreen(id)
            | Self::HttpUnlock(id, _)
            | Self::HttpLock(id)
            | Self::CancelScreen(id) => Some(*id),
            _ => None,
        }
    }
    fn op(&self) -> u8 {
        match self {
            Self::HttpUnlock(_, _) | Self::HttpLock(_) | Self::CancelScreen(_) => 0,
            Self::Karing(_) | Self::CancelKaring => 0,
            Self::List => 1,
            Self::Launch(_) => 2,
            Self::Terminate(_) => 3,
            Self::Restart(_) => 4,
            Self::Foreground => 5,
            Self::DeviceInfo => 25,
            Self::ReadProfile { .. } => 0,
            Self::Icon(_) => 6,
            Self::ScreenState(_) => 7,
            Self::PrepareUnlock(_) => 8,
            Self::ArmUnlock(_, _) => 9,
            Self::LockScreen(_) => 10,
            Self::Home => 12,
            Self::SubscribeNotifications(_) => 13,
            Self::Notifications => 14,
            Self::WriteClipboard { .. } => 16,
        }
    }
    fn mutates(&self) -> bool {
        if let Self::Karing(request) = self {
            return request.method != "GET" && request.method != "HEAD";
        }
        matches!(
            self,
            Self::Launch(_)
                | Self::Terminate(_)
                | Self::Restart(_)
                | Self::PrepareUnlock(_)
                | Self::ArmUnlock(_, _)
                | Self::LockScreen(_)
                | Self::HttpUnlock(_, _)
                | Self::HttpLock(_)
                | Self::Home
                | Self::WriteClipboard { .. }
        )
    }
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    Capabilities(AppCapabilities),
    Karing(crate::KaringResponse),
    LockState(bool),
    Notifications(crate::NotificationSnapshot),
    ScreenState(u32, ScreenState),
    UnlockArmed(u32, bool),
    ScreenProgress(u32, crate::ScreenProgress),
    List(Vec<RemoteApp>),
    Foreground(Option<String>),
    DeviceInfo(Value),
    Profile(crate::AppProfileResponse),
    Icon {
        bundle_id: String,
        width: u32,
        height: u32,
        bgra: Vec<u8>,
    },
    Finished(AppCommand),
    Failed {
        command: AppCommand,
        message: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScreenState {
    pub locked: bool,
    pub passcode_required: bool,
    pub input_ready: bool,
    pub input_empty: bool,
    #[serde(default)]
    pub reason: String,
}

pub fn valid_bundle_id(id: &str) -> bool {
    id.len() <= 255
        && id.split('.').count() >= 2
        && id.split('.').all(|part| {
            !part.is_empty() && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
}

pub(crate) struct AppRuntime {
    caps: AppCapabilities,
    allowed: bool,
    sequence: u32,
    queue: VecDeque<AppCommand>,
    pending: HashMap<u32, (AppCommand, Instant)>,
    control_host: Option<(String, Option<u16>)>,
    control: Option<ControlChannel>,
    control_ready: bool,
    http_client: Option<crate::phone_http::PhoneHttpClient>,
    control_connecting: bool,
    screen_operation: Option<crate::screen_control::ScreenOperation>,
    karing_operations: Vec<crate::karing::KaringOperation>,
    profile_operations: Vec<crate::app_profile::ProfileOperation>,
}

impl AppRuntime {
    pub fn new(allowed: bool) -> Self {
        Self {
            caps: AppCapabilities::default(),
            allowed,
            sequence: 0,
            queue: VecDeque::new(),
            pending: HashMap::new(),
            control_host: None,
            control: None,
            control_ready: false,
            http_client: None,
            control_connecting: false,
            screen_operation: None,
            karing_operations: Vec::new(),
            profile_operations: Vec::new(),
        }
    }
    fn supported(&self, cmd: &AppCommand) -> bool {
        (!cmd.mutates() || (self.allowed && self.caps.control))
            && match cmd {
                AppCommand::HttpUnlock(_, _) => self.caps.http_screen_control && self.caps.unlock,
                AppCommand::HttpLock(_) => self.caps.http_screen_control && self.caps.lock,
                AppCommand::CancelKaring => true,
                AppCommand::Karing(_) => self.caps.control && self.caps.zx_control.is_some(),
                AppCommand::CancelScreen(_) => true,
                AppCommand::Home => self.caps.home,
                AppCommand::WriteClipboard { .. } => self.caps.clipboard_write,
                AppCommand::SubscribeNotifications(_) | AppCommand::Notifications => {
                    self.caps.notifications
                }
                AppCommand::List => self.caps.list,
                AppCommand::Launch(_) => self.caps.launch,
                AppCommand::Terminate(_) => self.caps.terminate,
                AppCommand::Restart(_) => self.caps.restart,
                AppCommand::Foreground => self.caps.foreground,
                AppCommand::DeviceInfo => self.caps.device_info && self.caps.zx_control.is_some(),
                AppCommand::ReadProfile { .. } => {
                    self.caps.app_profile && self.caps.zx_control.is_some() && self.allowed
                }
                AppCommand::Icon(_) => self.caps.icons,
                AppCommand::ScreenState(_) => self.caps.unlock || self.caps.lock,
                AppCommand::LockScreen(_) => self.caps.lock,
                AppCommand::PrepareUnlock(_) | AppCommand::ArmUnlock(_, _) => self.caps.unlock,
            }
    }
    pub fn command(&mut self, command: AppCommand, send: &impl Fn(AppEvent)) {
        if let AppCommand::ReadProfile {
            app_id,
            bundle,
            backup,
        } = &command
        {
            if !self.supported(&command)
                || !self.control_ready
                || !self.profile_operations.is_empty()
                || !valid_bundle_id(bundle)
                || !backup.starts_with("snapshot:")
            {
                send(AppEvent::Failed {
                    command,
                    message: "手机账号资料读取不可用或正在执行".into(),
                });
                return;
            }
            if let Some(client) = self.http_client.clone() {
                self.profile_operations
                    .push(crate::app_profile::ProfileOperation::start(
                        client,
                        app_id.clone(),
                        bundle.clone(),
                        backup.clone(),
                    ));
            }
            return;
        }
        if matches!(command, AppCommand::CancelKaring) {
            self.karing_operations.clear();
            return;
        }
        if let AppCommand::Karing(request) = &command {
            if !self.supported(&command)
                || !self.control_ready
                || self.karing_operations.len() >= 16
            {
                send(AppEvent::Failed {
                    command,
                    message: "Karing 控制不可用或请求正在执行".into(),
                });
                return;
            }
            let operation = self
                .control_host
                .as_ref()
                .zip(self.caps.zx_control.as_ref())
                .ok_or_else(|| "Karing 控制地址不可用".to_string())
                .and_then(|_| {
                    crate::karing::KaringOperation::start(
                        self.http_client.clone().ok_or("HTTP/2 控制正在恢复")?,
                        request.clone(),
                    )
                });
            match operation {
                Ok(operation) => self.karing_operations.push(operation),
                Err(message) => send(AppEvent::Failed { command, message }),
            }
            return;
        }
        if let AppCommand::CancelScreen(attempt) = command {
            if self
                .screen_operation
                .as_ref()
                .is_some_and(|op| op.attempt == attempt)
            {
                self.screen_operation = None;
            }
            return;
        }
        if matches!(
            command,
            AppCommand::HttpUnlock(_, _) | AppCommand::HttpLock(_)
        ) {
            if !self.supported(&command) || !self.control_ready || self.screen_operation.is_some() {
                send(AppEvent::Failed {
                    command,
                    message: "HTTP/2 屏幕控制不可用或正在执行".into(),
                });
                return;
            }
            let Some(http_client) = self.http_client.clone() else {
                return;
            };
            let failure_command = match &command {
                AppCommand::HttpUnlock(id, _) => {
                    AppCommand::HttpUnlock(*id, crate::ScreenPasscode(String::new()))
                }
                _ => command.clone(),
            };
            match crate::screen_control::ScreenOperation::start(http_client, command) {
                Ok(op) => self.screen_operation = Some(op),
                Err(message) => send(AppEvent::Failed {
                    command: failure_command,
                    message,
                }),
            }
            return;
        }
        if let AppCommand::WriteClipboard { token, text } = &command
            && (token.len() != 36
                || uuid::Uuid::parse_str(token).is_err()
                || json!({"token":token,"text":text}).to_string().len() > 4096)
        {
            send(AppEvent::Failed {
                command,
                message: "剪贴板文本过长（请求最多 4 KiB）".into(),
            });
            return;
        }
        if !self.supported(&command) || command.bundle_id().is_some_and(|id| !valid_bundle_id(id)) {
            send(AppEvent::Failed {
                command,
                message: "This App operation is unavailable on this connection".into(),
            });
            return;
        }
        if matches!(command, AppCommand::SubscribeNotifications(_)) {
            self.queue
                .retain(|queued| !matches!(queued, AppCommand::SubscribeNotifications(_)));
        }
        if self.queue.contains(&command) || self.pending.values().any(|(cmd, _)| cmd == &command) {
            return;
        }
        if self.queue.len() >= 128 {
            send(AppEvent::Failed {
                command,
                message: "Too many queued App operations".into(),
            });
            return;
        }
        if command.unlock_attempt().is_some() {
            let index = self
                .queue
                .iter()
                .position(|queued| queued.unlock_attempt().is_none())
                .unwrap_or(self.queue.len());
            self.queue.insert(index, command);
        } else if command.mutates() {
            let index = self
                .queue
                .iter()
                .position(|queued| !queued.mutates())
                .unwrap_or(self.queue.len());
            self.queue.insert(index, command);
        } else {
            self.queue.push_back(command);
        }
    }
    pub async fn advance(&mut self, client: &VncClient, send: &impl Fn(AppEvent)) {
        self.advance_transport(Some(client), send).await;
    }
    pub(crate) async fn advance_http(&mut self, send: &impl Fn(AppEvent)) {
        self.advance_transport(None, send).await;
    }
    pub(crate) fn configure_http(
        &mut self,
        caps: AppCapabilities,
        client: crate::phone_http::PhoneHttpClient,
        send: &impl Fn(AppEvent),
    ) {
        self.caps = caps;
        self.http_client = Some(client.clone());
        self.control = Some(ControlChannel::start_with_client(client));
        self.control_connecting = true;
        send(AppEvent::Capabilities(self.caps.clone()));
        for command in [
            AppCommand::DeviceInfo,
            AppCommand::List,
            AppCommand::Foreground,
        ] {
            self.command(command, send);
        }
    }
    pub(crate) fn http_ready(&self) -> bool {
        self.control_ready
    }
    async fn advance_transport(&mut self, client: Option<&VncClient>, send: &impl Fn(AppEvent)) {
        self.profile_operations
            .retain_mut(|operation| match operation.result.try_recv() {
                Ok(Ok(response)) => {
                    send(AppEvent::Profile(response));
                    false
                }
                Ok(Err(message)) => {
                    send(AppEvent::Failed {
                        command: operation.command.clone(),
                        message,
                    });
                    false
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => true,
                Err(_) => {
                    send(AppEvent::Failed {
                        command: operation.command.clone(),
                        message: "账号资料读取已停止".into(),
                    });
                    false
                }
            });
        self.karing_operations
            .retain_mut(|operation| match operation.result.try_recv() {
                Ok(Ok(response)) => {
                    send(AppEvent::Karing(response));
                    false
                }
                Ok(Err(message)) => {
                    send(AppEvent::Failed {
                        command: operation.command.clone(),
                        message,
                    });
                    false
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => true,
                Err(_) => {
                    send(AppEvent::Failed {
                        command: operation.command.clone(),
                        message: "Karing 请求已停止".into(),
                    });
                    false
                }
            });
        if let Some(operation) = &mut self.screen_operation {
            let mut finished = false;
            while let Ok(event) = operation.events.try_recv() {
                finished |= matches!(&event, AppEvent::Failed { .. })
                    || matches!(&event,
                    AppEvent::ScreenProgress(_, progress) if matches!(progress.phase.as_str(), "locked" | "unlocked" | "failed"));
                send(event);
            }
            if finished {
                self.screen_operation = None;
            }
        }
        let mut events = Vec::new();
        if let Some(control) = &mut self.control {
            while let Ok(event) = control.events.try_recv() {
                events.push(event);
            }
        }
        for event in events {
            match event {
                ControlEvent::Ready => {
                    self.control_ready = true;
                    self.control_connecting = false;
                }
                ControlEvent::Unavailable => {
                    // A transport failure does not revoke negotiated capabilities.
                    // The channel restores itself; other HTTP streams own their outcome.
                    self.control_unavailable(send);
                }
                ControlEvent::Reply(reply) => self.reply(reply, send),
            }
        }
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, (command, start))| {
                start.elapsed()
                    > Duration::from_secs(
                        if matches!(
                            command,
                            AppCommand::ScreenState(_) | AppCommand::PrepareUnlock(_)
                        ) {
                            2
                        } else {
                            12
                        },
                    )
            })
            .map(|(&id, _)| id)
            .collect();
        for id in expired {
            let (command, _) = self.pending.remove(&id).unwrap();
            send(AppEvent::Failed {
                command,
                message: "App operation timed out; its result is unknown".into(),
            });
        }
        let unlock_queued = self
            .queue
            .front()
            .is_some_and(|cmd| cmd.unlock_attempt().is_some());
        if self.pending.len() >= if unlock_queued { 4 } else { 3 } {
            return;
        }
        let Some(command) = self.queue.pop_front() else {
            return;
        };
        let independent = matches!(command.op(), 1..=6 | 12 | 16 | 25);
        if independent
            && (self.control_connecting
                || (self.control_ready
                    && self
                        .pending
                        .values()
                        .any(|(cmd, _)| matches!(cmd.op(), 1..=6 | 12 | 16 | 25))))
        {
            self.queue.push_front(command);
            return;
        }
        if matches!(command, AppCommand::SubscribeNotifications(_))
            && self
                .pending
                .values()
                .any(|(cmd, _)| matches!(cmd, AppCommand::SubscribeNotifications(_)))
        {
            self.queue.push_front(command);
            return;
        }
        // A single device has one foreground app. Serialize user mutations across requests.
        if command.mutates() && self.pending.values().any(|(cmd, _)| cmd.mutates()) {
            self.queue.push_front(command);
            return;
        }
        self.sequence = self.sequence.wrapping_add(1).max(1);
        let payload = match &command {
            AppCommand::WriteClipboard { token, text } => json!({"token":token,"text":text}),
            AppCommand::SubscribeNotifications(enabled) => json!({"enabled":enabled}),
            AppCommand::ArmUnlock(_, digits) => json!({"digits":digits}),
            _ => command
                .bundle_id()
                .map_or_else(|| json!({}), |id| json!({"bundle_id":id})),
        }
        .to_string();
        let id = self.sequence;
        let request = DeviceRequest {
            op: command.op(),
            id,
            payload,
        };
        let sent = if independent {
            if let Some(control) = self.control.as_ref().filter(|_| self.control_ready) {
                control.requests.try_send(request).map_err(|_| {
                    "ZXTouch connection closed or busy; reconnect the session".to_string()
                })
            } else {
                Err("ZXTouch control is unavailable; check the connection settings and phone service".into())
            }
        } else if let Some(client) = client {
            client
                .input(X11Event::Device(request))
                .await
                .map_err(|error| error.to_string())
        } else {
            Err("此操作需要远程画面连接".into())
        };
        match sent {
            Ok(()) => {
                self.pending.insert(id, (command, Instant::now()));
            }
            Err(error) => send(AppEvent::Failed {
                command,
                message: error,
            }),
        }
    }
    pub fn reply(&mut self, reply: DeviceReply, send: &impl Fn(AppEvent)) {
        if reply.op == 15 {
            if self.caps.notifications
                && reply.id == 0
                && reply.status == 0
                && let Ok(value) = serde_json::from_str::<Value>(&reply.payload)
                && let Ok(snapshot) = crate::NotificationSnapshot::parse(value)
            {
                send(AppEvent::Notifications(snapshot));
            }
            return;
        }
        if reply.op == 11 {
            if reply.id == 0
                && reply.status == 0
                && let Ok(value) = serde_json::from_str::<Value>(&reply.payload)
                && let Some(locked) = value["locked"].as_bool()
            {
                send(AppEvent::LockState(locked));
            }
            return;
        }
        if reply.op == 0 {
            if reply.status != 0 {
                return;
            }
            if let Ok(caps) = serde_json::from_str::<AppCapabilities>(&reply.payload) {
                if self.control.is_none()
                    && let Some((host, port)) = &self.control_host
                    && let Some(mut endpoint) = caps.zx_control.clone()
                    && matches!(endpoint.version, 2 | 3)
                    && endpoint.port > 0
                    && endpoint.token.len() == 64
                    && endpoint.token.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    if let Some(port) = port {
                        endpoint.port = *port;
                    }
                    let http_client = crate::phone_http::PhoneHttpClient::new(
                        host,
                        endpoint.port,
                        endpoint.token.clone(),
                    )
                    .ok();
                    self.control = http_client
                        .as_ref()
                        .map(|client| ControlChannel::start_with_client(client.clone()));
                    self.http_client = http_client;
                    self.control_connecting = self.control.is_some();
                }
                self.caps = caps.clone();
                if self.control.is_none() {
                    self.disable_control_capabilities();
                }
                send(AppEvent::Capabilities(self.caps.clone()));
                if self.caps.list {
                    self.command(AppCommand::List, send);
                }
                if self.caps.foreground {
                    self.command(AppCommand::Foreground, send);
                }
                if self.caps.device_info && self.caps.zx_control.is_some() {
                    self.command(AppCommand::DeviceInfo, send);
                }
            }
            return;
        }
        let Some((command, _)) = self.pending.remove(&reply.id) else {
            return;
        };
        let parsed = serde_json::from_str::<Value>(&reply.payload);
        if reply.op != command.op() || parsed.is_err() {
            send(AppEvent::Failed {
                command,
                message: "Invalid App operation response".into(),
            });
            return;
        }
        let value = parsed.unwrap();
        if reply.status != 0 {
            send(AppEvent::Failed {
                command,
                message: value["error"]
                    .as_str()
                    .unwrap_or("App operation failed")
                    .to_owned(),
            });
            return;
        }
        let result: Result<(), String> = (|| {
            match &command {
                AppCommand::WriteClipboard { token, .. } => {
                    if value["token"].as_str() != Some(token.as_str())
                        || value["ok"].as_bool() != Some(true)
                    {
                        return Err("Invalid clipboard acknowledgement".into());
                    }
                    send(AppEvent::Finished(command.clone()));
                }
                AppCommand::Notifications => {
                    send(AppEvent::Notifications(crate::NotificationSnapshot::parse(
                        value.clone(),
                    )?));
                }
                AppCommand::Home | AppCommand::SubscribeNotifications(_) => {
                    if value["ok"].as_bool() != Some(true) {
                        return Err("Device request was not accepted".into());
                    }
                    send(AppEvent::Finished(command.clone()));
                }
                AppCommand::ScreenState(_)
                | AppCommand::PrepareUnlock(_)
                | AppCommand::LockScreen(_) => {
                    let state: ScreenState = serde_json::from_value(value.clone())
                        .map_err(|_| "Invalid screen state")?;
                    if state.reason.len() > 1024 {
                        return Err("Invalid screen state".into());
                    }
                    send(AppEvent::ScreenState(
                        command.unlock_attempt().unwrap(),
                        state,
                    ));
                }
                AppCommand::ArmUnlock(_, _) => {
                    let armed = value["armed"]
                        .as_bool()
                        .ok_or("Missing unlock input state")?;
                    send(AppEvent::UnlockArmed(
                        command.unlock_attempt().unwrap(),
                        armed,
                    ));
                }
                AppCommand::List => {
                    let apps = serde_json::from_value::<Vec<RemoteApp>>(value["apps"].clone())
                        .map_err(|e| e.to_string())?;
                    if apps.len() > 4096
                        || apps
                            .iter()
                            .any(|app| !valid_bundle_id(&app.bundle_id) || app.name.len() > 1024)
                    {
                        return Err("Invalid installed App list".into());
                    }
                    send(AppEvent::List(apps));
                }
                AppCommand::Foreground => {
                    let id = value.get("bundle_id").ok_or("Missing foreground state")?;
                    let id = if id.is_null() {
                        None
                    } else {
                        let id = id
                            .as_str()
                            .filter(|id| valid_bundle_id(id))
                            .ok_or("Invalid foreground App")?;
                        Some(id.to_owned())
                    };
                    send(AppEvent::Foreground(id));
                }
                AppCommand::DeviceInfo => {
                    if !value.is_object() {
                        return Err("Invalid device information".into());
                    }
                    send(AppEvent::DeviceInfo(value.clone()));
                }
                AppCommand::Icon(bundle_id) => {
                    if let Some(encoded) = value["png"].as_str() {
                        if encoded.len() > 350000 {
                            return Err("App icon is too large".into());
                        }
                        let png = base64::engine::general_purpose::STANDARD
                            .decode(encoded)
                            .map_err(|e| e.to_string())?;
                        let mut reader = image::ImageReader::with_format(
                            std::io::Cursor::new(png),
                            image::ImageFormat::Png,
                        );
                        let mut limits = image::Limits::default();
                        limits.max_image_width = Some(128);
                        limits.max_image_height = Some(128);
                        limits.max_alloc = Some(1024 * 1024);
                        reader.limits(limits);
                        let image = reader.decode().map_err(|e| e.to_string())?.into_rgba8();
                        let (width, height) = image.dimensions();
                        let mut bgra = image.into_raw();
                        crate::compositor::swap_red_blue(&mut bgra);
                        send(AppEvent::Icon {
                            bundle_id: bundle_id.clone(),
                            width,
                            height,
                            bgra,
                        });
                    } else if !value.get("png").is_some_and(Value::is_null) {
                        return Err("Invalid App icon response".into());
                    }
                    send(AppEvent::Finished(command.clone()));
                }
                _ => {
                    if value["bundle_id"].as_str() != command.bundle_id() {
                        return Err("App response target changed".into());
                    }
                    send(AppEvent::Finished(command.clone()));
                    if self.caps.foreground {
                        self.command(AppCommand::Foreground, send);
                    }
                }
            }
            Ok(())
        })();
        if let Err(message) = result {
            send(AppEvent::Failed { command, message });
        }
    }

    pub fn set_control_host(&mut self, host: String, port: Option<u16>) {
        self.control_host = Some((host, port));
    }

    pub(crate) fn http_client(&self) -> Option<crate::phone_http::PhoneHttpClient> {
        self.http_client.clone()
    }
    fn control_unavailable(&mut self, send: &impl Fn(AppEvent)) {
        let queued = std::mem::take(&mut self.queue);
        for command in queued {
            if command.mutates() {
                send(AppEvent::Failed {
                    command,
                    message: "ZXTouch 连接中断；排队操作未发送，请恢复后重新操作".into(),
                });
            } else {
                self.queue.push_back(command);
            }
        }
        self.control_ready = false;
        self.control_connecting = true;
    }

    fn disable_control_capabilities(&mut self) {
        // Invalidate HTTP discovery without disabling retained VNC screen control.
        self.caps.zx_control = None;
        self.caps.http_screen_control = false;
        self.caps.list = false;
        self.caps.launch = false;
        self.caps.terminate = false;
        self.caps.restart = false;
        self.caps.foreground = false;
        self.caps.icons = false;
        self.caps.home = false;
        self.caps.clipboard_write = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    #[test]
    fn retryable_control_failure_preserves_negotiated_capabilities() {
        let mut runtime = AppRuntime::new(true);
        runtime.caps.list = true;
        runtime.caps.launch = true;
        runtime.caps.control = true;
        runtime.caps.http_screen_control = true;
        runtime.caps.zx_control = Some(ControlEndpoint {
            version: 2,
            port: 6000,
            token: "a".repeat(64),
        });
        runtime.control_ready = true;
        runtime.control_unavailable(&|_| {});
        assert!(!runtime.control_ready && runtime.control_connecting);
        assert!(runtime.caps.list && runtime.caps.launch && runtime.caps.http_screen_control);
        assert!(runtime.caps.zx_control.is_some());
    }
    #[test]
    fn lost_control_discards_queued_mutations_and_retains_reads() {
        let mut runtime = AppRuntime::new(true);
        runtime.queue.extend([
            AppCommand::Launch("fr.vinted".into()),
            AppCommand::Restart("fr.vinted".into()),
            AppCommand::DeviceInfo,
        ]);
        let failed = RefCell::new(vec![]);
        runtime.control_unavailable(&|event| {
            if let AppEvent::Failed { command, .. } = event {
                failed.borrow_mut().push(command);
            }
        });
        assert_eq!(failed.borrow().len(), 2);
        assert_eq!(
            runtime.queue.into_iter().collect::<Vec<_>>(),
            vec![AppCommand::DeviceInfo]
        );
    }
    #[test]
    fn http_control_loss_disables_karing_but_preserves_legacy_vnc_screen_control() {
        let mut runtime = AppRuntime::new(true);
        runtime.caps.control = true;
        runtime.caps.unlock = true;
        runtime.caps.lock = true;
        runtime.caps.http_screen_control = true;
        runtime.caps.zx_control = Some(ControlEndpoint {
            version: 2,
            port: 6000,
            token: "fixture-token".into(),
        });
        let karing = AppCommand::Karing(crate::KaringRequest {
            id: 1,
            method: "GET".into(),
            path: "/proxies".into(),
            body: vec![],
        });
        assert!(runtime.supported(&karing));
        runtime.disable_control_capabilities();
        assert!(!runtime.supported(&karing));
        assert!(!runtime.supported(&AppCommand::HttpLock(1)));
        assert!(runtime.supported(&AppCommand::ArmUnlock(1, 6)));
        assert!(runtime.supported(&AppCommand::LockScreen(1)));
    }
    #[test]
    fn notification_subscription_keeps_latest_intent_and_allows_read_only() {
        let mut runtime = AppRuntime::new(false);
        runtime.caps.notifications = true;
        runtime.pending.insert(
            1,
            (AppCommand::SubscribeNotifications(true), Instant::now()),
        );
        runtime.command(AppCommand::SubscribeNotifications(false), &|_| {});
        assert_eq!(
            runtime.queue.front(),
            Some(&AppCommand::SubscribeNotifications(false))
        );
        runtime.command(AppCommand::SubscribeNotifications(true), &|_| {});
        assert!(runtime.queue.is_empty());
        runtime.command(AppCommand::Notifications, &|_| {});
        assert_eq!(runtime.queue.front(), Some(&AppCommand::Notifications));
        runtime.caps.home = true;
        runtime.caps.control = true;
        assert!(!runtime.supported(&AppCommand::Home));
        assert_eq!(AppCommand::Home.op(), 12);
        assert_eq!(AppCommand::SubscribeNotifications(true).op(), 13);
        assert_eq!(AppCommand::Notifications.op(), 14);
    }

    #[test]
    fn notification_push_requires_capability_and_preserves_pending_requests() {
        let events = RefCell::new(vec![]);
        let send = |event| events.borrow_mut().push(event);
        let mut runtime = AppRuntime::new(true);
        runtime
            .pending
            .insert(9, (AppCommand::List, Instant::now()));
        let reply = || DeviceReply {
            op: 15,
            id: 0,
            status: 0,
            payload: json!({"revision":"epoch:1", "notifications":[]}).to_string(),
        };
        runtime.reply(reply(), &send);
        assert!(events.borrow().is_empty());
        runtime.caps.notifications = true;
        runtime.reply(reply(), &send);
        assert!(
            matches!(&events.borrow()[0], AppEvent::Notifications(snapshot) if snapshot.revision == "epoch:1")
        );
        assert!(runtime.pending.contains_key(&9));
        let mut malformed = reply();
        malformed.payload = "{}".into();
        runtime.reply(malformed, &send);
        assert_eq!(events.borrow().len(), 1);
    }

    #[test]
    fn unsolicited_lock_push_does_not_consume_pending_requests_or_start_input() {
        let events = RefCell::new(vec![]);
        let send = |event| events.borrow_mut().push(event);
        let mut runtime = AppRuntime::new(true);
        runtime
            .pending
            .insert(5, (AppCommand::PrepareUnlock(2), Instant::now()));
        for locked in [true, false] {
            runtime.reply(
                DeviceReply {
                    op: 11,
                    id: 0,
                    status: 0,
                    payload: json!({"locked":locked}).to_string(),
                },
                &send,
            );
        }
        assert!(matches!(events.borrow()[0], AppEvent::LockState(true)));
        assert!(matches!(events.borrow()[1], AppEvent::LockState(false)));
        assert!(runtime.pending.contains_key(&5));
        assert!(runtime.queue.is_empty());
        for (id, status, payload) in [
            (5, 0, r#"{"locked":true}"#),
            (0, 1, r#"{"locked":true}"#),
            (0, 0, r#"{"locked":"false"}"#),
            (0, 0, "{}"),
            (0, 0, "bad-json"),
        ] {
            runtime.reply(
                DeviceReply {
                    op: 11,
                    id,
                    status,
                    payload: payload.into(),
                },
                &send,
            );
        }
        assert_eq!(events.borrow().len(), 2);
        assert!(runtime.pending.contains_key(&5));
    }

    #[test]
    fn locking_requires_capability_and_control_and_returns_observed_state() {
        let events = RefCell::new(vec![]);
        let send = |event| events.borrow_mut().push(event);
        let mut runtime = AppRuntime::new(true);
        runtime.caps.control = true;
        runtime.command(AppCommand::LockScreen(3), &send);
        assert!(runtime.queue.is_empty());
        runtime.caps.lock = true;
        runtime.command(AppCommand::LockScreen(3), &send);
        runtime.command(AppCommand::LockScreen(3), &send);
        assert_eq!(runtime.queue.len(), 1);
        assert_eq!(AppCommand::LockScreen(3).op(), 10);
        runtime
            .pending
            .insert(4, (AppCommand::LockScreen(3), Instant::now()));
        runtime.reply(DeviceReply {
            op: 10, id: 4, status: 0,
            payload: r#"{"locked":true,"passcode_required":true,"input_ready":false,"input_empty":true}"#.into(),
        }, &send);
        assert!(matches!(&events.borrow()[1], AppEvent::ScreenState(3, state) if state.locked));
        let mut read_only = AppRuntime::new(false);
        read_only.caps = runtime.caps;
        read_only.command(AppCommand::LockScreen(3), &send);
        assert!(read_only.queue.is_empty());
    }

    #[test]
    fn unlock_polls_and_cancel_precede_icon_reads_and_keep_attempt_order() {
        let mut runtime = AppRuntime::new(true);
        runtime.caps = AppCapabilities {
            unlock: true,
            control: true,
            icons: true,
            ..Default::default()
        };
        runtime.command(AppCommand::Icon("com.apple.mobilesafari".into()), &|_| {});
        runtime.command(AppCommand::ScreenState(1), &|_| {});
        runtime.command(AppCommand::ArmUnlock(1, 0), &|_| {});
        runtime.command(AppCommand::PrepareUnlock(2), &|_| {});
        assert_eq!(
            runtime.queue.into_iter().collect::<Vec<_>>(),
            vec![
                AppCommand::ScreenState(1),
                AppCommand::ArmUnlock(1, 0),
                AppCommand::PrepareUnlock(2),
                AppCommand::Icon("com.apple.mobilesafari".into())
            ]
        );
    }
    #[test]
    fn unlock_replies_keep_attempt_identity_and_require_complete_state() {
        let events = RefCell::new(vec![]);
        let send = |event| events.borrow_mut().push(event);
        let mut runtime = AppRuntime::new(true);
        runtime
            .pending
            .insert(10, (AppCommand::ArmUnlock(2, 6), Instant::now()));
        runtime.reply(
            DeviceReply {
                op: 9,
                id: 10,
                status: 0,
                payload: r#"{"armed":true}"#.into(),
            },
            &send,
        );
        assert!(matches!(events.borrow()[0], AppEvent::UnlockArmed(2, true)));
        runtime
            .pending
            .insert(11, (AppCommand::ScreenState(2), Instant::now()));
        runtime.reply(
            DeviceReply {
                op: 7,
                id: 11,
                status: 0,
                payload: r#"{"locked":true}"#.into(),
            },
            &send,
        );
        assert!(matches!(events.borrow()[1], AppEvent::Failed { .. }));
        let mut read_only = AppRuntime::new(false);
        read_only.caps = AppCapabilities {
            unlock: true,
            control: true,
            ..Default::default()
        };
        read_only.command(AppCommand::PrepareUnlock(1), &send);
        read_only.command(AppCommand::ArmUnlock(1, 6), &send);
        assert!(read_only.queue.is_empty());
    }
    #[test]
    fn capability_and_read_only_checks_reject_mutations() {
        let events = RefCell::new(vec![]);
        let send = |event| events.borrow_mut().push(event);
        let mut runtime = AppRuntime::new(false);
        runtime.caps = AppCapabilities {
            control: true,
            launch: true,
            ..Default::default()
        };
        runtime.command(AppCommand::Launch("com.example.app".into()), &send);
        assert!(runtime.queue.is_empty());
        assert!(matches!(&events.borrow()[0], AppEvent::Failed { .. }));
    }
    #[test]
    fn mutation_order_stays_fifo_ahead_of_reads() {
        let mut runtime = AppRuntime::new(true);
        runtime.caps = AppCapabilities {
            control: true,
            list: true,
            launch: true,
            ..Default::default()
        };
        runtime.pending.insert(
            1,
            (AppCommand::Launch("com.example.a".into()), Instant::now()),
        );
        runtime.command(AppCommand::List, &|_| {});
        runtime.command(AppCommand::Launch("com.example.b".into()), &|_| {});
        runtime.command(AppCommand::Launch("com.example.c".into()), &|_| {});
        assert_eq!(
            runtime.queue.into_iter().collect::<Vec<_>>(),
            vec![
                AppCommand::Launch("com.example.b".into()),
                AppCommand::Launch("com.example.c".into()),
                AppCommand::List
            ]
        );
    }
    #[test]
    fn stale_replies_are_ignored_and_wrong_target_fails() {
        let events = RefCell::new(vec![]);
        let send = |event| events.borrow_mut().push(event);
        let mut runtime = AppRuntime::new(true);
        let reply = |id| DeviceReply {
            op: 2,
            id,
            status: 0,
            payload: r#"{"bundle_id":"com.other.app"}"#.into(),
        };
        runtime.reply(reply(1), &send);
        assert!(events.borrow().is_empty());
        runtime.pending.insert(
            2,
            (AppCommand::Launch("com.example.app".into()), Instant::now()),
        );
        runtime.reply(reply(2), &send);
        assert!(matches!(&events.borrow()[0], AppEvent::Failed { .. }));
    }
    #[test]
    fn clipboard_requires_capability_control_size_and_matching_ack() {
        let events = RefCell::new(vec![]);
        let send = |e| events.borrow_mut().push(e);
        let command = AppCommand::WriteClipboard {
            token: uuid::Uuid::new_v4().to_string(),
            text: "中文商品".into(),
        };
        let mut runtime = AppRuntime::new(true);
        runtime.command(command.clone(), &send);
        assert!(runtime.queue.is_empty());
        runtime.caps = AppCapabilities {
            control: true,
            clipboard_write: true,
            ..Default::default()
        };
        runtime.command(command.clone(), &send);
        assert_eq!(runtime.queue.len(), 1);
        runtime.queue.clear();
        runtime
            .pending
            .insert(80, (command.clone(), Instant::now()));
        runtime.reply(
            DeviceReply {
                op: 16,
                id: 80,
                status: 0,
                payload: r#"{"ok":true,"token":"wrong"}"#.into(),
            },
            &send,
        );
        assert!(matches!(
            events.borrow().last(),
            Some(AppEvent::Failed { .. })
        ));
        let AppCommand::WriteClipboard { token, .. } = &command else {
            unreachable!()
        };
        runtime
            .pending
            .insert(81, (command.clone(), Instant::now()));
        runtime.reply(
            DeviceReply {
                op: 16,
                id: 81,
                status: 0,
                payload: json!({"ok":true,"token":token}).to_string(),
            },
            &send,
        );
        assert!(matches!(
            events.borrow().last(),
            Some(AppEvent::Finished(_))
        ));
        runtime.command(
            AppCommand::WriteClipboard {
                token: token.clone(),
                text: "中".repeat(2000),
            },
            &send,
        );
        assert!(runtime.queue.is_empty());
        let mut readonly = AppRuntime::new(false);
        readonly.caps = runtime.caps.clone();
        readonly.command(command, &send);
        assert!(readonly.queue.is_empty());
    }
}
