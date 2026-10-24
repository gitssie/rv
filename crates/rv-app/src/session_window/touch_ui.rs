use super::*;
use gpui_component::WindowExt;
use rv_session::{AppCommand, AppEvent};

#[derive(Default)]
pub(super) struct TouchGesture {
    next: u32,
    pending: Option<u32>,
    cancelling: bool,
    framebuffer: (u16, u16),
}
impl TouchGesture {
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub(super) fn disconnected(&mut self) {
        self.pending = None;
        self.cancelling = false;
    }
}

impl SessionView {
    pub(super) fn cancel_touch_gesture(&mut self) {
        if let Some(id) = self.touch_gesture.pending
            && !self.touch_gesture.cancelling
        {
            self.handle.app(AppCommand::CancelTouch(id));
            self.touch_gesture.cancelling = true;
            self.status = "正在停止触摸测试…".into();
        }
    }

    fn touch_gesture_allowed(&self) -> bool {
        self.phase == Phase::Connected
            && !self.view_only()
            && self.buttons == 0
            && !self.unlock.blocks_input()
            && !self.functions.open
            && !self.mailbox.open
            && !self.notifications.open
            && !self.apps.picker_open
            && !self.show_menu
            && !self.clipboard_transfer.busy()
            && !self.app_environment_busy()
            && self
                .apps
                .caps
                .as_ref()
                .is_some_and(|caps| caps.control && caps.zx_control.is_some())
    }

    fn toggle_touch_gesture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.touch_gesture.busy() {
            self.cancel_touch_gesture();
        } else if self.touch_gesture_allowed() && !window.has_active_dialog(cx) {
            self.cancel_post_paste_touch();
            let keys = self.keys.release_all();
            self.send_keys(keys);
            self.touch_gesture.next = self.touch_gesture.next.wrapping_add(1).max(1);
            let id = self.touch_gesture.next;
            self.touch_gesture.pending = Some(id);
            self.touch_gesture.cancelling = false;
            self.touch_gesture.framebuffer = (self.fb_w, self.fb_h);
            self.focus.focus(window, cx);
            self.handle.app(AppCommand::RandomTouch(id));
            self.status = "正在执行一次随机触摸轨迹…".into();
        }
        cx.notify();
    }

    pub(super) fn guard_touch_gesture(&mut self, window: &mut Window, cx: &mut App) {
        if self.touch_gesture.busy()
            && (!self.touch_gesture_allowed()
                || !window.is_window_active()
                || !self.focus.is_focused(window)
                || window.has_active_dialog(cx)
                || self.touch_gesture.framebuffer != (self.fb_w, self.fb_h))
        {
            self.cancel_touch_gesture();
        }
    }

    pub(super) fn render_touch_gesture(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.touch_gesture.busy();
        crate::ui::icon_button(
            "random-touch",
            crate::assets::FunctionIcon::Move,
            if busy {
                "停止触摸测试"
            } else {
                "执行一次随机触摸轨迹"
            },
        )
        .selected(busy)
        .disabled(self.touch_gesture.cancelling || (!busy && !self.touch_gesture_allowed()))
        .on_click(cx.listener(|this, _, window, cx| this.toggle_touch_gesture(window, cx)))
    }

    pub(super) fn touch_gesture_event(&mut self, event: &AppEvent) -> bool {
        let (id, message) = match event {
            AppEvent::Finished(AppCommand::RandomTouch(id)) => (*id, "触摸轨迹已发送".to_string()),
            AppEvent::Failed {
                command: AppCommand::RandomTouch(id),
                message,
            } => (*id, message.clone()),
            _ => return false,
        };
        if self.touch_gesture.pending == Some(id) {
            self.touch_gesture.disconnected();
            self.status = message.into();
        }
        true
    }
}
