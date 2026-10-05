//! Explicit boundary between the local assistant and the active phone session.
use super::*;
use crate::work_assistant::{AssistantEvent, WorkAssistant};
use rv_core::assistant::AssistantService;
impl SessionView {
    pub(super) fn toggle_assistant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.assistant_open = !self.assistant_open;
        if self.assistant_open && self.assistant.is_none() {
            let root = self
                .thumb_path
                .as_ref()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
                .map(PathBuf::from)
                .or_else(|| rv_core::StorePaths::default_dir().ok().map(|p| p.root));
            let Some(root) = root else {
                self.assistant_open = false;
                self.status = "无法定位本地资料目录".into();
                cx.notify();
                return;
            };
            let service = AssistantService::shared(root);
            let assistant = cx.new(|cx| WorkAssistant::new(service, window, cx));
            self.assistant_subscription = Some(cx.subscribe_in(
                &assistant,
                window,
                |this, _, event: &AssistantEvent, _, cx| match event {
                    AssistantEvent::Close => {
                        this.assistant_open = false;
                        cx.notify();
                    }
                    AssistantEvent::SendText(text) => this.assistant_send_text(text.clone(), cx),
                    AssistantEvent::Import(images) | AssistantEvent::RetryImport(images) => {
                        if this.assistant_batch.is_some()
                            || this.phase != Phase::Connected
                            || this.view_only()
                            || this.unlock.blocks_input()
                        {
                            return;
                        }
                        if let Some(assistant) = &this.assistant {
                            let service = assistant.read(cx).service.clone();
                            let durable = this.apps.caps.as_ref().is_some_and(|c| c.photo_tasks);
                            this.assistant_batch = Some(crate::photo_transfer::PhotoBatch::start(
                                service,
                                this.handle.file_client(),
                                format!("{}:{}", this.req.host, this.req.port),
                                images.clone(),
                                durable,
                                matches!(event, AssistantEvent::RetryImport(_)),
                            ));
                            this.sync_assistant(cx);
                        }
                    }
                    AssistantEvent::CancelImport => {
                        if let Some(batch) = &this.assistant_batch {
                            batch.cancel();
                        }
                    }
                    AssistantEvent::RefreshNotifications => {
                        if this.phase == Phase::Connected
                            && this.apps.caps.as_ref().is_some_and(|c| c.notifications)
                        {
                            this.handle.app(rv_session::AppCommand::Notifications);
                        }
                    }
                    AssistantEvent::Subscribe(enabled) => {
                        if this.phase == Phase::Connected
                            && this.apps.caps.as_ref().is_some_and(|c| c.notifications)
                        {
                            this.notifications.enabled = *enabled;
                            this.handle
                                .app(rv_session::AppCommand::SubscribeNotifications(*enabled));
                            this.sync_assistant(cx);
                        }
                    }
                },
            ));
            assistant.update(cx, |a, cx| {
                if let Some(snapshot) = self.notifications.snapshot.clone() {
                    a.receive_notifications(snapshot, cx);
                }
                a.load(window, cx);
            });
            self.assistant = Some(assistant);
        } else if !self.assistant_open
            && let Some(a) = &self.assistant
        {
            a.update(cx, |a, cx| a.flush(cx));
        }
        self.sync_assistant(cx);
        cx.notify();
    }
    fn assistant_send_text(&mut self, text: String, cx: &mut Context<Self>) {
        if self.phase != Phase::Connected
            || self.view_only()
            || self.unlock.blocks_input()
            || text.is_empty()
        {
            return;
        }
        if self.assistant_clipboard.is_some() {
            self.assistant_feedback("请等待上一条剪贴板写入结果".into(), cx);
            return;
        }
        if self
            .apps
            .caps
            .as_ref()
            .is_some_and(|c| c.clipboard_write && c.control)
        {
            let token = uuid::Uuid::new_v4().to_string();
            self.assistant_clipboard = Some(token.clone());
            self.handle
                .app(rv_session::AppCommand::WriteClipboard { token, text });
            self.assistant_feedback("正在写入手机剪贴板…".into(), cx);
        } else {
            if text.len() > 65536
                || (self.req.clipboard == ClipboardMode::Latin1
                    && text.chars().any(|c| c as u32 > 255))
            {
                self.assistant_feedback("当前连接不能发送此文本，请使用 UTF-8 剪贴板".into(), cx);
                return;
            }
            self.handle.copy_text(text);
            self.assistant_feedback("文本已发送，旧协议未确认；请在手机输入框粘贴".into(), cx);
        }
    }
    pub(super) fn assistant_feedback(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(a) = &self.assistant {
            a.update(cx, |a, cx| a.feedback(message, cx));
        }
    }
    pub(super) fn sync_assistant(&mut self, cx: &mut Context<Self>) {
        if let Some(batch) = &self.assistant_batch {
            let (message, done) = batch.progress();
            let recovery_required = batch.recovery_required();
            if message != self.assistant_progress {
                self.assistant_progress = message.clone();
                self.assistant_feedback(message, cx);
            }
            if done {
                if let Some(a) = &self.assistant {
                    a.update(cx, |a, cx| {
                        a.retry_import_available = recovery_required;
                        cx.notify();
                    });
                }
                self.assistant_batch = None;
            }
        }
        if let Some(a) = &self.assistant {
            let live = self.phase == Phase::Connected;
            let writable = !self.view_only()
                && !self.unlock.blocks_input()
                && self.apps.caps.as_ref().is_none_or(|c| c.control);
            let supported = self.apps.caps.as_ref().is_some_and(|c| c.notifications);
            let busy = self.assistant_batch.is_some();
            let enabled = self.notifications.enabled;
            a.update(cx, |a, cx| {
                a.set_connection(live, writable, busy, supported, enabled, cx)
            });
        }
    }
}
