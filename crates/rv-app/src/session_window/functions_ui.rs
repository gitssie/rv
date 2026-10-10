use super::*;
use crate::assets::FunctionIcon;
use gpui_component::input::{Textarea, TextareaState};
use rv_session::{
    FunctionCommand, FunctionEvent,
    functions::{OPERATIONS, OperationSpec, parse_operation},
};
#[path = "app_environments.rs"]
mod environments;
use environments::EnvironmentUi;

#[derive(Clone)]
struct FunctionPanel {
    editor_hidden: bool,
    email_rebinding: bool,
    email_rebind_operation: Option<String>,
    busy: bool,
    uncertain: bool,
    target_app: Option<(String, String)>,
    text: Entity<TextareaState>,
    verification_text: Entity<TextareaState>,
    verification_submitted: bool,
    verification_received: bool,
    pending_verification_code: Option<String>,
    selected: Option<&'static OperationSpec>,
    message: SharedString,
    result: Option<serde_json::Value>,
    result_open: bool,
    operation: Option<(u8, String)>,
    email_operation: Option<(u8, String)>,
    email_needs_resume: bool,
    challenge: Option<String>,
    local_data_policy: Option<&'static str>,
}
impl FunctionPanel {
    fn disconnected(&mut self) {
        if self.busy {
            self.uncertain = true;
        }
        self.busy = false;
        self.challenge = None;
        self.email_needs_resume = self.email_operation.is_some();
        if self.operation.is_none() {
            self.operation = self.email_operation.clone();
        }
        if self.operation.is_some() {
            self.message = "连接已中断；重新连接后可查询原操作".into();
        }
    }
}
pub(super) struct FunctionUi {
    panels: std::collections::HashMap<&'static str, FunctionPanel>,
    pub open: bool,
    editor_hidden: bool,
    email_rebinding: bool,
    email_rebind_operation: Option<String>,
    available: bool,
    account_state: Option<serde_json::Value>,
    account_loading: bool,
    account_error: Option<String>,
    busy: bool,
    uncertain: bool,
    target_app: Option<(String, String)>,
    environment: EnvironmentUi,
    text: Entity<TextareaState>,
    verification_text: Entity<TextareaState>,
    verification_submitted: bool,
    verification_received: bool,
    pending_verification_code: Option<String>,
    category: &'static str,
    pub(super) selected: Option<&'static OperationSpec>,
    message: SharedString,
    result: Option<serde_json::Value>,
    result_open: bool,
    operation: Option<(u8, String)>,
    email_operation: Option<(u8, String)>,
    email_needs_resume: bool,
    challenge: Option<String>,
    local_data_policy: Option<&'static str>,
}
impl FunctionUi {
    fn panel_key(&self) -> Option<&'static str> {
        self.selected.map(|s| {
            if s.id == "email-verify" {
                "email-add"
            } else {
                s.id
            }
        })
    }
    fn snapshot(&self) -> FunctionPanel {
        FunctionPanel {
            editor_hidden: self.editor_hidden.clone(),
            email_rebinding: self.email_rebinding.clone(),
            email_rebind_operation: self.email_rebind_operation.clone(),
            busy: self.busy.clone(),
            uncertain: self.uncertain.clone(),
            target_app: self.target_app.clone(),
            text: self.text.clone(),
            verification_text: self.verification_text.clone(),
            verification_submitted: self.verification_submitted.clone(),
            verification_received: self.verification_received.clone(),
            pending_verification_code: self.pending_verification_code.clone(),
            selected: self.selected.clone(),
            message: self.message.clone(),
            result: self.result.clone(),
            result_open: self.result_open.clone(),
            operation: self.operation.clone(),
            email_operation: self.email_operation.clone(),
            email_needs_resume: self.email_needs_resume.clone(),
            challenge: self.challenge.clone(),
            local_data_policy: self.local_data_policy.clone(),
        }
    }
    fn restore(&mut self, p: FunctionPanel) {
        self.editor_hidden = p.editor_hidden;
        self.email_rebinding = p.email_rebinding;
        self.email_rebind_operation = p.email_rebind_operation;
        self.busy = p.busy;
        self.uncertain = p.uncertain;
        self.target_app = p.target_app;
        self.text = p.text;
        self.verification_text = p.verification_text;
        self.verification_submitted = p.verification_submitted;
        self.verification_received = p.verification_received;
        self.pending_verification_code = p.pending_verification_code;
        self.selected = p.selected;
        self.message = p.message;
        self.result = p.result;
        self.result_open = p.result_open;
        self.operation = p.operation;
        self.email_operation = p.email_operation;
        self.email_needs_resume = p.email_needs_resume;
        self.challenge = p.challenge;
        self.local_data_policy = p.local_data_policy;
    }
    pub fn new(
        environment_apps: Vec<rv_core::AppShortcut>,
        window: &mut Window,
        cx: &mut Context<SessionView>,
    ) -> Self {
        let text = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(4)
                .placeholder("粘贴资料，或复制示例后修改")
        });
        let verification_text = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(4)
                .placeholder("六位验证码")
        });
        Self {
            panels: Default::default(),
            open: false,
            editor_hidden: false,
            email_rebinding: false,
            email_rebind_operation: None,
            available: false,
            account_state: None,
            account_loading: false,
            account_error: None,
            busy: false,
            uncertain: false,
            target_app: None,
            environment: EnvironmentUi::new(environment_apps),
            text,
            verification_text,
            verification_submitted: false,
            verification_received: false,
            pending_verification_code: None,
            category: "Apple",
            selected: None,
            message: "".into(),
            result: None,
            result_open: false,
            operation: None,
            email_operation: None,
            email_needs_resume: false,
            challenge: None,
            local_data_policy: None,
        }
    }
    fn stash_panel(&mut self) {
        if let Some(key) = self.panel_key() {
            self.panels.insert(key, self.snapshot());
        }
    }
    pub fn disconnected(&mut self) {
        self.available = false;
        self.account_state = None;
        self.account_loading = false;
        self.account_error = None;
        self.environment.clear_records();
        let mut current = self.snapshot();
        current.disconnected();
        self.restore(current);
        for panel in self.panels.values_mut() {
            panel.disconnected();
        }
    }
    fn query_target(&self) -> Option<(u8, String)> {
        self.operation.clone().or_else(|| {
            self.email_needs_resume
                .then(|| self.email_operation.clone())
                .flatten()
        })
    }
}

impl SessionView {
    #[cfg(test)]
    pub(super) fn preview_function(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.functions.open = true;
        let query_success = id == "query-result";
        let id = if query_success { "name-query" } else { id };
        let logout_success = id == "logout-success";
        let id = if logout_success { "apple-logout" } else { id };
        self.apply_function_event(FunctionEvent::Available(true), cx);
        if let Some(spec) = OPERATIONS.iter().find(|s| s.id == id) {
            if needs_app(spec) {
                self.functions.environment.open = true;
            }
            self.select_function(spec, window, cx);
            if needs_app(spec) {
                self.apply_app_event(
                    rv_session::AppEvent::List(vec![
                        rv_session::RemoteApp {
                            bundle_id: "fr.vinted".into(),
                            name: "Vinted".into(),
                            can_launch: true,
                            can_terminate: true,
                        },
                        rv_session::RemoteApp {
                            bundle_id: "com.apple.Preferences".into(),
                            name: "设置".into(),
                            can_launch: true,
                            can_terminate: true,
                        },
                    ]),
                    cx,
                );
                self.functions.target_app = Some(("fr.vinted".into(), "Vinted".into()));
            }
            self.functions.text.update(cx, |input, cx| {
                input.set_value(function_example(spec), window, cx)
            });
            if query_success {
                self.apply_function_event(FunctionEvent::State { task:31, record:serde_json::json!({
                    "operation_id":"preview-query", "method":"queryName", "status":"completed",
                    "result":{"apple_id":"demo@example.com", "last_name":"Hill", "middle_name":"", "first_name":"Adaline"}
                }) }, cx);
            }
            if logout_success {
                self.apply_function_event(FunctionEvent::State { task:31, record:serde_json::json!({
                    "operation_id":"preview-logout", "method":"signOutAppleID", "status":"completed",
                    "result":{"outcome":"succeeded","completed_domains":["icloud","identity","media"]}
                }) }, cx);
            }
        }
    }
    pub(super) fn apply_function_event(&mut self, event: FunctionEvent, cx: &mut Context<Self>) {
        match event {
            FunctionEvent::AccountStateLoading => {
                self.functions.account_loading = true;
                self.functions.account_state = None;
            }
            FunctionEvent::AccountState(state) => {
                self.functions.account_loading = false;
                match state {
                    Ok(state) => { self.functions.account_state = Some(state); self.functions.account_error = None; }
                    Err(error) => { self.functions.account_state = None; self.functions.account_error = Some(error); }
                }
            }
            FunctionEvent::ForOperation { id, event } => {
                if matches!(event.as_ref(), FunctionEvent::Available(_)) {
                    return;
                }
                let Some(spec) = OPERATIONS.iter().find(|s| s.id == id) else {
                    return;
                };
                let key = if spec.id == "email-verify" {
                    "email-add"
                } else {
                    spec.id
                };
                if self.functions.panel_key() == Some(key) {
                    self.apply_function_event(*event, cx);
                } else {
                    let previous = self.functions.snapshot();
                    let mut target = self
                        .functions
                        .panels
                        .remove(key)
                        .unwrap_or_else(|| previous.clone());
                    if target.selected.is_none_or(|s| {
                        (if s.id == "email-verify" {
                            "email-add"
                        } else {
                            s.id
                        }) != key
                    }) {
                        target.selected = Some(spec);
                        target.busy = false;
                        target.uncertain = false;
                        target.operation = None;
                        target.challenge = None;
                        target.result = None;
                    }
                    let environment = self.functions.environment.clone();
                    // Background results belong to their operation panel. Only the
                    // visible environment applies a list or starts its refresh.
                    self.functions.environment.open = false;
                    self.functions.restore(target);
                    self.apply_function_event(*event, cx);
                    self.functions.environment = environment;
                    let saved = self.functions.snapshot();
                    self.functions.panels.insert(key, saved);
                    self.functions.restore(previous);
                }
            }
            FunctionEvent::Available(available) => {
                self.functions.available = available;
                if available { self.functions.account_loading = true; }
            },
            FunctionEvent::EmailCodePending(record) => {
                self.apply_function_event(FunctionEvent::State { task: 31, record }, cx);
                self.functions.busy = true;
                self.functions.message = "正在读取邮箱验证码…".into();
            }
            FunctionEvent::EmailCodeFetched(code) => {
                self.functions.busy = false;
                if rv_session::functions::valid_code(&code) {
                    self.functions.pending_verification_code = Some(code);
                    self.functions.verification_received = true;
                    self.functions.message = if self.functions.uncertain {
                        "已回填最新邮件验证码；原验证结果仍未知，请先核对邮箱状态".into()
                    } else {
                        "已回填最新邮件验证码".into()
                    };
                }
            }
            FunctionEvent::VerificationCodeReceived(code) => {
                if rv_session::functions::valid_code(&code) {
                    self.functions.pending_verification_code = Some(code);
                    self.functions.verification_received = true;
                    self.functions.message = "已收到验证码，正在验证…".into();
                }
            }
            FunctionEvent::Progress(message) => {
                self.functions.busy = true;
                self.functions.message = message.into();
            }
            FunctionEvent::Uncertain(message) => {
                self.functions.uncertain = true;
                self.functions.busy = false;
                self.functions.message = message.into();
            }
            FunctionEvent::Error(message) => {
                // Keep operation/challenge for query or manual verification; no replay.
                self.functions.busy = false;
                self.functions.message = message.into();
            }
            FunctionEvent::State { task, record } => {
                let environment_completed = task == 32
                    && record["status"] == "completed"
                    && self.functions.environment.open;
                let Some(id) = record["operation_id"].as_str() else {
                    return;
                };
                self.functions.operation = Some((task, id.into()));
                self.functions.challenge = record["challenge"]["challenge_id"]
                    .as_str()
                    .map(str::to_owned);
                self.functions.result = Some(record.clone());
                self.functions.message = match record["status"].as_str() {
                    Some("requires_verification_code") => {
                        self.functions.busy = false;
                        "请输入六位验证码，或等待链接读取".into()
                    }
                    Some("completed") => {
                        self.functions.busy = false;
                        self.functions.result_open =
                            record["result"]["outcome"] != "requires_email_verification";
                        let unknown = rv_session::functions::uncertain_result(task, &record);
                        self.functions.email_rebinding = false;
                        self.functions.uncertain = unknown;
                        if !unknown {
                            self.functions.operation = None;
                            self.functions.challenge = None;
                        }
                        if record["method"] == "beginAddingContactEmail"
                            && record.get("error").is_none()
                            && record["result"]["outcome"] == "requires_email_verification"
                        {
                            self.functions.email_operation = Some((task, id.into()));
                            self.functions.email_needs_resume = false;
                        }
                        if record["method"] == "verifyContactEmail"
                            && record.get("error").is_none()
                            && record["result"]["outcome"] == "succeeded"
                        {
                            self.functions.email_operation = None;
                            self.functions.email_needs_resume = false;
                        }
                        if unknown {
                            "结果未知；请查询原操作，不要重复执行".into()
                        } else if record.get("error").is_some() {
                            function_error_message(&record).into()
                        } else if record["result"]["outcome"] == "requires_email_verification" {
                            self.functions.selected =
                                OPERATIONS.iter().find(|spec| spec.id == "email-verify");
                            "请输入邮箱收到的六位验证码".into()
                        } else {
                            "操作已完成，可复制结果查看详情".into()
                        }
                    }
                    _ => {
                        if self.functions.email_rebinding
                            && record["method"] == "beginAddingContactEmail"
                        {
                            self.functions.email_rebind_operation =
                                record["operation_id"].as_str().map(str::to_owned);
                        }
                        self.functions.busy = true;
                        "正在执行…".into()
                    }
                };
                if environment_completed {
                    self.finish_environment_operation(&record, cx);
                }
            }
        }
        cx.notify();
    }
    fn send_function_command(&self, command: FunctionCommand) {
        let id = match &command {
            FunctionCommand::Invoke { id, .. } => id.clone(),
            _ => self
                .functions
                .selected
                .map(|s| s.id.to_owned())
                .unwrap_or_default(),
        };
        self.handle.function(FunctionCommand::Scoped {
            id,
            command: Box::new(command),
        });
    }
    fn function_input(&self) -> &Entity<TextareaState> {
        let verification = if self.functions.busy {
            self.functions.verification_submitted
        } else {
            self.functions
                .selected
                .is_some_and(|spec| spec.id == "email-verify")
                || self.functions.challenge.is_some()
        };
        if verification {
            &self.functions.verification_text
        } else {
            &self.functions.text
        }
    }
    pub(super) fn fill_received_verification_code(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(code) = self.functions.pending_verification_code.take() {
            self.functions.verification_text.update(cx, |input, cx| {
                input.set_value(code, window, cx);
            });
        }
    }
    pub(super) fn clear_function_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.functions
            .text
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.functions
            .verification_text
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.functions.verification_submitted = false;
        self.functions.verification_received = false;
        self.functions.pending_verification_code = None;
    }
    pub(super) fn close_function_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Visibility is independent from selection, inputs and operation recovery.
        self.functions.editor_hidden = true;
        self.focus.focus(window, cx);
        cx.notify();
    }
    fn refresh_function_accounts(&mut self, cx: &mut Context<Self>) {
        if !self.functions.available || self.phase != Phase::Connected || self.functions.account_loading { return; }
        self.functions.account_loading = true;
        self.functions.account_state = None;
        self.functions.account_error = None;
        self.handle.function(FunctionCommand::RefreshAccountState);
        cx.notify();
    }
    fn select_function(
        &mut self,
        spec: &'static OperationSpec,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !account_action_visible(spec.id, self.functions.account_state.as_ref()) { return; }
        if self.functions.selected.is_some_and(|selected| {
            selected.id == spec.id || (selected.id == "email-verify" && spec.id == "email-add")
        }) {
            self.functions.editor_hidden = false;
            cx.notify();
            return;
        }
        if let Some(key) = self.functions.panel_key() {
            let state = self.functions.snapshot();
            self.functions.panels.insert(key, state);
        }
        let key = if spec.id == "email-verify" {
            "email-add"
        } else {
            spec.id
        };
        let previous = self.functions.panels.remove(key);
        let existing = previous.is_some();
        let app = self.functions.target_app.clone();
        let state = previous.unwrap_or_else(|| FunctionUi::new(Vec::new(), window, cx).snapshot());
        self.functions.restore(state);
        if spec.task == 32 {
            self.functions.target_app = app;
        }
        self.functions.editor_hidden = false;
        if needs_app(spec) && !self.functions.environment.open {
            self.open_app_environment(None, window, cx);
            return;
        }
        if !existing {
            self.functions.selected = Some(spec);
        }
        if existing {
            cx.notify();
            return;
        }
        self.functions.local_data_policy = None;
        if needs_app(spec) && self.apps.installed.is_empty() {
            self.refresh_apps(cx);
        }
        self.functions.message = "".into();
        self.functions.result = None;
        self.functions.result_open = false;
        self.clear_function_text(window, cx);
        self.function_input()
            .update(cx, |input, cx| input.focus(window, cx));
        if is_query(spec) {
            self.execute_function(window, cx);
        }
        cx.notify();
    }
    fn execute_function(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.phase != Phase::Connected
            || self.view_only()
            || !self.functions.available
            || self.functions.busy
        {
            return;
        }
        let Some(spec) = self.functions.selected else {
            return;
        };
        if !account_action_visible(spec.id, self.functions.account_state.as_ref()) {
            self.functions.message = "账户状态已改变，请重新选择操作".into();
            cx.notify(); return;
        }
        if self
            .functions
            .result
            .as_ref()
            .is_some_and(|record| operation_succeeded(spec, record))
        {
            return;
        }
        if spec.id == "email-verify" && self.functions.email_needs_resume {
            self.functions.message = "请先查询原邮箱添加结果，恢复验证步骤".into();
            cx.notify();
            return;
        }
        let text = self.function_input().read(cx).value().to_string();
        let text = if matches!(spec.id, "apple-logout" | "cloud-logout") {
            let Some(policy) = self.functions.local_data_policy else {
                self.functions.message = "请选择保留或移除本机数据".into();
                cx.notify();
                return;
            };
            format!("本机数据={policy}")
        } else if needs_app(spec) {
            let Some((id, _)) = self
                .functions
                .target_app
                .as_ref()
                .filter(|(id, _)| self.apps.installed.iter().any(|app| &app.bundle_id == id))
            else {
                self.functions.message = "请先选择手机上的 App".into();
                cx.notify();
                return;
            };
            environments::operation_text(
                spec,
                id,
                self.functions
                    .environment
                    .backup
                    .as_ref()
                    .map(|b| b.id.as_str()),
                &text,
            )
        } else {
            text
        };
        if let Err(error) = parse_operation(spec, &text) {
            self.functions.message = error.into();
            cx.notify();
            return;
        }
        self.send_function_command(FunctionCommand::Invoke {
            id: spec.id.into(),
            text,
        });
        self.functions.verification_submitted = spec.id == "email-verify";
        self.functions.busy = true;
        self.functions.result = None;
        self.functions.result_open = false;
        self.functions.message = if is_query(spec) {
            "正在读取…"
        } else {
            "正在提交…"
        }
        .into();
        // Retain the current input in memory; the rendered processing state is read-only.
        cx.notify();
    }
    fn submit_function_code(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.functions.available || self.functions.busy || self.view_only() {
            return;
        }
        let Some((_, operation_id)) = self.functions.operation.clone() else {
            return;
        };
        let Some(challenge_id) = self.functions.challenge.clone() else {
            return;
        };
        let text = self.function_input().read(cx).value().to_string();
        let code = text
            .trim()
            .strip_prefix("验证码=")
            .unwrap_or(text.trim())
            .to_owned();
        if !rv_session::functions::valid_code(&code) {
            self.functions.message = "验证码须为六位数字".into();
            cx.notify();
            return;
        }
        self.send_function_command(FunctionCommand::SubmitCode {
            operation_id,
            challenge_id,
            code,
        });
        self.functions.verification_submitted = true;
        self.functions.busy = true;
        cx.notify();
    }
    pub(super) fn render_function_button(
        &self,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Button::new("function-menu-toggle")
            .debug_selector(|| "function-menu-toggle".into())
            .ghost()
            .small()
            .h(px(28.))
            .icon(FunctionIcon::Wrench)
            .when(!compact, |button| button.label("功能"))
            .when(compact, |button| button.size(px(28.)))
            .text_color(theme::toolbar_fg())
            .selected(self.functions.open)
            .when(self.functions.open, |button| button.bg(rgb(0x2a3a50)))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_functions(window, cx)))
    }
    fn toggle_functions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.functions.environment.open && !self.environment_locked() {
            self.return_function_menu(window, cx);
            return;
        }
        self.functions.open = !self.functions.open;
        self.notifications.open = false;
        self.show_menu = false;
        self.info_open = false;
        let keys = self.keys.release_all();
        self.send_keys(keys);
        if self.functions.open {
            self.refresh_function_accounts(cx);
            self.assistant_open = false;
            if self.functions.selected.is_some() && !self.functions.editor_hidden {
                self.functions
                    .text
                    .update(cx, |input, cx| input.focus(window, cx));
            } else {
                self.focus.focus(window, cx);
            }
        } else {
            if !self.functions.busy {
                self.clear_function_text(window, cx);
            }
            self.focus.focus(window, cx);
        }
        cx.notify();
    }
    pub(super) fn render_functions(
        &self,
        width: f32,
        height: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.functions.environment.open {
            return self.render_app_environment(width, height, cx);
        }
        let selected = self
            .functions
            .selected
            .filter(|_| !self.functions.editor_hidden);
        let side_by_side = width >= 900. && height >= 480.;
        h_flex()
            .id("function-panel")
            .debug_selector(|| "function-panel".into())
            .absolute()
            .top(px(8.))
            .right(px(8.))
            .items_start()
            .gap(px(10.))
            .max_w(px((width - 16.).max(100.)))
            .when(selected.is_some(), |panel| {
                panel.child(self.render_function_editor(width, height, side_by_side, cx))
            })
            .when(selected.is_none() || side_by_side, |panel| {
                panel.child(self.render_function_menu(width, height, cx))
            })
            .into_any_element()
    }
    fn render_function_menu(
        &self,
        width: f32,
        height: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.functions.selected;
        let mut rows = Vec::new();
        let mut previous_group = "";
        for spec in OPERATIONS
            .iter()
            .filter(|spec| spec.category == self.functions.category && spec.id != "email-verify"
                && account_action_visible(spec.id, self.functions.account_state.as_ref()))
        {
            let (group, icon) = function_presentation(spec);
            if group != previous_group {
                rows.push(
                    div()
                        .pt(px(if previous_group.is_empty() { 2. } else { 10. }))
                        .pb(px(4.))
                        .px_2()
                        .text_size(px(11.))
                        .text_color(theme::toolbar_muted())
                        .when(!previous_group.is_empty(), |el| {
                            el.border_t_1().border_color(theme::toolbar_line())
                        })
                        .child(group)
                        .into_any_element(),
                );
                previous_group = group;
            }
            let active = selected.is_some_and(|s| {
                s.id == spec.id || (s.id == "email-verify" && spec.id == "email-add")
            });
            rows.push(
                h_flex()
                    .h(px(28.))
                    .rounded_md()
                    .when(active, |el| el.bg(rgb(0x2a3a50)))
                    .hover(|el| el.bg(rgb(0x29333e)))
                    .child(
                        Button::new(spec.id)
                            .debug_selector(move || format!("function-{}", spec.id))
                            .ghost()
                            .small()
                            .h(px(28.))
                            .disabled(account_action_requires_state(spec.id) &&
                                (self.functions.account_loading || !account_action_known(spec.id, self.functions.account_state.as_ref())))
                            .text_color(theme::toolbar_fg())
                            .px_2()
                            .flex_1()
                            .min_w_0()
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(Icon::new(icon).size(px(15.)).text_color(if active {
                                        theme::accent(cx)
                                    } else {
                                        theme::toolbar_fg()
                                    }))
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_left()
                                            .text_size(px(12.))
                                            .child(if spec.id == "cloud-login" && domain_signed(self.functions.account_state.as_ref(), "icloud") == Some(true) { "继续开通 iCloud" } else { spec.title }),
                                    )
                                    .child(
                                        Icon::new(IconName::ChevronRight)
                                            .size(px(12.))
                                            .text_color(theme::toolbar_muted()),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_function(spec, window, cx)
                            })),
                    )
                    .into_any_element(),
            );
        }
        v_flex()
            .id("function-browser")
            .debug_selector(|| "function-browser".into())
            .w(px(360f32.min(width - 16.)))
            .max_h(px((height - 16.).max(100.)))
            .flex_shrink_0()
            .bg(theme::toolbar())
            .text_color(theme::toolbar_fg())
            .border_1()
            .border_color(theme::toolbar_line())
            .rounded_lg()
            .shadow_lg()
            .p_2()
            .gap_2()
            .text_size(px(12.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(self.render_function_navigation(cx))
            .child(
                h_flex().min_h_0().child(
                    v_flex()
                        .id("function-list")
                        .flex_1()
                        .min_w_0()
                        .max_h(px(400f32.min((height - 100.).max(60.))))
                        .overflow_y_scroll()
                        .children(rows)
                        .when(previous_group.is_empty(), |list| {
                            list.child(
                                div()
                                    .p_2()
                                    .text_size(px(12.))
                                    .text_color(theme::toolbar_muted())
                                    .child("暂无功能"),
                            )
                        }),
                ),
            )
    }
    fn render_function_navigation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let environment = self.functions.environment.open;
        h_flex()
            .gap_1()
            .items_center()
            .child(
                h_flex().gap_1().flex_1().children(
                    [("Apple", IconName::User), ("App 环境", IconName::Folder)]
                        .into_iter()
                        .map(|(category, icon)| {
                            let active = if environment {
                                category == "App 环境"
                            } else {
                                self.functions.category == category
                            };
                            Button::new(category)
                                .debug_selector(move || format!("function-tab-{category}"))
                                .ghost()
                                .small()
                                .h(px(26.))
                                .flex_1()
                                .text_color(theme::toolbar_muted())
                                .child(
                                    h_flex()
                                        .gap_1()
                                        .items_center()
                                        .child(Icon::new(icon).size(px(13.)))
                                        .child(div().text_size(px(11.)).child(category)),
                                )
                                .when(active, |button| {
                                    button.bg(rgb(0x2a3a50)).text_color(theme::accent(cx))
                                })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if category == "App 环境" {
                                        if !this.functions.environment.open {
                                            this.open_app_environment(None, window, cx);
                                        }
                                        return;
                                    }
                                    if this.functions.environment.open {
                                        this.return_function_menu(window, cx);
                                    }
                                    this.functions.category = category;
                                    cx.notify();
                                }))
                        }),
                ),
            )
            .when(!environment, |row| row.child(
                Button::new("function-account-refresh")
                    .debug_selector(|| "function-account-refresh".into())
                    .ghost().size(px(24.))
                    .icon(Icon::new(IconName::Redo).size(px(14.)))
                    .loading(self.functions.account_loading)
                    .disabled(!self.functions.available || self.functions.account_loading)
                    .tooltip(self.functions.account_error.clone().unwrap_or_else(|| "刷新账户状态".into()))
                    .text_color(theme::toolbar_muted())
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_function_accounts(cx))),
            ))
            .child(
                Button::new("function-close")
                    .ghost()
                    .size(px(24.))
                    .icon(Icon::new(IconName::Close).size(px(14.)))
                    .text_color(theme::toolbar_fg())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.functions.open = false;
                        if !this.functions.busy && this.functions.operation.is_none() {
                            this.clear_function_text(window, cx);
                        }
                        this.focus.focus(window, cx);
                        cx.notify();
                    })),
            )
    }
    fn render_function_editor(
        &self,
        width: f32,
        height: f32,
        side_by_side: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.functions.selected;
        let live = self.functions.available && self.phase == Phase::Connected && !self.view_only();
        let locked = self.functions.busy;
        let operation_done = selected.is_some_and(|spec| {
            self.functions
                .result
                .as_ref()
                .is_some_and(|record| operation_succeeded(spec, record))
        });
        let query = selected.is_some_and(is_query);
        let completed = self.functions.result.as_ref().is_some_and(|record| {
            record["status"] == "completed"
                && record.get("error").is_none()
                && !self.functions.uncertain
                && record["result"]["outcome"] != "requires_email_verification"
        });
        v_flex()
            .id("function-editor")
            .debug_selector(|| "function-editor".into())
            .w(px((if selected
                .is_some_and(|spec| matches!(spec.id, "apple-logout" | "cloud-logout"))
            {
                320f32
            } else {
                360f32
            })
            .min(width - 16.)))
            .max_h(px((height - if side_by_side { 56. } else { 16. }).max(100.)))
            .when(side_by_side, |panel| panel.mt(px(40.)))
            .overflow_y_scroll()
            .flex_shrink_0()
            .p_3()
            .gap_2()
            .bg(theme::toolbar())
            .text_color(theme::toolbar_fg())
            .border_1()
            .border_color(theme::toolbar_line())
            .rounded_lg()
            .shadow_lg()
            .text_size(px(12.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .when_some(selected, |panel, spec| {
                panel.child(
                    v_flex()
                        .gap_1()
                        .child(
                            h_flex()
                                .items_center()
                                .gap_1()
                                .pb_1()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_size(px(13.))
                                        .font_semibold()
                                        .child(spec.title),
                                )
                                .when(query, |row| {
                                    row.child(
                                        tool_btn(
                                            cx,
                                            "function-query-refresh",
                                            crate::assets::AppActionIcon::Restart,
                                            "刷新原始结果",
                                            cx.listener(|this, _, window, cx| {
                                                this.execute_function(window, cx)
                                            }),
                                        )
                                        .size(px(24.))
                                        .icon(
                                            Icon::new(crate::assets::AppActionIcon::Restart)
                                                .size(px(14.)),
                                        )
                                        .debug_selector(|| "function-query-refresh".into())
                                        .disabled(!live || locked),
                                    )
                                    .child(
                                        tool_btn(
                                            cx,
                                            "function-copy-result",
                                            IconName::Copy,
                                            "复制原始结果",
                                            cx.listener(|this, _, _, cx| {
                                                if let Some(result) = &this.functions.result {
                                                    cx.write_to_clipboard(
                                                        ClipboardItem::new_string(
                                                            function_response_text(result),
                                                        ),
                                                    );
                                                }
                                            }),
                                        )
                                        .size(px(24.))
                                        .icon(Icon::new(IconName::Copy).size(px(14.)))
                                        .disabled(self.functions.result.is_none()),
                                    )
                                })
                                .child(
                                    tool_btn(
                                        cx,
                                        "function-editor-close",
                                        IconName::Close,
                                        "关闭",
                                        cx.listener(|this, _, window, cx| {
                                            this.close_function_editor(window, cx);
                                        }),
                                    )
                                    .debug_selector(|| "function-editor-close".into())
                                    .size(px(24.))
                                    .icon(Icon::new(IconName::Close).size(px(14.))),
                                ),
                        )
                        .when(self.functions.environment.open, |panel| {
                            panel.child(self.render_environment_context(cx))
                        })
                        .when(
                            !operation_done
                                && !self.functions.busy
                                && matches!(spec.id, "apple-logout" | "cloud-logout"),
                            |panel| {
                                panel.child(
                                    h_flex()
                                        .justify_between()
                                        .gap_2()
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(theme::toolbar_muted())
                                                .child("本机数据"),
                                        )
                                        .child(
                                            h_flex()
                                                .p(px(2.))
                                                .gap(px(2.))
                                                .rounded_md()
                                                .bg(rgb(0x151b22))
                                                .children(
                                                    [("keep", "保留"), ("remove", "移除")].map(
                                                        |(policy, label)| {
                                                            let active =
                                                                self.functions.local_data_policy
                                                                    == Some(policy);
                                                            Button::new(SharedString::from(
                                                                format!("logout-data-{policy}"),
                                                            ))
                                                            .ghost()
                                                            .small()
                                                            .border_0()
                                                            .debug_selector(move || {
                                                                format!("logout-data-{policy}")
                                                            })
                                                            .h(px(24.))
                                                            .w(px(64.))
                                                            .rounded(px(4.))
                                                            .bg(if active {
                                                                rgb(0x334356)
                                                            } else {
                                                                rgb(0x151b22)
                                                            })
                                                            .text_color(if active {
                                                                theme::toolbar_fg()
                                                            } else {
                                                                theme::toolbar_muted()
                                                            })
                                                            .text_size(px(12.))
                                                            .when(active, |choice| {
                                                                choice.icon(
                                                                    Icon::new(IconName::Check)
                                                                        .size(px(12.)),
                                                                )
                                                            })
                                                            .label(label)
                                                            .disabled(!live || locked)
                                                            .on_click(cx.listener(
                                                                move |this, _, _, cx| {
                                                                    this.functions
                                                                        .local_data_policy =
                                                                        Some(policy);
                                                                    this.functions.message =
                                                                        "".into();
                                                                    cx.notify();
                                                                },
                                                            ))
                                                        },
                                                    ),
                                                ),
                                        ),
                                )
                            },
                        )
                        .when(
                            !operation_done
                                && (self.function_has_text(spec)
                                    || self.functions.challenge.is_some()),
                            |panel| {
                                panel.child(div().text_size(px(12.)).child(
                                    if self.functions.busy {
                                        "已提交资料"
                                    } else if self.functions.challenge.is_some()
                                        || spec.id == "email-verify"
                                    {
                                        "粘贴验证码"
                                    } else {
                                        "粘贴操作资料"
                                    },
                                ))
                            },
                        )
                        .when(
                            !operation_done
                                && (self.function_has_text(spec)
                                    || self.functions.challenge.is_some()),
                            |panel| {
                                panel.child(
                                    div()
                                        .debug_selector(|| "function-input-area".into())
                                        .rounded_md()
                                        .border_1()
                                        .border_color(if self.functions.busy {
                                            theme::toolbar_line()
                                        } else {
                                            theme::accent(cx).opacity(0.65)
                                        })
                                        .bg(rgb(0x28323d))
                                        .text_color(theme::toolbar_fg())
                                        .child(
                                            Textarea::new(self.function_input())
                                                .readonly(self.functions.busy)
                                                .h(px(100.))
                                                .appearance(false)
                                                .bordered(false)
                                                .font_family("Menlo")
                                                .text_size(px(12.))
                                                .text_color(theme::toolbar_fg()),
                                        ),
                                )
                            },
                        )
                        .when(
                            self.functions.verification_received
                                && (self.functions.busy || operation_done),
                            |panel| {
                                panel.child(
                                    h_flex()
                                        .debug_selector(|| "function-received-code".into())
                                        .gap_2()
                                        .items_center()
                                        .child(div().text_size(px(12.)).child("验证码"))
                                        .child(
                                            Textarea::new(&self.functions.verification_text)
                                                .readonly(true)
                                                .h(px(28.))
                                                .w(px(112.))
                                                .appearance(false)
                                                .bordered(false)
                                                .font_family("Menlo")
                                                .text_size(px(12.))
                                                .text_color(theme::toolbar_fg()),
                                        ),
                                )
                            },
                        )
                        .when(!operation_done && !query && !self.functions.busy, |panel| {
                            panel.child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(theme::toolbar_muted())
                                    .child(match spec.id {
                                        "cloud-logout" => "退出手机当前的 iCloud",
                                        "apple-logout" => "退出手机当前的 Apple 账户",
                                        _ => spec.hint,
                                    }),
                            )
                        })
                        .when(!operation_done && !query && !self.functions.busy, |panel| {
                            panel.child(
                                h_flex()
                                    .justify_end()
                                    .gap_1()
                                    .when(self.function_has_text(spec) && !locked, |row| {
                                        row.child(
                                            Button::new("function-example")
                                                .ghost()
                                                .text_color(theme::toolbar_fg())
                                                .small()
                                                .h(px(26.))
                                                .label("复制示例")
                                                .icon(IconName::Copy)
                                                .text_color(theme::accent(cx))
                                                .disabled(locked)
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    cx.write_to_clipboard(
                                                        ClipboardItem::new_string(
                                                            environments::example(
                                                                spec,
                                                                this.functions
                                                                    .environment
                                                                    .backup
                                                                    .is_some(),
                                                            ),
                                                        ),
                                                    )
                                                })),
                                        )
                                    })
                                    .when(
                                        matches!(spec.id, "email-add" | "email-verify")
                                            && self.functions.uncertain,
                                        |row| {
                                            row.child(
                                                Button::new("function-email-rebind")
                                                    .ghost()
                                                    .small()
                                                    .h(px(26.))
                                                    .label("重新发送绑定")
                                                    .text_color(theme::accent(cx))
                                                    .disabled(!live || self.functions.busy)
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        if this.functions.busy {
                                                            return;
                                                        }
                                                        let text = this
                                                            .functions
                                                            .text
                                                            .read(cx)
                                                            .value()
                                                            .to_string();
                                                        this.functions.email_rebinding = true;
                                                        this.functions.email_rebind_operation =
                                                            None;
                                                        this.send_function_command(
                                                            FunctionCommand::RebindEmail { text },
                                                        );
                                                        this.functions.selected = OPERATIONS
                                                            .iter()
                                                            .find(|s| s.id == "email-add");
                                                        this.functions.busy = true;
                                                        this.functions.result_open = false;
                                                        this.functions.message =
                                                            "正在核对邮箱并初始化新绑定…".into();
                                                        cx.notify();
                                                    })),
                                            )
                                        },
                                    )
                                    .when(spec.id == "email-verify", |row| {
                                        row.child(
                                            Button::new("function-email-refetch")
                                                .ghost()
                                                .small()
                                                .h(px(26.))
                                                .icon(IconName::Redo)
                                                .label("重新获取验证码")
                                                .text_color(theme::accent(cx))
                                                .disabled(!live || self.functions.busy)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    if this.functions.busy {
                                                        return;
                                                    }
                                                    let text = this
                                                        .functions
                                                        .text
                                                        .read(cx)
                                                        .value()
                                                        .to_string();
                                                    this.send_function_command(
                                                        FunctionCommand::ReadEmailCode { text },
                                                    );
                                                    this.functions.busy = true;
                                                    this.functions.message =
                                                        "正在重新读取最新邮件验证码…".into();
                                                    cx.notify();
                                                })),
                                        )
                                    })
                                    .child(div().flex_1())
                                    .child(
                                        Button::new("function-editor-cancel")
                                            .ghost()
                                            .bg(theme::toolbar())
                                            .text_color(theme::toolbar_fg())
                                            .disabled(locked)
                                            .small()
                                            .h(px(26.))
                                            .label("取消")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.functions.stash_panel();
                                                this.functions.editor_hidden = true;
                                                this.functions.environment.backup = None;
                                                this.focus.focus(window, cx);
                                                cx.notify();
                                            })),
                                    )
                                    .when(self.functions.challenge.is_none(), |row| {
                                        row.child(
                                            Button::new("function-execute")
                                                .loading(self.functions.busy)
                                                .debug_selector(|| "function-execute".into())
                                                .primary()
                                                .small()
                                                .h(px(26.))
                                                .label(
                                                    if spec.id == "store-login"
                                                        || spec.id == "cloud-login"
                                                    {
                                                        "解析并登录"
                                                    } else {
                                                        match spec.id {
                                                            "app-save" => "保存",
                                                            "app-switch" => "切换",
                                                            "app-delete" => "确认删除",
                                                            "app-edit" => "保存修改",
                                                            "store-logout" => "退出 App Store",
                                                            "cloud-logout" => "退出 iCloud",
                                                            "apple-logout" => "退出账户",
                                                            "device-remove" => "确认移除其它设备",
                                                            "email-add" => "添加并验证",
                                                            "email-verify" => "验证",
                                                            _ => "执行",
                                                        }
                                                    },
                                                )
                                                .bg(theme::accent(cx))
                                                .text_color(theme::accent_fg(cx))
                                                .disabled(
                                                    !live
                                                        || locked
                                                        || (spec.id == "email-verify"
                                                            && self.functions.email_needs_resume)
                                                        || (matches!(
                                                            spec.id,
                                                            "apple-logout" | "cloud-logout"
                                                        ) && self
                                                            .functions
                                                            .local_data_policy
                                                            .is_none())
                                                        || (needs_app(spec)
                                                            && self.functions.target_app.is_none()),
                                                )
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.execute_function(window, cx)
                                                })),
                                        )
                                    })
                                    .when(self.functions.challenge.is_some(), |row| {
                                        row.child(
                                            Button::new("function-read-code")
                                                .small()
                                                .h(px(26.))
                                                .label("重新读取链接")
                                                .disabled(!live || self.functions.busy)
                                                .on_click(cx.listener(|this, _, _, _| {
                                                    this.send_function_command(
                                                        FunctionCommand::FetchCode,
                                                    )
                                                })),
                                        )
                                        .child(
                                            Button::new("function-submit-code")
                                                .primary()
                                                .small()
                                                .h(px(26.))
                                                .label("提交验证码")
                                                .bg(theme::accent(cx))
                                                .text_color(theme::accent_fg(cx))
                                                .disabled(!live || self.functions.busy)
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.submit_function_code(window, cx)
                                                })),
                                        )
                                    }),
                            )
                        }),
                )
            })
            .when(
                selected.is_some_and(|spec| matches!(spec.id, "store-login" | "cloud-login"))
                    && self.functions.message.is_empty(),
                |panel| {
                    panel.child(
                        h_flex()
                            .gap_1()
                            .text_size(px(11.))
                            .text_color(theme::toolbar_muted())
                            .child(Icon::new(IconName::Info).size(px(12.)))
                            .child("需要验证时，再读取链接。"),
                    )
                },
            )
            .when(
                self.functions.challenge.is_some() && !self.functions.busy,
                |panel| {
                    panel.child(
                        Button::new("function-cancel-verification")
                            .ghost()
                            .text_color(theme::toolbar_fg())
                            .small()
                            .h(px(26.))
                            .label("取消验证码验证")
                            .disabled(!live || self.functions.busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let (Some((_, operation_id)), Some(challenge_id)) = (
                                    this.functions.operation.clone(),
                                    this.functions.challenge.clone(),
                                ) {
                                    this.send_function_command(FunctionCommand::CancelChallenge {
                                        operation_id,
                                        challenge_id,
                                    });
                                    this.functions.busy = true;
                                    cx.notify();
                                }
                            })),
                    )
                },
            )
            .when(!live, |panel| {
                panel.child(div().text_xs().text_color(theme::toolbar_muted()).child(
                    if self.view_only() {
                        "只读连接不能执行手机操作"
                    } else {
                        "等待 ZXTouch 功能连接"
                    },
                ))
            })
            .when(self.functions.busy, |panel| {
                panel.child(
                    h_flex()
                        .id("function-query-loading")
                        .debug_selector(|| "function-query-loading".into())
                        .items_center()
                        .gap_2()
                        .py_2()
                        .child(
                            gpui_component::spinner::Spinner::new()
                                .small()
                                .color(theme::toolbar_muted()),
                        )
                        .child(
                            div()
                                .debug_selector(|| "function-operation-message".into())
                                .text_xs()
                                .text_color(theme::toolbar_muted())
                                .child(if query {
                                    SharedString::from("正在读取…")
                                } else if self.functions.message.is_empty() {
                                    SharedString::from("正在处理…")
                                } else {
                                    self.functions.message.clone()
                                }),
                        ),
                )
            })
            .when(
                !completed && !self.functions.message.is_empty() && !self.functions.busy,
                |panel| {
                    panel.child(
                        div()
                            .debug_selector(|| "function-operation-message".into())
                            .text_xs()
                            .child(self.functions.message.clone()),
                    )
                },
            )
            .when(
                self.functions.query_target().is_some() && !self.functions.busy,
                |panel| {
                    panel.child(
                        Button::new("function-query")
                            .ghost()
                            .text_color(theme::toolbar_fg())
                            .small()
                            .h(px(26.))
                            .debug_selector(|| "function-query-resume".into())
                            .label("查询原操作结果")
                            .disabled(!live || self.functions.busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some((task, operation_id)) = this.functions.query_target() {
                                    this.send_function_command(FunctionCommand::Resume {
                                        task,
                                        operation_id,
                                    });
                                    this.functions.busy = true;
                                    cx.notify();
                                }
                            })),
                    )
                },
            )
            .when(completed && !query, |panel| {
                let record = self.functions.result.as_ref().unwrap();
                let (title, detail) = result_summary(record);
                panel.child(
                    h_flex()
                        .id("function-result-summary")
                        .debug_selector(|| "function-result-summary".into())
                        .gap_2()
                        .py_1()
                        .items_start()
                        .child(
                            Icon::new(IconName::Check)
                                .size(px(15.))
                                .text_color(rgb(0x7cc4a4)),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .flex_1()
                                .child(div().text_size(px(12.)).child(title))
                                .when(!detail.is_empty(), |text| {
                                    text.child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(theme::toolbar_muted())
                                            .child(detail),
                                    )
                                }),
                        ),
                )
            })
            .when(
                !query
                    && !self.functions.busy
                    && self
                        .functions
                        .result
                        .as_ref()
                        .is_some_and(|record| record["status"] == "completed"),
                |panel| {
                    panel.child(
                        h_flex()
                            .justify_end()
                            .gap_1()
                            .child(
                                Button::new("function-view-result")
                                    .ghost()
                                    .small()
                                    .h(px(24.))
                                    .text_color(theme::toolbar_muted())
                                    .label(if self.functions.result_open {
                                        "收起详情"
                                    } else {
                                        "详情"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.functions.result_open = !this.functions.result_open;
                                        cx.notify();
                                    })),
                            )
                            .when(self.functions.result_open, |row| {
                                row.child(
                                    Button::new("function-copy-result")
                                        .ghost()
                                        .size(px(24.))
                                        .icon(Icon::new(IconName::Copy).size(px(14.)))
                                        .text_color(theme::toolbar_muted())
                                        .tooltip("复制原始结果")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(result) = &this.functions.result {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    function_response_text(result),
                                                ));
                                            }
                                        })),
                                )
                            }),
                    )
                },
            )
            .when(
                self.functions
                    .result
                    .as_ref()
                    .is_some_and(|record| record["status"] == "completed")
                    && !self.functions.busy
                    && (query || self.functions.result_open),
                |panel| {
                    let text = self
                        .functions
                        .result
                        .as_ref()
                        .map(function_response_text)
                        .unwrap_or_default();
                    panel.child(
                        div()
                            .id("function-result-text")
                            .debug_selector(|| "function-result-text".into())
                            .p_2()
                            .rounded_md()
                            .bg(rgb(0x151b22))
                            .font_family("Menlo")
                            .text_color(theme::toolbar_muted())
                            .border_1()
                            .border_color(theme::toolbar_line())
                            .max_h(px(
                                (height - if side_by_side { 156. } else { 116. }).clamp(60., 320.)
                            ))
                            .overflow_scroll()
                            .text_size(px(11.))
                            .line_height(px(17.))
                            .child(div().whitespace_nowrap().child(raw_json_text(text))),
                    )
                },
            )
    }
}

fn function_response_text(record: &serde_json::Value) -> String {
    let mut response = record.clone();
    if let Some(object) = response.as_object_mut() {
        for key in ["operation_id", "method", "action", "params", "request"] {
            object.remove(key);
        }
    }
    serde_json::to_string_pretty(&response).unwrap_or_default()
}

fn raw_json_text(text: String) -> gpui::StyledText {
    let mut highlights = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'"' {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' => i = (i + 2).min(bytes.len()),
                b'"' => {
                    i += 1;
                    break;
                }
                _ => i += 1,
            }
        }
        let end = i;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let color = if bytes.get(i) == Some(&b':') {
            rgb(0x9cc7ec)
        } else {
            rgb(0xdddbc7)
        };
        highlights.push((
            start..end,
            gpui::HighlightStyle {
                color: Some(color.into()),
                ..Default::default()
            },
        ));
    }
    gpui::StyledText::new(text).with_highlights(highlights)
}

fn domain_signed(state: Option<&serde_json::Value>, domain: &str) -> Option<bool> {
    let section = &state?[domain];
    match section["status"].as_str() {
        Some("signed_in") if section["account"].is_object() => Some(true),
        Some("not_signed_in") if section["account"].is_null() => Some(false),
        _ => None,
    }
}
fn account_action_requires_state(id: &str) -> bool {
    matches!(id, "store-login" | "store-logout" | "cloud-login" | "cloud-logout" | "apple-logout" | "findmy-off")
}
fn account_action_known(id: &str, state: Option<&serde_json::Value>) -> bool {
    match id {
        "store-login" | "store-logout" => domain_signed(state, "app_store").is_some(),
        "cloud-login" | "cloud-logout" => domain_signed(state, "icloud").is_some(),
        "findmy-off" => domain_signed(state, "icloud").is_some() && state.and_then(|s| s["icloud"]["find_my_enabled"].as_bool()).is_some(),
        "apple-logout" => ["app_store", "icloud", "identity"].iter().all(|d| domain_signed(state, d).is_some()),
        _ => true,
    }
}
fn account_action_visible(id: &str, state: Option<&serde_json::Value>) -> bool {
    match id {
        "store-login" => domain_signed(state, "app_store") != Some(true),
        "store-logout" => domain_signed(state, "app_store") != Some(false),
        "cloud-login" => !(domain_signed(state, "icloud") == Some(true) && state.and_then(|s| s["icloud"]["account"]["enabled_dataclasses"].as_array()).is_some_and(|classes| !classes.is_empty())),
        "cloud-logout" => domain_signed(state, "icloud") != Some(false),
        "apple-logout" => !["app_store", "icloud", "identity"].iter().all(|d| domain_signed(state, d) == Some(false)),
        "findmy-off" => domain_signed(state, "icloud") != Some(false) && state.and_then(|s|s["icloud"]["find_my_enabled"].as_bool()) != Some(false),
        _ => true,
    }
}

fn is_query(spec: &OperationSpec) -> bool {
    spec.method.starts_with("query") || matches!(spec.method, "list" | "describe" | "status")
}

fn operation_succeeded(spec: &OperationSpec, record: &serde_json::Value) -> bool {
    !is_query(spec)
        && record["method"] == spec.method
        && record["status"] == "completed"
        && record.get("error").is_none()
        && (record["result"]["outcome"] == "succeeded"
            || (spec.task == 32 && record["result"].is_object()))
}

fn result_summary(record: &serde_json::Value) -> (String, String) {
    let title = match record["method"].as_str().unwrap_or_default() {
        "signOutICloud" => "已退出 iCloud",
        "signOutAppleID" => "已退出 Apple 账户",
        "signInAppStore" => "已登录 App Store",
        "enableICloud" => "已登录 iCloud",
        "disableFindMy" => "已关闭查找",
        "setName" => "姓名已更新",
        "setForwardingEmail" => "转发邮箱已更新",
        "beginAddingContactEmail"
            if record["result"]["outcome"] == "requires_email_verification" =>
        {
            "等待邮箱验证码"
        }
        "verifyContactEmail" => "邮箱已验证",
        "queryState"
        | "queryName"
        | "queryEmail"
        | "querySignInWithAppleApps"
        | "queryICloudDevices" => "已读取",
        _ => "已完成",
    };
    let detail = record["result"]["completed_domains"]
        .as_array()
        .map(|domains| {
            let mut labels = Vec::new();
            for domain in domains {
                let label = match domain.as_str() {
                    Some("icloud") => "iCloud",
                    Some("identity") => "Apple 身份",
                    Some("media") => "App Store",
                    _ => continue,
                };
                if !labels.contains(&label) {
                    labels.push(label);
                }
            }
            labels.join(" · ")
        })
        .unwrap_or_default();
    (title.into(), detail)
}

fn function_error_message(record: &serde_json::Value) -> &'static str {
    if record["error"]["domain"] == "org.trollvnc.apple" {
        match record["error"]["code"].as_i64() {
            Some(1) => "手机暂不支持此操作",
            Some(2) => "手机服务权限不足",
            Some(4) => "请先关闭「查找」",
            Some(5) => "无法读取手机账户",
            Some(6) => "结果未确认，请先检查手机账户状态",
            Some(7) => "手机账户状态不一致",
            Some(8) => "账户验证未通过",
            Some(9) => "验证码无效或已过期",
            _ => "操作未完成，请查看详情",
        }
    } else {
        "操作未完成，请查看详情"
    }
}

fn needs_app(spec: &OperationSpec) -> bool {
    spec.task == 32 && spec.fields.first() == Some(&"bundle_id")
}
fn function_example(spec: &OperationSpec) -> String {
    if needs_app(spec) {
        spec.example
            .lines()
            .filter(|line| !line.starts_with("App="))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        spec.example.into()
    }
}
fn function_app_text(id: &str, text: &str) -> String {
    if text.trim().is_empty() {
        return format!("App={id}");
    }
    let named = text
        .lines()
        .find(|line| !line.is_empty())
        .and_then(|line| line.split_once('='))
        .is_some_and(|(key, _)| {
            matches!(
                key.trim(),
                "App"
                    | "应用"
                    | "bundle_id"
                    | "名称"
                    | "备注"
                    | "备份"
                    | "放弃当前环境"
                    | "display_name"
                    | "note"
                    | "backup_id"
                    | "discard_current"
            )
        });
    if named {
        format!("App={id}\n{text}")
    } else {
        format!("{id}----{text}")
    }
}
fn function_presentation(spec: &OperationSpec) -> (&'static str, FunctionIcon) {
    match spec.id {
        "store-login" => ("登录与退出", FunctionIcon::User),
        "cloud-login" => ("登录与退出", FunctionIcon::Cloud),
        "apple-logout" | "cloud-logout" | "store-logout" | "findmy-off" => {
            ("登录与退出", FunctionIcon::LogOut)
        }
        "name" => ("资料与邮箱", FunctionIcon::User),
        "email-add" | "email-verify" | "forward" => ("资料与邮箱", FunctionIcon::Mail),
        "authorization-list" | "authorization-stop" => ("授权与设备", FunctionIcon::Shield),
        "device-list" | "device-remove" => ("授权与设备", FunctionIcon::Monitor),
        "app-save" | "app-switch" | "app-baseline" | "app-list" | "app-edit" | "app-delete" => {
            ("环境与备份", FunctionIcon::Folder)
        }
        "app-open" | "app-stop" => ("App 操作", FunctionIcon::Play),
        _ => ("按需查询", FunctionIcon::Search),
    }
}

#[cfg(test)]
mod tests {
    fn unscoped(cmd: rv_session::SessionCommand) -> rv_session::SessionCommand {
        match cmd {
            rv_session::SessionCommand::Function(FunctionCommand::Scoped { command, .. }) => {
                rv_session::SessionCommand::Function(*command)
            }
            c => c,
        }
    }

    use super::super::ui_tests::setup;
    use super::{FunctionCommand, FunctionEvent, OPERATIONS, SessionView};
    use gpui::{Entity, TestAppContext, VisualTestContext, px, size};

    fn enable(view: &Entity<SessionView>, cx: &mut VisualTestContext) {
        view.update(cx, |view, cx| {
            view.functions.open = true;
            view.apply_function_event(FunctionEvent::Available(true), cx);
        });
    }
    #[test]
    fn menu_actions_follow_independent_account_domains() {
        use serde_json::json;
        let mut state=json!({"app_store":{"status":"signed_in","account":{"authenticated":true}},
            "icloud":{"status":"not_signed_in","account":null},"identity":{"status":"signed_in","account":{}}});
        assert!(!super::account_action_visible("store-login",Some(&state)));
        assert!(super::account_action_visible("store-logout",Some(&state)));
        assert!(super::account_action_visible("cloud-login",Some(&state)));
        assert!(!super::account_action_visible("cloud-logout",Some(&state)));
        assert!(!super::account_action_visible("findmy-off",Some(&state)));
        state["icloud"]=json!({"status":"signed_in","account":{"enabled_dataclasses":["contacts"]},"find_my_enabled":false});
        assert!(!super::account_action_visible("cloud-login",Some(&state)));
        assert!(super::account_action_visible("cloud-logout",Some(&state)));
        assert!(!super::account_action_visible("findmy-off",Some(&state)));
        state["icloud"]["find_my_enabled"]=json!(true);
        assert!(super::account_action_visible("findmy-off",Some(&state)));
        state["icloud"]["account"]["enabled_dataclasses"]=json!([]);
        assert!(super::account_action_visible("cloud-login",Some(&state)));
        state["app_store"]=json!({"status":"conflict","account":null});
        assert!(!super::account_action_known("store-login",Some(&state)));
        assert!(!super::account_action_known("store-login",None));
    }
    #[test]
    fn selected_app_cannot_be_overridden_by_pasted_text() {
        let spec = OPERATIONS.iter().find(|s| s.id == "app-save").unwrap();
        assert!(
            super::parse_operation(
                spec,
                &super::function_app_text("fr.vinted", "App=com.other.app\n名称=A")
            )
            .is_err()
        );
        assert!(!super::function_example(spec).contains("App="));
        assert!(
            super::parse_operation(
                spec,
                &super::function_app_text("fr.vinted", "工作账号 A----备用环境")
            )
            .is_ok()
        );
    }
    #[gpui::test]
    fn successful_logout_hides_actions_and_opens_raw_result(cx: &mut TestAppContext) {
        let (view, cx, _) = setup(cx, false);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preview_function("logout-success", window, cx)
            });
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("function-result-summary").is_some());
        assert!(cx.debug_bounds("function-result-text").is_some());
        assert!(cx.debug_bounds("function-execute").is_none());
        assert!(cx.debug_bounds("logout-data-keep").is_none());
        assert_eq!(
            super::result_summary(
                &serde_json::json!({"method":"signOutAppleID","result":{"completed_domains":["icloud","identity","media"]}})
            ),
            (
                "已退出 Apple 账户".into(),
                "iCloud · Apple 身份 · App Store".into()
            )
        );
    }
    #[test]
    fn response_view_excludes_request_envelope_and_preserves_returned_fields() {
        let record = serde_json::json!({"operation_id":"op", "method":"queryName", "params":{"apple_id":"request"},
            "status":"completed", "result":{"apple_id":"response", "middle_name":"", "request":{"native":true}}});
        let shown: serde_json::Value =
            serde_json::from_str(&super::function_response_text(&record)).unwrap();
        assert_eq!(
            shown,
            serde_json::json!({"status":"completed", "result":record["result"]})
        );
        assert_eq!(record["params"]["apple_id"], "request");
    }

    #[gpui::test]
    fn query_click_starts_once_shows_loading_and_needs_no_confirmation(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        view.update(cx, |view, cx| {
            view.functions.open = true;
            view.functions.category = "Apple";
            cx.notify();
        });
        while peer.commands.try_recv().map(unscoped).is_ok() {}
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let bounds = cx.debug_bounds("function-authorization-list").unwrap();
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("function-query-loading").is_some());
        assert!(cx.debug_bounds("function-execute").is_none());
        view.update(cx, |view, cx| view.apply_function_event(FunctionEvent::State {
            task: 31, record: serde_json::json!({"operation_id":"query","method":"querySignInWithAppleApps","status":"running"})
        }, cx));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("function-query-loading").is_some());
        assert!(cx.debug_bounds("function-result-text").is_none());
        assert!(cx.debug_bounds("function-query-resume").is_none());
        let first = peer.commands.try_recv().map(unscoped).unwrap();
        assert!(
            matches!(first, rv_session::SessionCommand::Function(FunctionCommand::Invoke { id, text }) if id == "authorization-list" && text.is_empty())
        );
        cx.update(|window, cx| view.update(cx, |view, cx| view.execute_function(window, cx)));
        let mut remaining = Vec::new();
        while let Ok(command) = peer.commands.try_recv().map(unscoped) {
            remaining.push(command);
        }
        assert!(
            remaining
                .iter()
                .all(|command| !matches!(command, rv_session::SessionCommand::Function(_))),
            "{remaining:?}"
        );
        view.update(cx, |view, cx| view.apply_function_event(FunctionEvent::State {
            task: 31, record: serde_json::json!({"operation_id":"query","method":"querySignInWithAppleApps","status":"completed","result":{"apps":[]}})
        }, cx));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("function-query-loading").is_none());
        assert!(cx.debug_bounds("function-result-summary").is_none());
        assert!(cx.debug_bounds("function-result-text").is_some());
        cx.update(|window, cx| view.update(cx, |view, cx| view.close_function_editor(window, cx)));
        view.read_with(cx, |view, _| {
            assert!(view.functions.open);
            assert!(view.functions.selected.is_some());
            assert!(view.functions.editor_hidden);
            assert!(view.functions.result.is_some());
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("function-authorization-list").is_some());
        assert!(cx.debug_bounds("function-editor").is_none());
        assert!(cx.debug_bounds("function-execute").is_none());
    }

    #[gpui::test]
    fn successful_write_cannot_be_submitted_again_from_same_editor(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        while peer.commands.try_recv().map(unscoped).is_ok() {}
        cx.update(|window, cx| view.update(cx, |view, cx| {
            let spec = OPERATIONS.iter().find(|spec| spec.id == "device-remove").unwrap();
            view.select_function(spec, window, cx);
            view.apply_function_event(FunctionEvent::State {task:31, record:serde_json::json!({
                "operation_id":"removed","method":"removeOtherICloudDevices","status":"completed","result":{"outcome":"succeeded"}
            })}, cx);
            view.execute_function(window, cx);
        }));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("function-execute").is_none());
        assert!(peer.commands.try_recv().map(unscoped).is_err());
    }
    #[gpui::test]
    fn submitted_text_is_retained_readonly_without_duplicate_actions(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        while peer.commands.try_recv().map(unscoped).is_ok() {}
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.select_function(&OPERATIONS[0], window, cx);
                view.functions.text.update(cx, |input, cx| {
                    input.set_value("demo@example.test----test-password", window, cx)
                });
                view.execute_function(window, cx);
                assert_eq!(
                    view.functions.text.read(cx).value(),
                    "demo@example.test----test-password"
                );
                assert!(view.functions.busy);
                view.execute_function(window, cx);
            })
        });
        assert!(
            matches!(peer.commands.try_recv().map(unscoped),Ok(rv_session::SessionCommand::Function(FunctionCommand::Invoke{id,text})) if id=="store-login" && text=="demo@example.test----test-password")
        );
        assert!(peer.commands.try_recv().map(unscoped).is_err());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("function-input-area").is_some());
        assert!(cx.debug_bounds("function-query-loading").is_some());
        assert!(cx.debug_bounds("function-execute").is_none());
        cx.simulate_input("must-not-change");
        view.read_with(cx, |view, cx| {
            assert_eq!(
                view.functions.text.read(cx).value(),
                "demo@example.test----test-password"
            )
        });
    }
    #[gpui::test]
    fn cloud_logout_has_no_text_and_requires_explicit_policy_and_execute(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        while peer.commands.try_recv().map(unscoped).is_ok() {}
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let spec = OPERATIONS.iter().find(|s| s.id == "cloud-logout").unwrap();
                view.select_function(spec, window, cx);
                assert!(!view.function_has_text(spec));
                assert!(view.functions.local_data_policy.is_none());
                view.execute_function(window, cx);
            });
            window.draw(cx).clear(cx);
        });
        assert!(peer.commands.try_recv().map(unscoped).is_err());
        assert!(cx.debug_bounds("logout-data-keep").is_some());
        let keep = cx.debug_bounds("logout-data-keep").unwrap().center();
        cx.simulate_click(keep, gpui::Modifiers::default());
        assert!(
            peer.commands.try_recv().map(unscoped).is_err(),
            "selecting policy must not log out"
        );
        cx.update(|window, cx| view.update(cx, |view, cx| view.execute_function(window, cx)));
        assert!(
            matches!(peer.commands.try_recv().map(unscoped), Ok(rv_session::SessionCommand::Function(FunctionCommand::Invoke { id, text })) if id == "cloud-logout" && text == "本机数据=keep")
        );
        assert!(peer.commands.try_recv().map(unscoped).is_err());
    }
    #[gpui::test]
    fn added_email_moves_to_code_input_without_account_or_challenge_text(cx: &mut TestAppContext) {
        let (view, cx, _) = setup(cx, false);
        enable(&view, cx);
        view.update(cx, |view, cx| {
            view.apply_function_event(FunctionEvent::State { task: 31, record: serde_json::json!({
                "operation_id":"add-email","method":"beginAddingContactEmail","status":"completed",
                "result":{"outcome":"requires_email_verification","challenge_id":"opaque"}
            }) }, cx);
            assert_eq!(view.functions.selected.unwrap().id, "email-verify");
            assert_eq!(view.functions.selected.unwrap().fields, &["code"]);
            assert!(!view.functions.busy);
            assert_eq!(
                view.functions.email_operation,
                Some((31, "add-email".into()))
            );
            view.functions.disconnected();
            assert_eq!(view.functions.operation, Some((31, "add-email".into())));
            view.apply_function_event(FunctionEvent::Available(true), cx);
            // Querying the original completed result after reconnect releases
            // the generic fence while restoring the runtime's email challenge.
            view.apply_function_event(FunctionEvent::State { task: 31, record: serde_json::json!({
                "operation_id":"add-email","method":"beginAddingContactEmail","status":"completed",
                "result":{"outcome":"requires_email_verification","challenge_id":"opaque"}
            }) }, cx);
            assert!(view.functions.operation.is_none());
            assert!(!view.functions.uncertain);
            assert_eq!(view.functions.selected.unwrap().fields, &["code"]);
            view.apply_function_event(
                FunctionEvent::State {
                    task: 31,
                    record: serde_json::json!({
                        "operation_id":"other-read","method":"queryName","status":"running"
                    }),
                },
                cx,
            );
            view.functions.disconnected();
            assert_eq!(
                view.functions.query_target(),
                Some((31, "other-read".into()))
            );
            view.apply_function_event(FunctionEvent::Available(true), cx);
            view.apply_function_event(FunctionEvent::State { task: 31, record: serde_json::json!({
                "operation_id":"other-read","method":"queryName","status":"completed","result":{}
            }) }, cx);
            assert_eq!(
                view.functions.query_target(),
                Some((31, "add-email".into()))
            );
            assert!(view.functions.email_needs_resume);
            view.apply_function_event(FunctionEvent::State { task: 31, record: serde_json::json!({
                "operation_id":"add-email","method":"beginAddingContactEmail","status":"completed",
                "result":{"outcome":"requires_email_verification","challenge_id":"opaque"}
            }) }, cx);
            assert!(view.functions.query_target().is_none());
            assert!(!view.functions.email_needs_resume);
        });
    }
    #[gpui::test]
    fn email_auto_fetch_progress_and_manual_fallback_remain_visible(cx: &mut TestAppContext) {
        let (view, cx, _) = setup(cx, false);
        enable(&view, cx);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let spec = OPERATIONS.iter().find(|s| s.id == "email-add").unwrap();
                view.select_function(spec, window, cx);
                view.functions.text.update(cx, |input, cx| input.set_value("demo@outlook.com----test-secret", window, cx));
                view.apply_function_event(FunctionEvent::EmailCodePending(serde_json::json!({
                    "operation_id":"add-email","method":"beginAddingContactEmail","status":"completed",
                    "result":{"outcome":"requires_email_verification","challenge_id":"opaque"}
                })), cx);
                assert!(view.functions.busy);
                assert_eq!(view.functions.selected.unwrap().title, "添加联系邮箱");
            });
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("function-operation-message").is_some());
        assert!(cx.debug_bounds("function-query-loading").is_some());
        assert!(cx.debug_bounds("function-input-area").is_some());
        assert!(cx.debug_bounds("function-execute").is_none());
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.apply_function_event(
                    FunctionEvent::Error("未收到验证码，可手动输入".into()),
                    cx,
                );
                assert!(!view.functions.busy);
                assert_eq!(view.functions.selected.unwrap().fields, &["code"]);
                assert_eq!(
                    view.functions.text.read(cx).value(),
                    "demo@outlook.com----test-secret"
                );
                assert!(view.functions.verification_text.read(cx).value().is_empty());
                view.functions
                    .verification_text
                    .update(cx, |input, cx| input.focus(window, cx));
            });
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("function-operation-message").is_some());
        assert!(cx.debug_bounds("function-execute").is_some());
        assert!(cx.debug_bounds("function-input-area").is_some());
        assert!(cx.debug_bounds("function-query-loading").is_none());
    }

    #[gpui::test]
    fn received_code_fills_input_and_survives_completion(cx: &mut TestAppContext) {
        let (view, cx, _) = setup(cx, false);
        enable(&view, cx);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let spec = OPERATIONS.iter().find(|s| s.id == "email-add").unwrap();
                view.select_function(spec, window, cx);
                view.functions.text.update(cx, |input, cx| input.set_value("demo@outlook.com----test-secret", window, cx));
                view.apply_function_event(FunctionEvent::EmailCodePending(serde_json::json!({
                    "operation_id":"add-email","method":"beginAddingContactEmail","status":"completed",
                    "result":{"outcome":"requires_email_verification","challenge_id":"opaque"}
                })), cx);
                view.apply_function_event(FunctionEvent::VerificationCodeReceived("012345".into()), cx);
            });
            window.draw(cx).clear(cx);
            view.update(cx, |view, cx| {
                assert_eq!(view.functions.verification_text.read(cx).value(), "012345");
                assert_eq!(view.functions.text.read(cx).value(), "demo@outlook.com----test-secret");
                assert!(view.functions.busy);
            });
        });
        assert!(cx.debug_bounds("function-received-code").is_some());
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.apply_function_event(FunctionEvent::State {task:31,record:serde_json::json!({
                    "operation_id":"verify-email","method":"verifyContactEmail","status":"completed",
                    "result":{"outcome":"succeeded"}
                })}, cx);
                assert_eq!(view.functions.verification_text.read(cx).value(), "012345");
                assert!(view.functions.result_open);
            });
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("function-received-code").is_some());
    }

    #[gpui::test]
    fn invalid_or_readonly_operation_does_not_send(cx: &mut TestAppContext) {
        for readonly in [false, true] {
            let (view, cx, mut peer) = setup(cx, readonly);
            enable(&view, cx);
            while peer.commands.try_recv().map(unscoped).is_ok() {}
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.select_function(&OPERATIONS[0], window, cx);
                    view.functions.text.update(cx, |input, cx| {
                        input.set_value("not-enough-fields", window, cx)
                    });
                    view.execute_function(window, cx);
                })
            });
            assert!(peer.commands.try_recv().map(unscoped).is_err());
        }
    }
    #[gpui::test]
    fn background_panel_disconnect_releases_busy_and_preserves_inputs(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        while peer.commands.try_recv().is_ok() {}
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let login = OPERATIONS.iter().find(|s| s.id == "store-login").unwrap();
                let name = OPERATIONS.iter().find(|s| s.id == "name-query").unwrap();
                view.select_function(login, window, cx);
                view.functions.text.update(cx, |input, cx| {
                    input.set_value("demo@example.com----secret", window, cx)
                });
                view.apply_function_event(
                    FunctionEvent::State {
                        task: 31,
                        record: serde_json::json!({"operation_id":"login-op","status":"running"}),
                    },
                    cx,
                );
                view.select_function(name, window, cx);
                assert_eq!(view.functions.selected.unwrap().id, "name-query");
                view.functions.disconnected();
                view.apply_function_event(FunctionEvent::Available(true), cx);
                view.select_function(login, window, cx);
                assert!(!view.functions.busy);
                assert!(view.functions.challenge.is_none());
                assert_eq!(view.functions.operation.as_ref().unwrap().1, "login-op");
                assert_eq!(
                    view.functions.text.read(cx).value(),
                    "demo@example.com----secret"
                );
                view.open_app_environment(None, window, cx);
                assert!(view.functions.environment.open);
                assert!(!view.environment_locked());
            })
        });
        assert!(std::iter::from_fn(||peer.commands.try_recv().map(unscoped).ok()).any(|cmd|matches!(cmd,rv_session::SessionCommand::Function(FunctionCommand::Invoke{id,..}) if id=="name-query")));
    }
    #[gpui::test]
    fn function_categories_and_popup_fit_small_window(cx: &mut TestAppContext) {
        let (view, cx, _) = setup(cx, false);
        enable(&view, cx);
        cx.simulate_resize(size(px(640.), px(400.)));
        cx.update(|window, cx| {
            view.update(cx, |view, _cx| {
                view.functions.category = "Apple";
            });
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("function-forward").is_some());
        assert!(cx.debug_bounds("function-store-login").is_some());
        let bounds = cx.debug_bounds("function-panel").unwrap();
        assert!(bounds.right() <= px(640.) && bounds.bottom() <= px(400.));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.select_function(&OPERATIONS[0], window, cx)
            });
            window.draw(cx).clear(cx);
        });
        let bounds = cx.debug_bounds("function-execute").unwrap();
        assert!(
            bounds.bottom() <= px(400.),
            "execute button must remain visible: {bounds:?}"
        );
        assert!(cx.debug_bounds("function-forward").is_none());
    }
    #[gpui::test]
    fn menu_and_editor_match_design_without_covering_phone(cx: &mut TestAppContext) {
        let (view, cx, _) = setup(cx, false);
        cx.simulate_resize(size(px(1100.), px(760.)));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.assistant_open = true;
                view.toggle_functions(window, cx);
                assert!(!view.assistant_open);
                view.apply_function_event(FunctionEvent::Available(true), cx);
                view.select_function(&OPERATIONS[0], window, cx);
            });
            window.draw(cx).clear(cx);
        });
        let menu = cx.debug_bounds("function-browser").unwrap();
        let editor = cx.debug_bounds("function-editor").unwrap();
        let execute = cx.debug_bounds("function-execute").unwrap();
        assert!(editor.right() < menu.left());
        assert!(menu.right() <= px(1100.) && menu.bottom() <= px(760.));
        assert!(execute.bottom() <= editor.bottom());
        let phone = view.read_with(cx, |view, _| view.image_box.get());
        assert!(phone.size.width > px(0.) && phone.right() < editor.left());
        assert!(cx.debug_bounds("function-store-login").is_some());
    }
    #[gpui::test]
    fn function_text_stays_local_to_client(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        while peer.commands.try_recv().map(unscoped).is_ok() {}
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.select_function(&OPERATIONS[0], window, cx)
            })
        });
        cx.simulate_input("private-text");
        cx.simulate_keystrokes("ctrl-a");
        assert!(peer.commands.try_recv().map(unscoped).is_err());
    }
}
