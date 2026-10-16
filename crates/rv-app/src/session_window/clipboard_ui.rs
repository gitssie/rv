use super::*;
use gpui_component::WindowExt;

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
enum Feedback {
    #[default]
    Idle,
    Sending,
    Sent(std::time::Instant),
    Failed,
}

struct PendingClipboard {
    token: String,
    item: ClipboardItem,
    revision: Option<isize>,
    paste: bool,
    cancelled: bool,
    acknowledged: bool,
}

#[derive(Default)]
pub(super) struct ClipboardTransfer {
    pending: Option<PendingClipboard>,
    feedback: Feedback,
    outbound_echo: Option<(u64, std::time::Instant)>,
}
impl ClipboardTransfer {
    pub(super) fn expire_feedback(&mut self, now: std::time::Instant) -> bool {
        if matches!(self.feedback, Feedback::Sent(deadline) if now >= deadline) {
            self.feedback = Feedback::Idle;
            return true;
        }
        false
    }
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub(super) fn color(&self, cx: &App) -> Hsla {
        match self.feedback {
            Feedback::Idle => theme::ink(cx),
            Feedback::Sending => theme::accent(cx),
            Feedback::Sent(_) => theme::success(cx),
            Feedback::Failed => theme::danger(cx),
        }
    }
    pub(super) fn cancel_paste(&mut self) {
        if let Some(pending) = &mut self.pending {
            if pending.paste {
                pending.cancelled = true;
            }
        }
    }
    pub(super) fn disconnected(&mut self) {
        if self.pending.take().is_some() {
            self.feedback = Feedback::Failed;
        }
        self.outbound_echo = None;
    }
    pub(super) fn failed(&mut self, token: &str) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.token == token)
        {
            self.pending = None;
            self.feedback = Feedback::Failed;
        }
    }
    fn acknowledge(&mut self, token: &str) -> bool {
        let Some(pending) = &mut self.pending else {
            return false;
        };
        if pending.token != token || pending.acknowledged {
            return false;
        }
        pending.acknowledged = true;
        if let Some(text) = pending.item.text() {
            self.outbound_echo = Some((fingerprint(&text), std::time::Instant::now()));
        }
        true
    }
    pub(super) fn is_outbound_echo(&self, text: &str) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| pending.item.text().as_deref() == Some(text))
            || self.outbound_echo.is_some_and(|(hash, sent)| {
                sent.elapsed() < Duration::from_secs(10) && hash == fingerprint(text)
            })
    }
}

fn fingerprint(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hash);
    hash.finish()
}

impl SessionView {
    pub(super) fn begin_clipboard_transfer(&mut self, paste: bool, cx: &App) {
        if self.view_only() || self.phase != Phase::Connected || self.clipboard_transfer.busy() {
            return;
        }
        if paste
            && !self
                .apps
                .caps
                .as_ref()
                .is_some_and(|caps| caps.control && caps.clipboard_write)
        {
            self.clipboard_transfer.feedback = Feedback::Failed;
            self.status = "手机剪贴板暂不可用".into();
            return;
        }
        let revision = clipboard_revision();
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        let Some(text) = item.text().filter(|text| !text.is_empty()) else {
            return;
        };
        if revision != clipboard_revision() {
            return;
        }
        let token = uuid::Uuid::new_v4().to_string();
        self.clipboard_transfer.pending = Some(PendingClipboard {
            token: token.clone(),
            item,
            revision,
            paste,
            cancelled: false,
            acknowledged: false,
        });
        self.clipboard_transfer.feedback = Feedback::Sending;
        self.clipboard_transfer.outbound_echo =
            Some((fingerprint(&text), std::time::Instant::now()));
        self.handle
            .app(rv_session::AppCommand::WriteClipboard { token, text });
        self.status = "正在发送剪贴板".into();
    }

    pub(super) fn clipboard_transfer_finished(&mut self, token: &str, cx: &App) {
        if !self.clipboard_transfer.acknowledge(token) {
            return;
        }
        if self
            .clipboard_transfer
            .pending
            .as_ref()
            .is_some_and(|pending| !pending.paste)
        {
            let pending = self.clipboard_transfer.pending.take().unwrap();
            clear_sent_clipboard(&pending, cx);
            self.clipboard_transfer.feedback =
                Feedback::Sent(std::time::Instant::now() + Duration::from_secs(3));
        }
        // Keyboard paste is dispatched during rendering, where focus and current modifiers
        // can be checked. A write acknowledgement alone does not establish text insertion.
    }

    pub(super) fn finish_keyboard_paste(&mut self, window: &mut Window, cx: &mut App) {
        if !self
            .clipboard_transfer
            .pending
            .as_ref()
            .is_some_and(|pending| pending.paste)
        {
            return;
        }
        let blocked = !window.is_window_active()
            || !self.focus.is_focused(window)
            || self.phase != Phase::Connected
            || self.view_only()
            || self.unlock.blocks_input()
            || self.functions.open
            || self.notifications.open
            || self.apps.picker_open
            || self.show_menu
            || !self
                .apps
                .caps
                .as_ref()
                .is_some_and(|caps| caps.control && caps.clipboard_write)
            || window.has_active_dialog(cx);
        if blocked {
            self.clipboard_transfer.cancel_paste();
        }
        if !self
            .clipboard_transfer
            .pending
            .as_ref()
            .unwrap()
            .acknowledged
        {
            return;
        }
        let pending = self.clipboard_transfer.pending.take().unwrap();
        if pending.cancelled {
            self.clipboard_transfer.feedback = Feedback::Failed;
            self.status = "粘贴已取消".into();
            return;
        }
        let mods = window.modifiers();
        let mut events = self.keys.set_modifiers(false, false, true);
        events.extend(self.keys.key_down("v", Some("v"), false, false));
        events.extend(self.keys.key_up("v"));
        events.extend(
            self.keys
                .set_modifiers(mods.control, mods.alt, mods.platform),
        );
        self.send_keys(events);
        clear_sent_clipboard(&pending, cx);
        self.clipboard_transfer.feedback =
            Feedback::Sent(std::time::Instant::now() + Duration::from_secs(3));
        self.status = "剪贴板已发送".into();
    }
}

// Preserve anything copied while the phone was processing, including a new copy of
// identical text. Compare the complete item as well as the native change counter.
fn clear_sent_clipboard(pending: &PendingClipboard, cx: &App) {
    if may_clear(
        pending,
        clipboard_revision(),
        cx.read_from_clipboard().as_ref(),
    ) {
        cx.write_to_clipboard(ClipboardItem {
            entries: Vec::new(),
        });
    }
}
fn may_clear(
    pending: &PendingClipboard,
    revision: Option<isize>,
    item: Option<&ClipboardItem>,
) -> bool {
    pending.revision == revision && item == Some(&pending.item)
}

#[cfg(target_os = "macos")]
fn clipboard_revision() -> Option<isize> {
    Some(objc2_app_kit::NSPasteboard::generalPasteboard().changeCount())
}
#[cfg(not(target_os = "macos"))]
fn clipboard_revision() -> Option<isize> {
    None
}

#[cfg(test)]
mod tests {
    use super::{ClipboardTransfer, Feedback, PendingClipboard};
    use gpui::ClipboardItem;

    fn pending() -> ClipboardTransfer {
        ClipboardTransfer {
            pending: Some(PendingClipboard {
                token: "request".into(),
                item: ClipboardItem::new_string("text".into()),
                revision: Some(1),
                paste: true,
                cancelled: false,
                acknowledged: false,
            }),
            feedback: Feedback::Sending,
            outbound_echo: None,
        }
    }
    #[test]
    fn clipboard_ack_is_correlated_and_consumed_once() {
        let mut state = pending();
        assert!(!state.acknowledge("late-request"));
        assert!(!state.pending.as_ref().unwrap().acknowledged);
        assert!(state.acknowledge("request"));
        assert!(!state.acknowledge("request"));
    }
    #[test]
    fn clipboard_failure_and_disconnect_do_not_leave_a_paste_queued() {
        let mut state = pending();
        state.failed("late-request");
        assert!(state.busy());
        state.failed("request");
        assert!(!state.busy());
        assert_eq!(state.feedback, Feedback::Failed);
        assert!(!state.acknowledge("request"));
        let mut state = pending();
        state.disconnected();
        assert!(!state.busy());
        assert!(!state.acknowledge("request"));
    }
    #[test]
    fn clipboard_focus_loss_cancels_paste_even_after_focus_returns() {
        let mut state = pending();
        state.cancel_paste();
        assert!(state.acknowledge("request"));
        assert!(state.pending.as_ref().unwrap().cancelled);
    }
    #[test]
    fn clipboard_clear_preserves_new_content_and_identical_recopies() {
        let state = pending();
        let pending = state.pending.as_ref().unwrap();
        assert!(super::may_clear(pending, Some(1), Some(&pending.item)));
        assert!(!super::may_clear(pending, Some(2), Some(&pending.item)));
        assert!(!super::may_clear(
            pending,
            Some(1),
            Some(&ClipboardItem::new_string("new".into()))
        ));
        assert!(!super::may_clear(pending, Some(1), None));
    }
    #[test]
    fn clipboard_outbound_echo_does_not_refill_the_cleared_clipboard() {
        let mut state = pending();
        assert!(state.is_outbound_echo("text"));
        assert!(!state.is_outbound_echo("remote copy"));
        state.outbound_echo = Some((super::fingerprint("text"), std::time::Instant::now()));
        state.pending = None;
        assert!(state.is_outbound_echo("text"));
        state.disconnected();
        assert!(!state.is_outbound_echo("text"));
    }
}
