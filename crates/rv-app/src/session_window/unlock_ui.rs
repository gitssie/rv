use super::*;
use gpui_component::menu::{ContextMenuExt, PopupMenuItem};
use rv_core::{UnlockCode, delete_unlock_code, load_unlock_code, save_unlock_code};
use rv_session::{AppCommand, ScreenState};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Idle,
    Preparing,
    Arming,
    Typing,
    Checking,
    Locking,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnlockVisualState {
    Locked,
    Unlocking,
    Unlocked,
}

pub(super) struct UnlockUI {
    input: Entity<InputState>,
    editing: bool,
    focus_pending: bool,
    busy: bool,
    validation: Option<String>,
    code: Option<UnlockCode>,
    step: Step,
    deadline: Instant,
    next: Instant,
    index: usize,
    ready_samples: u8,
    attempt: u32,
    screen_locked: Option<bool>,
    http_active: bool,
    _subscription: Subscription,
}

impl UnlockUI {
    pub(super) fn editing(&self) -> bool {
        self.editing
    }
    pub(super) fn new(window: &mut Window, cx: &mut Context<SessionView>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("6 位数字密码")
        });
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } if this.unlock.editing => {
                    this.save_unlock_setting(window, cx)
                }
                InputEvent::Change => {
                    this.unlock.validation = None;
                    cx.notify();
                }
                _ => {}
            },
        );
        Self {
            input,
            editing: false,
            focus_pending: false,
            busy: false,
            validation: None,
            code: None,
            step: Step::Idle,
            deadline: Instant::now(),
            next: Instant::now(),
            index: 0,
            ready_samples: 0,
            attempt: 0,
            screen_locked: None,
            http_active: false,
            _subscription: subscription,
        }
    }
    pub(super) fn blocks_input(&self) -> bool {
        self.editing || self.busy || self.step != Step::Idle
    }
    pub(super) fn reset(&mut self) {
        self.code = None;
        self.step = Step::Idle;
        self.http_active = false;
        self.ready_samples = 0;
    }
    pub(super) fn disconnected(&mut self) {
        self.reset();
        self.screen_locked = None;
    }
    fn visual_state(&self) -> UnlockVisualState {
        if self.step != Step::Idle {
            UnlockVisualState::Unlocking
        } else if self.screen_locked == Some(false) {
            UnlockVisualState::Unlocked
        } else {
            UnlockVisualState::Locked
        }
    }
}

impl SessionView {
    fn can_unlock(&self) -> bool {
        self.phase == Phase::Connected
            && !self.view_only()
            && self
                .apps
                .caps
                .as_ref()
                .is_some_and(|caps| caps.control && caps.unlock)
    }
    fn can_lock(&self) -> bool {
        self.phase == Phase::Connected
            && !self.view_only()
            && self
                .apps
                .caps
                .as_ref()
                .is_some_and(|caps| caps.control && caps.lock)
    }
    fn can_screen_action(&self) -> bool {
        if self.unlock.screen_locked == Some(false) {
            self.can_lock()
        } else {
            self.can_unlock()
        }
    }
    fn start_lock(&mut self, cx: &mut Context<Self>) {
        if !self.can_lock() || self.unlock.blocks_input() {
            return;
        }
        let keys = self.keys.release_all();
        self.send_keys(keys);
        self.unlock.attempt = self.unlock.attempt.wrapping_add(1).max(1);
        self.unlock.step = Step::Locking;
        self.unlock.deadline = Instant::now() + Duration::from_secs(6);
        self.unlock.next = Instant::now() + Duration::from_millis(500);
        self.status = "".into();
        self.unlock.http_active = self
            .apps
            .caps
            .as_ref()
            .is_some_and(|caps| caps.http_screen_control);
        if self.unlock.http_active {
            self.unlock.deadline = Instant::now() + Duration::from_secs(20);
            self.handle.app(AppCommand::HttpLock(self.unlock.attempt));
        } else {
            self.handle.app(AppCommand::LockScreen(self.unlock.attempt));
        }
        cx.notify();
    }
    fn start_unlock(&mut self, code: UnlockCode, cx: &mut Context<Self>) {
        if !self.can_unlock() {
            return;
        }
        let keys = self.keys.release_all();
        self.send_keys(keys);
        self.unlock.attempt = self.unlock.attempt.wrapping_add(1).max(1);
        if self
            .apps
            .caps
            .as_ref()
            .is_some_and(|caps| caps.http_screen_control)
        {
            self.unlock.step = Step::Preparing;
            self.unlock.http_active = true;
            self.unlock.deadline = Instant::now() + Duration::from_secs(20);
            self.status = "".into();
            self.handle.app(AppCommand::HttpUnlock(
                self.unlock.attempt,
                rv_session::ScreenPasscode(String::from_utf8(code.digits().to_vec()).unwrap()),
            ));
            cx.notify();
            return;
        }
        self.unlock.code = Some(code);
        self.unlock.step = Step::Preparing;
        self.unlock.deadline = Instant::now() + Duration::from_secs(12);
        self.unlock.next = Instant::now();
        self.unlock.index = 0;
        self.unlock.ready_samples = 0;
        self.status = "".into();
        self.tick_unlock(cx);
        cx.notify();
    }
    pub(super) fn prompt_unlock_setting(&mut self, cx: &mut Context<Self>) {
        self.unlock.editing = true;
        self.unlock.focus_pending = true;
        self.unlock.validation = None;
        cx.notify();
    }

    fn edit_unlock_setting(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.unlock.busy || self.unlock.step != Step::Idle {
            return;
        }
        let keys = self.keys.release_all();
        self.send_keys(keys);
        self.unlock.editing = true;
        self.unlock.validation = None;
        self.unlock.input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        cx.notify();
    }
    fn click_unlock(&mut self, cx: &mut Context<Self>) {
        if !self.can_screen_action() || self.unlock.blocks_input() {
            return;
        }
        if self.unlock.screen_locked == Some(false) {
            self.start_lock(cx);
            return;
        }
        self.unlock.busy = true;
        let req = self.req.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { load_unlock_code(&req) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.unlock.busy = false;
                match result {
                    Ok(Some(code)) => this.start_unlock(code, cx),
                    Ok(None) => {
                        // The render pass focuses the local masked field.
                        this.unlock.editing = true;
                        this.unlock.focus_pending = true;
                        this.unlock.validation = None;
                    }
                    Err(error) => this.status = error.to_string().into(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn save_unlock_setting(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.unlock.busy {
            return;
        }
        let value = self.unlock.input.read(cx).unmask_value().to_string();
        let code = match UnlockCode::new(value) {
            Ok(code) => code,
            Err(error) => {
                self.unlock.validation = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        self.unlock
            .input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.unlock.busy = true;
        let req = self.req.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { save_unlock_code(&req, &code).map(|()| code) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.unlock.busy = false;
                match result {
                    Ok(code) => {
                        this.unlock.editing = false;
                        this.start_unlock(code, cx);
                    }
                    Err(error) => this.unlock.validation = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn clear_unlock_setting(&mut self, cx: &mut Context<Self>) {
        if self.unlock.blocks_input() {
            return;
        }
        self.unlock.busy = true;
        let req = self.req.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { delete_unlock_code(&req) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.unlock.busy = false;
                this.status = match result {
                    Ok(()) => "已清除锁屏密码".into(),
                    Err(e) => e.to_string().into(),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn finish_unlock(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        if self.unlock.http_active {
            self.handle
                .app(AppCommand::CancelScreen(self.unlock.attempt));
        } else if self.unlock.step != Step::Idle && self.unlock.step != Step::Locking {
            self.handle
                .app(AppCommand::ArmUnlock(self.unlock.attempt, 0));
        }
        self.unlock.reset();
        self.status = message.into();
        cx.notify();
    }
    pub(super) fn screen_http_progress(
        &mut self,
        attempt: u32,
        progress: rv_session::ScreenProgress,
        cx: &mut Context<Self>,
    ) {
        if !self.unlock.http_active
            || self.unlock.step == Step::Idle
            || attempt != self.unlock.attempt
        {
            return;
        }
        if let Some(state) = &progress.state {
            self.unlock.screen_locked = Some(state.locked);
        }
        match progress.phase.as_str() {
            "locked" | "unlocked" => self.finish_unlock("", cx),
            "failed" => {
                self.finish_unlock(progress.error.unwrap_or_else(|| "屏幕操作失败".into()), cx)
            }
            "preparing" => self.status = "正在准备解锁".into(),
            "input_ready" | "entering" => self.status = "正在输入锁屏密码".into(),
            "checking" => self.status = "正在确认解锁状态".into(),
            "locking" => self.status = "正在锁定".into(),
            _ => {}
        }
        cx.notify();
    }
    pub(super) fn unlock_lock_state(&mut self, locked: bool, cx: &mut Context<Self>) {
        self.unlock.screen_locked = Some(locked);
        // Passive push updates the observed state only. Readiness, keyboard input
        // and attempt completion still belong to the active RV request flow.
        cx.notify();
    }
    pub(super) fn unlock_screen_state(
        &mut self,
        attempt: u32,
        state: ScreenState,
        cx: &mut Context<Self>,
    ) {
        if self.unlock.http_active
            || self.unlock.step == Step::Idle
            || attempt != self.unlock.attempt
        {
            return;
        }
        self.unlock.screen_locked = Some(state.locked);
        if self.unlock.step == Step::Locking {
            if state.locked {
                self.finish_unlock("", cx);
            } else {
                cx.notify();
            }
            return;
        }
        if !state.locked {
            self.finish_unlock("", cx);
            return;
        }
        if self.unlock.step == Step::Preparing {
            if state.input_ready && state.input_empty {
                self.unlock.ready_samples += 1;
                if self.unlock.ready_samples >= 2 {
                    self.unlock.step = Step::Arming;
                    let digits = self.unlock.code.as_ref().unwrap().digits().len() as u8;
                    self.handle
                        .app(AppCommand::ArmUnlock(self.unlock.attempt, digits));
                }
            } else {
                self.unlock.ready_samples = 0;
                if state.input_ready && !state.input_empty {
                    self.finish_unlock("密码框已有输入，请先清空", cx);
                    return;
                }
            }
        }
        cx.notify();
    }
    pub(super) fn unlock_armed(&mut self, attempt: u32, armed: bool, cx: &mut Context<Self>) {
        if self.unlock.http_active
            || self.unlock.step != Step::Arming
            || attempt != self.unlock.attempt
        {
            return;
        }
        if armed {
            self.unlock.step = Step::Typing;
            self.unlock.next = Instant::now();
        } else {
            self.unlock.step = Step::Checking;
            self.unlock.next = Instant::now();
        }
        cx.notify();
    }
    pub(super) fn unlock_failed(
        &mut self,
        command: &AppCommand,
        message: String,
        cx: &mut Context<Self>,
    ) {
        if self.unlock.step == Step::Idle || command.unlock_attempt() != Some(self.unlock.attempt) {
            return;
        }
        // Poll requests may be retried; an arm/input request must never be replayed.
        if matches!(
            command,
            AppCommand::ScreenState(_) | AppCommand::PrepareUnlock(_)
        ) {
            self.unlock.ready_samples = 0;
        } else if matches!(
            command,
            AppCommand::ArmUnlock(_, _)
                | AppCommand::LockScreen(_)
                | AppCommand::HttpUnlock(_, _)
                | AppCommand::HttpLock(_)
        ) {
            self.finish_unlock(message, cx);
        }
    }
    pub(super) fn tick_unlock(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        if self.unlock.step == Step::Idle {
            return;
        }
        let locking = self.unlock.step == Step::Locking;
        if if locking {
            !self.can_lock()
        } else {
            !self.can_unlock()
        } {
            self.finish_unlock(
                if locking {
                    "锁屏已停止"
                } else {
                    "解锁已停止"
                },
                cx,
            );
            return;
        }
        if now >= self.unlock.deadline {
            self.finish_unlock(
                if locking {
                    "锁屏超时，请检查手机"
                } else {
                    "解锁超时，请检查手机"
                },
                cx,
            );
            return;
        }
        if self.unlock.http_active {
            return;
        }
        if now < self.unlock.next {
            return;
        }
        match self.unlock.step {
            Step::Preparing => {
                self.handle
                    .app(AppCommand::PrepareUnlock(self.unlock.attempt));
                self.unlock.next = now + Duration::from_millis(500);
            }
            Step::Checking | Step::Locking => {
                self.handle
                    .app(AppCommand::ScreenState(self.unlock.attempt));
                self.unlock.next = now + Duration::from_millis(500);
            }
            Step::Typing => {
                let digits = self.unlock.code.as_ref().unwrap().digits();
                if let Some(&digit) = digits.get(self.unlock.index) {
                    self.handle.key(digit as u32, true);
                    self.handle.key(digit as u32, false);
                    self.unlock.index += 1;
                    self.unlock.next = now + Duration::from_millis(100);
                } else {
                    self.unlock.code = None;
                    self.unlock.step = Step::Checking;
                    self.unlock.next = now + Duration::from_millis(500);
                }
            }
            _ => {}
        }
    }
    pub(super) fn render_unlock_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let state = self.unlock.visual_state();
        let (icon, color, label) = match state {
            UnlockVisualState::Locked => (
                crate::assets::AppActionIcon::Lock,
                theme::muted(cx),
                "屏幕已锁定，点击解锁",
            ),
            UnlockVisualState::Unlocking => (
                crate::assets::AppActionIcon::Lock,
                theme::accent(cx),
                if self.unlock.step == Step::Locking {
                    "正在锁屏"
                } else {
                    "正在解锁"
                },
            ),
            UnlockVisualState::Unlocked => (
                crate::assets::AppActionIcon::Unlock,
                theme::success(cx),
                "屏幕已解锁，点击锁屏",
            ),
        };
        Button::new("unlock-screen")
            .ghost()
            .icon(Icon::new(icon))
            .loading(state == UnlockVisualState::Unlocking)
            .text_color(color)
            .accessibility_label(label)
            .debug_selector(|| "unlock-screen".into())
            .disabled(
                !(if self.unlock.step == Step::Locking {
                    self.can_lock()
                } else if self.unlock.step != Step::Idle {
                    self.can_unlock()
                } else {
                    self.can_screen_action()
                }) || self.unlock.editing
                    || self.unlock.busy,
            )
            .on_click(cx.listener(|this, _, _, cx| this.click_unlock(cx)))
            .context_menu(move |menu, _, _| {
                let edit = view.clone();
                let clear = view.clone();
                menu.item(
                    PopupMenuItem::new("修改锁屏密码").on_click(move |_, window, cx| {
                        edit.update(cx, |this, cx| this.edit_unlock_setting(window, cx));
                    }),
                )
                .item(
                    PopupMenuItem::new("清除锁屏密码").on_click(move |_, _, cx| {
                        clear.update(cx, |this, cx| this.clear_unlock_setting(cx));
                    }),
                )
            })
    }
    pub(super) fn sync_unlock_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_component::{WindowExt, dialog::DialogFooter};
        if self.unlock_dialog_open == self.unlock.editing() {
            return;
        }
        self.unlock_dialog_open = self.unlock.editing();
        if !self.unlock_dialog_open {
            if window.has_active_dialog(cx) {
                window.close_dialog(cx);
            }
            self.focus.focus(window, cx);
            return;
        }
        let owner = cx.entity();
        window.open_dialog(cx, move |dialog, _, cx| {
            owner.update(cx, |this, cx| {
                let submit = cx.entity();
                let cancel = submit.clone();
                let close = submit.clone();
                dialog
                    .title("设置锁屏密码")
                    .max_h(relative(0.9))
                    .keyboard(!this.unlock.busy)
                    .close_button(!this.unlock.busy)
                    .overlay_closable(!this.unlock.busy)
                    .child(
                        v_flex()
                            .id("unlock-setting")
                            .debug_selector(|| "unlock-setting".into())
                            .gap_3()
                            .child(Input::new(&this.unlock.input).disabled(this.unlock.busy))
                            .children(
                                this.unlock
                                    .validation
                                    .clone()
                                    .map(|error| div().text_color(theme::danger(cx)).child(error)),
                            ),
                    )
                    .footer(
                        DialogFooter::new()
                            .child(
                                Button::new("unlock-cancel")
                                    .label("取消")
                                    .disabled(this.unlock.busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.cancel_unlock_dialog(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("unlock-save")
                                    .primary()
                                    .label("保存并解锁")
                                    .loading(this.unlock.busy)
                                    .disabled(this.unlock.busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_unlock_setting(window, cx)
                                    })),
                            ),
                    )
                    .on_ok(move |_, window, cx| {
                        submit.update(cx, |this, cx| this.save_unlock_setting(window, cx));
                        false
                    })
                    .on_cancel(move |_, window, cx| {
                        cancel.update(cx, |this, cx| {
                            if this.unlock.busy {
                                return false;
                            }
                            this.cancel_unlock_dialog(window, cx);
                            true
                        })
                    })
                    .on_close(move |_, window, cx| {
                        close.update(cx, |this, cx| this.cancel_unlock_dialog(window, cx))
                    })
            })
        });
        self.unlock.focus_pending = false;
        self.unlock
            .input
            .update(cx, |input, cx| input.focus(window, cx));
    }
    fn cancel_unlock_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.unlock.busy {
            return;
        }
        self.unlock.editing = false;
        self.unlock
            .input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.focus.focus(window, cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::super::ui_tests::setup;
    use super::{Instant, ScreenState, SessionView, Step, UnlockCode, UnlockVisualState};
    use gpui::{Entity, Focusable, TestAppContext, VisualTestContext};
    use rv_session::{AppCapabilities, AppCommand, AppEvent, SessionCommand, SessionTestPeer};

    fn enable(view: &Entity<SessionView>, cx: &mut VisualTestContext) {
        view.update(cx, |view, cx| {
            view.apply_app_event(
                AppEvent::Capabilities(AppCapabilities {
                    unlock: true,
                    lock: true,
                    control: true,
                    ..Default::default()
                }),
                cx,
            )
        });
    }
    fn commands(peer: &mut SessionTestPeer) -> Vec<SessionCommand> {
        std::iter::from_fn(|| peer.commands.try_recv().ok()).collect()
    }
    fn ready(locked: bool) -> ScreenState {
        ScreenState {
            locked,
            passcode_required: true,
            input_ready: true,
            input_empty: true,
            reason: String::new(),
        }
    }
    fn key_events(commands: &[SessionCommand]) -> Vec<(u32, bool)> {
        commands
            .iter()
            .filter_map(|command| match command {
                SessionCommand::Input(vnc::X11Event::KeyEvent(key)) => {
                    Some((key.keycode, key.down))
                }
                _ => None,
            })
            .collect()
    }

    #[gpui::test]
    fn http_unlock_uses_one_request_and_ignores_stale_progress_without_vnc_keys(
        cx: &mut TestAppContext,
    ) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        view.update(cx, |view, cx| {
            view.apps.caps.as_mut().unwrap().http_screen_control = true;
            view.start_unlock(UnlockCode::new("001234".into()).unwrap(), cx);
            assert!(view.unlock.code.is_none());
            view.unlock_screen_state(view.unlock.attempt, ready(true), cx);
            view.unlock_screen_state(view.unlock.attempt, ready(true), cx);
            view.unlock_armed(view.unlock.attempt, true, cx);
            for _ in 0..20 {
                view.unlock.next = Instant::now();
                view.tick_unlock(cx);
            }
            view.screen_http_progress(
                view.unlock.attempt + 1,
                rv_session::ScreenProgress {
                    phase: "unlocked".into(),
                    state: Some(ready(false)),
                    error: None,
                },
                cx,
            );
            assert!(view.unlock.blocks_input());
            view.screen_http_progress(
                view.unlock.attempt,
                rv_session::ScreenProgress {
                    phase: "unlocked".into(),
                    state: Some(ready(false)),
                    error: None,
                },
                cx,
            );
            assert!(!view.unlock.blocks_input());
            assert!(view.status.is_empty());
        });
        let sent = commands(&mut peer);
        assert_eq!(sent.len(), 2);
        assert!(
            matches!(&sent[0], SessionCommand::App(AppCommand::HttpUnlock(_, code)) if code.0 == "001234")
        );
        assert!(matches!(
            &sent[1],
            SessionCommand::App(AppCommand::CancelScreen(_))
        ));
        assert!(key_events(&sent).is_empty());
    }
    #[gpui::test]
    fn unlock_dialog_restores_remote_keyboard_focus(cx: &mut TestAppContext) {
        let (view, cx, _peer) = setup(cx, false);
        cx.update(|window, cx| view.update(cx, |view, cx| view.edit_unlock_setting(window, cx)));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| {
            view.read_with(cx, |view, cx| {
                assert!(view.unlock_dialog_open);
                assert!(
                    view.unlock
                        .input
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                );
            })
        });
        cx.update(|window, cx| view.update(cx, |view, cx| view.cancel_unlock_dialog(window, cx)));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| {
            view.read_with(cx, |view, _| {
                assert!(!view.unlock_dialog_open);
                assert!(view.focus.is_focused(window));
            })
        });
    }
    #[gpui::test]
    fn http_lock_and_failed_unlock_do_not_poll_or_replay(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        view.update(cx, |view, cx| {
            view.apps.caps.as_mut().unwrap().http_screen_control = true;
            view.unlock_lock_state(false, cx);
            view.click_unlock(cx);
            view.unlock.next = Instant::now();
            view.tick_unlock(cx);
            view.screen_http_progress(
                view.unlock.attempt,
                rv_session::ScreenProgress {
                    phase: "locked".into(),
                    state: Some(ready(true)),
                    error: None,
                },
                cx,
            );
            view.start_unlock(UnlockCode::new("001234".into()).unwrap(), cx);
            let command = AppCommand::HttpUnlock(
                view.unlock.attempt,
                rv_session::ScreenPasscode(String::new()),
            );
            view.unlock_failed(&command, "结果未知".into(), cx);
            for _ in 0..20 {
                view.tick_unlock(cx);
            }
            assert!(!view.unlock.blocks_input());
            assert!(view.status.contains("结果未知"));
        });
        let sent = commands(&mut peer);
        assert_eq!(sent.len(), 4);
        assert!(matches!(
            &sent[0],
            SessionCommand::App(AppCommand::HttpLock(_))
        ));
        assert!(matches!(
            &sent[2],
            SessionCommand::App(AppCommand::HttpUnlock(_, _))
        ));
        assert!(key_events(&sent).is_empty());
    }

    #[gpui::test]
    fn same_button_locks_unlocked_phone_once_without_password_or_success_text(
        cx: &mut TestAppContext,
    ) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        view.update(cx, |view, cx| {
            view.unlock_lock_state(false, cx);
            view.click_unlock(cx);
            view.click_unlock(cx);
            assert!(view.unlock.code.is_none());
            assert_eq!(view.unlock.visual_state(), UnlockVisualState::Unlocking);
            assert!(view.status.is_empty());
            // Poll only after the request; a pre-action observation cannot complete locking.
            view.unlock_lock_state(true, cx);
            assert!(view.unlock.step == Step::Locking);
            view.unlock_screen_state(view.unlock.attempt, ready(false), cx);
            assert!(view.unlock.step == Step::Locking);
        });
        let sent = commands(&mut peer);
        assert_eq!(sent.len(), 1);
        assert!(matches!(
            sent[0],
            SessionCommand::App(AppCommand::LockScreen(_))
        ));
        view.update(cx, |view, cx| {
            view.unlock.next = Instant::now();
            view.tick_unlock(cx);
            view.unlock_screen_state(view.unlock.attempt, ready(true), cx);
            assert_eq!(view.unlock.visual_state(), UnlockVisualState::Locked);
            assert!(view.status.is_empty());
        });
        let sent = commands(&mut peer);
        assert_eq!(sent.len(), 1);
        assert!(matches!(
            sent[0],
            SessionCommand::App(AppCommand::ScreenState(_))
        ));
        assert!(key_events(&sent).is_empty());
    }

    #[gpui::test]
    fn locking_rejects_view_only_and_older_servers_and_does_not_retry_action(
        cx: &mut TestAppContext,
    ) {
        let (view, cx, mut peer) = setup(cx, true);
        enable(&view, cx);
        view.update(cx, |view, cx| {
            view.unlock_lock_state(false, cx);
            view.click_unlock(cx);
            assert!(!view.can_screen_action());
            view.req.view_only = false;
            view.apps.caps.as_mut().unwrap().lock = false;
            view.click_unlock(cx);
            assert!(!view.can_screen_action());
        });
        assert!(commands(&mut peer).is_empty());
        view.update(cx, |view, cx| {
            view.apps.caps.as_mut().unwrap().lock = true;
            view.click_unlock(cx);
            view.unlock.deadline = Instant::now();
            view.tick_unlock(cx);
            view.unlock_screen_state(view.unlock.attempt, ready(true), cx);
            assert!(!view.unlock.blocks_input());
            assert!(view.status.contains("锁屏超时"));
        });
        let sent = commands(&mut peer);
        assert_eq!(sent.len(), 1);
        assert!(matches!(
            sent[0],
            SessionCommand::App(AppCommand::LockScreen(_))
        ));
    }

    #[gpui::test]
    fn icon_tracks_phone_state_and_unlock_progress_without_success_text(cx: &mut TestAppContext) {
        let (view, cx, _peer) = setup(cx, false);
        enable(&view, cx);
        view.update(cx, |view, cx| {
            assert_eq!(view.unlock.visual_state(), UnlockVisualState::Locked);
            view.unlock_lock_state(false, cx);
            assert_eq!(view.unlock.visual_state(), UnlockVisualState::Unlocked);
            // A physical lock updates the icon through the passive push.
            view.unlock_lock_state(true, cx);
            assert_eq!(view.unlock.visual_state(), UnlockVisualState::Locked);
            view.start_unlock(UnlockCode::new("123456".into()).unwrap(), cx);
            assert_eq!(view.unlock.visual_state(), UnlockVisualState::Unlocking);
            view.unlock_lock_state(false, cx);
            assert_eq!(view.unlock.visual_state(), UnlockVisualState::Unlocking);
            view.unlock_screen_state(view.unlock.attempt, ready(false), cx);
            assert_eq!(view.unlock.visual_state(), UnlockVisualState::Unlocked);
            assert!(
                view.status.is_empty(),
                "success is conveyed by the icon and remote screen"
            );
            // A late reply from a finished attempt must not override newer phone state.
            view.unlock_lock_state(true, cx);
            view.unlock_screen_state(view.unlock.attempt, ready(false), cx);
            assert_eq!(view.unlock.visual_state(), UnlockVisualState::Locked);
            view.unlock.disconnected();
            assert_eq!(view.unlock.screen_locked, None);
        });
    }

    #[gpui::test]
    fn idle_push_updates_icon_without_polling_or_input(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        view.update(cx, |view, cx| {
            for _ in 0..100 {
                view.tick_unlock(cx);
            }
            view.apply_app_event(AppEvent::LockState(false), cx);
            assert_eq!(view.unlock.visual_state(), UnlockVisualState::Unlocked);
            view.apply_app_event(AppEvent::LockState(true), cx);
            assert_eq!(view.unlock.visual_state(), UnlockVisualState::Locked);
            view.tick_unlock(cx);
        });
        assert!(commands(&mut peer).is_empty());
    }

    #[gpui::test]
    fn unlock_waits_for_stable_ready_and_arm_ack_then_types_once(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        view.update(cx, |view, cx| {
            view.start_unlock(UnlockCode::new("001234".into()).unwrap(), cx)
        });
        assert!(matches!(
            &commands(&mut peer)[0],
            SessionCommand::App(AppCommand::PrepareUnlock(_))
        ));
        view.update(cx, |view, cx| {
            view.unlock_screen_state(view.unlock.attempt, ready(true), cx);
            view.tick_unlock(cx);
        });
        assert!(
            commands(&mut peer).is_empty(),
            "one ready sample must not type a password"
        );
        view.update(cx, |view, cx| {
            view.unlock_screen_state(view.unlock.attempt, ready(true), cx)
        });
        assert!(matches!(
            &commands(&mut peer)[0],
            SessionCommand::App(AppCommand::ArmUnlock(_, 6))
        ));
        view.update(cx, |view, cx| {
            view.tick_unlock(cx);
            view.tap_key('9' as u32);
        });
        assert!(
            commands(&mut peer).is_empty(),
            "must wait for arm acknowledgement and block manual input"
        );
        view.update(cx, |view, cx| {
            view.unlock_armed(view.unlock.attempt, true, cx);
            for _ in 0..7 {
                view.unlock.next = Instant::now();
                view.tick_unlock(cx);
            }
            view.unlock_armed(view.unlock.attempt, true, cx); // duplicated/late reply must not replay the secret.
            view.tick_unlock(cx);
        });
        let sent = commands(&mut peer);
        let expected: Vec<_> = b"001234"
            .iter()
            .flat_map(|b| [(*b as u32, true), (*b as u32, false)])
            .collect();
        assert_eq!(key_events(&sent), expected);
        view.read_with(cx, |view, _| assert!(view.unlock.code.is_none()));
        view.update(cx, |view, cx| {
            view.unlock_screen_state(view.unlock.attempt, ready(false), cx)
        });
        assert!(key_events(&commands(&mut peer)).is_empty());
        view.read_with(cx, |view, _| assert!(!view.unlock.blocks_input()));
    }

    #[gpui::test]
    fn unlock_timeout_and_late_ack_never_send_password(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        view.update(cx, |view, cx| {
            view.start_unlock(UnlockCode::new("123456".into()).unwrap(), cx);
            view.unlock_screen_state(view.unlock.attempt, ready(true), cx);
            view.unlock_screen_state(view.unlock.attempt, ready(true), cx);
            view.unlock.deadline = Instant::now();
            view.tick_unlock(cx);
            view.unlock_armed(view.unlock.attempt, true, cx);
            view.tick_unlock(cx);
        });
        let sent = commands(&mut peer);
        assert!(key_events(&sent).is_empty());
        assert!(
            sent.iter()
                .any(|c| matches!(c, SessionCommand::App(AppCommand::ArmUnlock(_, 0))))
        );
        view.read_with(cx, |view, _| assert!(view.unlock.code.is_none()));
    }

    #[gpui::test]
    fn unlock_rejects_read_only_and_nonempty_or_unlocked_screen(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, true);
        enable(&view, cx);
        view.update(cx, |view, cx| {
            view.start_unlock(UnlockCode::new("123456".into()).unwrap(), cx)
        });
        assert!(commands(&mut peer).is_empty());
        view.update(cx, |view, cx| {
            view.req.view_only = false;
            view.start_unlock(UnlockCode::new("123456".into()).unwrap(), cx);
            let mut state = ready(true);
            state.input_empty = false;
            view.unlock_screen_state(view.unlock.attempt, state, cx);
            view.unlock_armed(view.unlock.attempt, true, cx);
        });
        assert!(key_events(&commands(&mut peer)).is_empty());
        view.update(cx, |view, cx| {
            view.start_unlock(UnlockCode::new("123456".into()).unwrap(), cx);
            view.unlock_screen_state(view.unlock.attempt, ready(false), cx);
        });
        assert!(key_events(&commands(&mut peer)).is_empty());
    }

    #[gpui::test]
    fn old_unlock_reply_cannot_start_input_in_new_attempt(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        view.update(cx, |view, cx| {
            view.start_unlock(UnlockCode::new("123456".into()).unwrap(), cx);
            let previous = view.unlock.attempt;
            view.finish_unlock("cancelled", cx);
            view.start_unlock(UnlockCode::new("654321".into()).unwrap(), cx);
            let current = view.unlock.attempt;
            view.unlock_screen_state(current, ready(true), cx);
            view.unlock_screen_state(current, ready(true), cx);
            view.unlock_armed(previous, true, cx);
            view.unlock_screen_state(previous, ready(false), cx);
            view.tick_unlock(cx);
        });
        assert!(key_events(&commands(&mut peer)).is_empty());
        view.read_with(cx, |view, _| assert!(view.unlock.code.is_some()));
    }

    #[gpui::test]
    fn unlock_settings_input_is_local_and_masked(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        view.update_in(cx, |view, window, cx| view.edit_unlock_setting(window, cx));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_input("012345");
        cx.simulate_keystrokes("backspace");
        assert!(
            commands(&mut peer).is_empty(),
            "settings must not send passcode keystrokes to the phone"
        );
        view.read_with(cx, |view, cx| {
            assert!(!view.unlock.input.read(cx).unmask_value().is_empty());
            assert!(
                view.unlock
                    .input
                    .read(cx)
                    .context_menu_capabilities()
                    .is_masked()
            );
        });
    }
}
