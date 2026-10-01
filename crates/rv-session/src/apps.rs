use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

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
    pub lock: bool,
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
    Launch(String),
    Terminate(String),
    Restart(String),
    Foreground,
    Icon(String),
    ScreenState(u32),
    PrepareUnlock(u32),
    ArmUnlock(u32, u8),
    LockScreen(u32),
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
            | Self::LockScreen(id) => Some(*id),
            _ => None,
        }
    }
    fn op(&self) -> u8 {
        match self {
            Self::List => 1,
            Self::Launch(_) => 2,
            Self::Terminate(_) => 3,
            Self::Restart(_) => 4,
            Self::Foreground => 5,
            Self::Icon(_) => 6,
            Self::ScreenState(_) => 7,
            Self::PrepareUnlock(_) => 8,
            Self::ArmUnlock(_, _) => 9,
            Self::LockScreen(_) => 10,
        }
    }
    fn mutates(&self) -> bool {
        matches!(
            self,
            Self::Launch(_)
                | Self::Terminate(_)
                | Self::Restart(_)
                | Self::PrepareUnlock(_)
                | Self::ArmUnlock(_, _)
                | Self::LockScreen(_)
        )
    }
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    Capabilities(AppCapabilities),
    LockState(bool),
    ScreenState(u32, ScreenState),
    UnlockArmed(u32, bool),
    List(Vec<RemoteApp>),
    Foreground(Option<String>),
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
    last_poll: Instant,
}

impl AppRuntime {
    pub fn new(allowed: bool) -> Self {
        Self {
            caps: AppCapabilities::default(),
            allowed,
            sequence: 0,
            queue: VecDeque::new(),
            pending: HashMap::new(),
            last_poll: Instant::now(),
        }
    }
    fn supported(&self, cmd: &AppCommand) -> bool {
        (!cmd.mutates() || (self.allowed && self.caps.control))
            && match cmd {
                AppCommand::List => self.caps.list,
                AppCommand::Launch(_) => self.caps.launch,
                AppCommand::Terminate(_) => self.caps.terminate,
                AppCommand::Restart(_) => self.caps.restart,
                AppCommand::Foreground => self.caps.foreground,
                AppCommand::Icon(_) => self.caps.icons,
                AppCommand::ScreenState(_) => self.caps.unlock || self.caps.lock,
                AppCommand::LockScreen(_) => self.caps.lock,
                AppCommand::PrepareUnlock(_) | AppCommand::ArmUnlock(_, _) => self.caps.unlock,
            }
    }
    pub fn command(&mut self, command: AppCommand, send: &impl Fn(AppEvent)) {
        if !self.supported(&command) || command.bundle_id().is_some_and(|id| !valid_bundle_id(id)) {
            send(AppEvent::Failed {
                command,
                message: "This App operation is unavailable on this connection".into(),
            });
            return;
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
        if self.caps.foreground && self.last_poll.elapsed() >= Duration::from_secs(2) {
            self.last_poll = Instant::now();
            self.command(AppCommand::Foreground, send);
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
        // A single device has one foreground app. Serialize user mutations across requests.
        if command.mutates() && self.pending.values().any(|(cmd, _)| cmd.mutates()) {
            self.queue.push_front(command);
            return;
        }
        self.sequence = self.sequence.wrapping_add(1).max(1);
        let payload = match &command {
            AppCommand::ArmUnlock(_, digits) => json!({"digits":digits}),
            _ => command
                .bundle_id()
                .map_or_else(|| json!({}), |id| json!({"bundle_id":id})),
        }
        .to_string();
        let id = self.sequence;
        match client
            .input(X11Event::Device(DeviceRequest {
                op: command.op(),
                id,
                payload,
            }))
            .await
        {
            Ok(()) => {
                self.pending.insert(id, (command, Instant::now()));
            }
            Err(error) => send(AppEvent::Failed {
                command,
                message: error.to_string(),
            }),
        }
    }
    pub fn reply(&mut self, reply: DeviceReply, send: &impl Fn(AppEvent)) {
        if reply.op == 11 {
            if reply.id == 0 && reply.status == 0 {
                if let Ok(value) = serde_json::from_str::<Value>(&reply.payload) {
                    if let Some(locked) = value["locked"].as_bool() {
                        send(AppEvent::LockState(locked));
                    }
                }
            }
            return;
        }
        if reply.op == 0 {
            if reply.status != 0 {
                return;
            }
            if let Ok(caps) = serde_json::from_str::<AppCapabilities>(&reply.payload) {
                self.caps = caps.clone();
                send(AppEvent::Capabilities(caps));
                if self.caps.list {
                    self.command(AppCommand::List, send);
                }
                if self.caps.foreground {
                    self.command(AppCommand::Foreground, send);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
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
}
