use super::*;
use crate::assets::FunctionIcon;
use gpui_component::{
    WindowExt,
    alert::Alert,
    input::{Editor, EditorState, Textarea, TextareaState},
    notification::Notification,
};
use rv_session::{
    FunctionCommand, FunctionEvent,
    functions::{OPERATIONS, OperationSpec, parse_operation},
};
#[path = "app_environments.rs"]
mod environments;
use environments::EnvironmentUi;
#[path = "apple_ui.rs"]
mod apple_ui;
#[path = "registration_ui.rs"]
mod registration_ui;

#[derive(Clone)]
struct FunctionPanel {
    editor_hidden: bool,
    email_rebinding: bool,
    email_rebind_operation: Option<String>,
    busy: bool,
    uncertain: bool,
    failed: bool,
    target_app: Option<(String, String)>,
    expected_account: Option<String>,
    text: Entity<TextareaState>,
    cloud_password: Entity<InputState>,
    cloud_preparation: Option<u64>,
    verification_text: Entity<InputState>,
    _verification_subscription: Rc<Subscription>,
    verification_submitted: bool,
    verification_received: bool,
    pending_verification_code: Option<String>,
    selected: Option<&'static OperationSpec>,
    message: SharedString,
    result: Option<serde_json::Value>,
    result_editor: Entity<EditorState>,
    result_open: bool,
    operation: Option<(u8, String)>,
    email_operation: Option<(u8, String)>,
    email_needs_resume: bool,
    challenge: Option<String>,
    local_data_policy: Option<&'static str>,
}
impl FunctionPanel {
    /// Completed history is display-only; a new menu click starts a fresh operation.
    /// Live receipts and challenges remain resumable. Ended attempts retain their final state until the user leaves.
    fn resumable(&self) -> bool {
        self.busy
            || self.uncertain
            || self.cloud_preparation.is_some()
            || self.operation.is_some()
            || self.challenge.is_some()
            || self.email_operation.is_some()
    }
    fn finished(&self) -> bool {
        let terminal = self
            .result
            .as_ref()
            .is_some_and(|record| record["status"] == "completed");
        if self.selected.is_some_and(|spec| spec.task == 32)
            && !self.busy
            && (terminal || self.uncertain)
        {
            return true;
        }
        if self.selected.is_some_and(is_login_action) && terminal {
            return !self.busy && self.cloud_preparation.is_none() && self.challenge.is_none();
        }
        !self.resumable() && terminal
    }
    fn disconnected(&mut self) {
        if self.cloud_preparation.take().is_some() {
            self.busy = false;
            self.message = "登录准备已中断，请重新点击登录".into();
        }
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
    next_cloud_preparation: u64,
    pending_results: Vec<(&'static OperationSpec, serde_json::Value)>,
    notified_results: std::collections::HashSet<String>,
    notification_close_task: Option<Task<()>>,
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
    failed: bool,
    target_app: Option<(String, String)>,
    expected_account: Option<String>,
    environment: EnvironmentUi,
    registration: registration_ui::RegistrationUi,
    text: Entity<TextareaState>,
    cloud_password: Entity<InputState>,
    cloud_preparation: Option<u64>,
    verification_text: Entity<InputState>,
    _verification_subscription: Rc<Subscription>,
    verification_submitted: bool,
    verification_received: bool,
    pending_verification_code: Option<String>,
    category: &'static str,
    pub(super) selected: Option<&'static OperationSpec>,
    message: SharedString,
    result: Option<serde_json::Value>,
    result_editor: Entity<EditorState>,
    result_open: bool,
    operation: Option<(u8, String)>,
    email_operation: Option<(u8, String)>,
    email_needs_resume: bool,
    challenge: Option<String>,
    local_data_policy: Option<&'static str>,
}
impl FunctionUi {
    pub(in crate::session_window) fn app_state_unsettled(&self) -> bool {
        let active = self.selected.is_some_and(|spec| spec.task == 32) && self.busy;
        active
            || self
                .panels
                .values()
                .any(|panel| panel.selected.is_some_and(|spec| spec.task == 32) && panel.busy)
    }
    fn panel_key(&self) -> Option<&'static str> {
        self.selected.map(|s| {
            if s.id == "email-verify" {
                "email-add"
            } else {
                s.id
            }
        })
    }
    fn has_pending(&self, spec: &OperationSpec) -> bool {
        let key = if spec.id == "email-verify" {
            "email-add"
        } else {
            spec.id
        };
        if self.panel_key() == Some(key) {
            self.busy
                || self.uncertain
                || self.cloud_preparation.is_some()
                || self.operation.is_some()
                || self.challenge.is_some()
                || self.email_operation.is_some()
        } else {
            self.panels.get(key).is_some_and(FunctionPanel::resumable)
        }
    }

    fn snapshot(&self) -> FunctionPanel {
        FunctionPanel {
            editor_hidden: self.editor_hidden.clone(),
            email_rebinding: self.email_rebinding.clone(),
            email_rebind_operation: self.email_rebind_operation.clone(),
            busy: self.busy.clone(),
            uncertain: self.uncertain.clone(),
            failed: self.failed,
            target_app: self.target_app.clone(),
            expected_account: self.expected_account.clone(),
            text: self.text.clone(),
            cloud_password: self.cloud_password.clone(),
            cloud_preparation: self.cloud_preparation,
            verification_text: self.verification_text.clone(),
            _verification_subscription: self._verification_subscription.clone(),
            verification_submitted: self.verification_submitted.clone(),
            verification_received: self.verification_received.clone(),
            pending_verification_code: self.pending_verification_code.clone(),
            selected: self.selected.clone(),
            message: self.message.clone(),
            result: self.result.clone(),
            result_editor: self.result_editor.clone(),
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
        self.failed = p.failed;
        self.target_app = p.target_app;
        self.expected_account = p.expected_account;
        self.text = p.text;
        self.cloud_password = p.cloud_password;
        self.cloud_preparation = p.cloud_preparation;
        self.verification_text = p.verification_text;
        self._verification_subscription = p._verification_subscription;
        self.verification_submitted = p.verification_submitted;
        self.verification_received = p.verification_received;
        self.pending_verification_code = p.pending_verification_code;
        self.selected = p.selected;
        self.message = p.message;
        self.result = p.result;
        self.result_editor = p.result_editor;
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
                .placeholder("粘贴资料")
        });
        let cloud_password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("当前 App Store 账户的登录密码")
        });
        let verification_text = cx.new(|cx| InputState::new(window, cx).placeholder("六位验证码"));
        let verification_subscription =
            Rc::new(cx.observe(&verification_text, |_, _, cx| cx.notify()));
        let result_editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("json")
                .soft_wrap(false)
        });
        Self {
            panels: Default::default(),
            next_cloud_preparation: 0,
            pending_results: Vec::new(),
            notified_results: Default::default(),
            notification_close_task: None,
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
            failed: false,
            target_app: None,
            expected_account: None,
            environment: EnvironmentUi::new(environment_apps),
            registration: registration_ui::RegistrationUi::new(window, cx),
            text,
            cloud_password,
            cloud_preparation: None,
            verification_text,
            _verification_subscription: verification_subscription,
            verification_submitted: false,
            verification_received: false,
            pending_verification_code: None,
            category: "Apple",
            selected: None,
            message: "".into(),
            result: None,
            result_editor,
            result_open: false,
            operation: None,
            email_operation: None,
            email_needs_resume: false,
            challenge: None,
            local_data_policy: None,
        }
    }
    fn stash_panel(&mut self) {
        if self.cloud_preparation.take().is_some() {
            self.busy = false;
            self.message = "登录准备已中断，请重新点击登录".into();
        }
        if let Some(key) = self.panel_key() {
            self.panels.insert(key, self.snapshot());
        }
    }
    pub(super) fn cancel_cloud_preparations(&mut self) {
        if self.cloud_preparation.take().is_some() {
            self.busy = false;
            self.message = "登录准备已中断，请重新点击登录".into();
        }
        for panel in self.panels.values_mut() {
            if panel.cloud_preparation.take().is_some() {
                panel.busy = false;
                panel.message = "登录准备已中断，请重新点击登录".into();
            }
        }
    }

    pub fn disconnected(&mut self) {
        self.available = false;
        self.account_state = None;
        self.account_loading = false;
        self.account_error = None;
        self.environment.mark_disconnected();
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

use crate::ui::loading as function_loading;

impl SessionView {
    #[cfg(test)]
    pub(super) fn preview_function(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.functions.open = true;
        let preview = id;
        let id = match id {
            "authorization-result" => "authorization-list",
            "devices-result" => "device-list",
            "login-challenge" => "store-login",
            "unknown-operation" => "name",
            "email-unknown" => "email-add",
            "authorization-confirmation" => "authorization-stop",
            "menu-signed-out" | "menu-store-only" => "",
            _ => id,
        };
        let query_success = id == "query-result";
        let id = if query_success { "name-query" } else { id };
        let logout_success = id == "logout-success";
        let id = if logout_success { "apple-logout" } else { id };
        self.apply_function_event(FunctionEvent::Available(true), cx);
        let section = |signed: bool| {
            if signed {
                serde_json::json!({"status":"signed_in","account":{"apple_id":"demo@example.com","authenticated":true,"enabled_dataclasses":["contacts"]},"find_my_enabled":true})
            } else {
                serde_json::json!({"status":"not_signed_in","account":null})
            }
        };
        let signed_out = matches!(
            preview,
            "store-login" | "login-challenge" | "menu-signed-out"
        );
        self.apply_function_event(FunctionEvent::AccountState(Ok(serde_json::json!({
            "app_store":section(!signed_out),
            "icloud":section(!signed_out && id != "cloud-login" && preview != "menu-store-only"),
            "identity":section(!signed_out && id != "cloud-login" && preview != "menu-store-only")
        }))), cx);
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
            if preview == "authorization-result" {
                self.apply_function_event(FunctionEvent::State {task:31, record:serde_json::json!({
                    "operation_id":"preview-apps", "method":"querySignInWithAppleApps", "status":"completed",
                    "result":{"apple_id":"demo@example.com","apps":[
                        {"name":"Vinted","bundle_id":"fr.vinted","client_id":"fr.vinted.app"},
                        {"name":"Example App","bundle_id":"test.example","client_id":"test.example.app"}]}
                })}, cx);
            }
            if preview == "devices-result" {
                self.apply_function_event(FunctionEvent::State {task:31, record:serde_json::json!({
                    "operation_id":"preview-devices", "method":"queryICloudDevices", "status":"completed",
                    "result":{"apple_id":"demo@example.com","devices":[
                        {"name":"我的 iPhone","model":"iPhone 15","current_device":true},
                        {"name":"旧 iPhone","model":"iPhone 12","current_device":false}]}
                })}, cx);
            }
            if preview == "login-challenge" {
                self.apply_function_event(FunctionEvent::State {task:31, record:serde_json::json!({
                    "operation_id":"preview-login", "method":"signInAppStore", "status":"requires_verification_code",
                    "challenge":{"challenge_id":"preview-challenge"}
                })}, cx);
                self.apply_function_event(
                    FunctionEvent::CodeFetchWaiting {
                        remaining_seconds: 0,
                    },
                    cx,
                );
            }
            if preview == "authorization-confirmation" {
                self.functions
                    .text
                    .update(cx, |input, cx| input.set_value("App=fr.vinted", window, cx));
            }
            if preview == "email-unknown" {
                self.apply_function_event(FunctionEvent::State {task:31, record:serde_json::json!({
                    "operation_id":"preview-email", "method":"beginAddingContactEmail", "status":"running"
                })}, cx);
                self.apply_function_event(
                    FunctionEvent::Uncertain("连接中断，结果待确认".into()),
                    cx,
                );
            }
            if preview == "unknown-operation" {
                self.apply_function_event(
                    FunctionEvent::State {
                        task: 31,
                        record: serde_json::json!({
                            "operation_id":"preview-unknown", "method":"setName", "status":"running"
                        }),
                    },
                    cx,
                );
                self.apply_function_event(
                    FunctionEvent::Uncertain("连接中断，结果待确认".into()),
                    cx,
                );
            }
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
    #[cfg(test)]
    pub(super) fn preview_function_confirmation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_apple_operation(window, cx);
    }
    pub(super) fn apply_function_event(&mut self, event: FunctionEvent, cx: &mut Context<Self>) {
        if self.receive_home_delete(&event, cx) {
            return;
        }
        match event {
            FunctionEvent::Registration(status) => {
                self.functions.registration.busy = status.running;
                self.functions.registration.message = status.message.into();
            }
            FunctionEvent::AccountStateLoading => {
                self.functions.account_loading = true;
                self.functions.account_state = None;
            }
            FunctionEvent::AccountState(state) => {
                self.functions.account_loading = false;
                match state {
                    Ok(state) => {
                        self.functions.account_state = Some(state);
                        self.functions.account_error = None;
                    }
                    Err(error) => {
                        self.functions.account_state = None;
                        self.functions.account_error = Some(error);
                    }
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
                if available {
                    self.functions.account_loading = true;
                } else {
                    self.functions.cancel_cloud_preparations();
                }
            }
            FunctionEvent::EmailCodePending(record) => {
                self.apply_function_event(FunctionEvent::State { task: 31, record }, cx);
                self.functions.busy = true;
                self.functions.message = "正在读取邮箱验证码…".into();
            }
            FunctionEvent::CodeFetchWaiting { remaining_seconds } => {
                self.functions.busy = false;
                self.functions.message = if remaining_seconds == 0 {
                    "请手动输入验证码".into()
                } else {
                    format!("自动获取中 · 剩余 {remaining_seconds} 秒").into()
                };
            }
            FunctionEvent::CodeFetched(code) => {
                self.functions.busy = false;
                if rv_session::functions::valid_code(&code) {
                    self.functions.pending_verification_code = Some(code);
                    self.functions.verification_received = true;
                    self.functions.message = "已获取验证码".into();
                }
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
                    self.functions.verification_submitted = true;
                    self.functions.pending_verification_code = Some(code);
                    self.functions.verification_received = true;
                    self.functions.message = "已收到验证码，正在验证…".into();
                }
            }
            FunctionEvent::Progress(message) => {
                self.functions.failed = false;
                self.functions.busy = true;
                self.functions.message = message.into();
            }
            FunctionEvent::Uncertain(message) => {
                self.functions.failed = false;
                self.environment_run_error(&message, true);
                if self.functions.environment.open
                    && self
                        .functions
                        .selected
                        .is_some_and(|spec| spec.id == "app-status")
                {
                    self.functions.environment.mark_status_failed();
                }
                self.functions.uncertain = true;
                self.functions.busy = false;
                self.functions.message = message.into();
            }
            FunctionEvent::Error(message) => {
                self.functions.failed = true;
                self.environment_run_error(&message, false);
                if self.functions.environment.open
                    && self
                        .functions
                        .selected
                        .is_some_and(|spec| spec.id == "app-status")
                {
                    self.functions.environment.mark_status_failed();
                }
                // Keep operation/challenge for query or manual verification; no replay.
                self.functions.busy = false;
                self.functions.message = message.into();
            }
            FunctionEvent::State { task, record } => {
                if task == 32 {
                    self.update_environment_run(&record);
                }
                let environment_completed = task == 32
                    && record["status"] == "completed"
                    && self.functions.environment.open;
                let Some(id) = record["operation_id"].as_str() else {
                    return;
                };
                self.functions.operation = Some((task, id.into()));
                self.functions.challenge = record["challenge"]["challenge_id"]
                    .as_str()
                    .filter(|challenge| !challenge.is_empty() && challenge.len() <= 128)
                    .map(str::to_owned);
                self.functions.result = Some(record.clone());
                self.functions.failed =
                    record["status"] == "completed" && record.get("error").is_some();
                self.functions.message = match record["status"].as_str() {
                    Some("requires_verification_code") => {
                        self.functions.busy = false;
                        // A queried native challenge confirms this original operation is awaiting input.
                        if self.functions.challenge.is_some() {
                            self.functions.uncertain = false;
                        }
                        "请输入六位验证码，或等待链接读取".into()
                    }
                    Some("completed") => {
                        self.functions.busy = false;
                        self.functions.result_open = false;
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
                            if record["error"]["message"]
                                .as_str()
                                .is_some_and(|message| !message.trim().is_empty())
                            {
                                format!("结果待确认：{}", function_error_message(&record)).into()
                            } else {
                                "结果待确认".into()
                            }
                        } else if record.get("error").is_some() {
                            function_error_message(&record).into()
                        } else if record["result"]["outcome"] == "requires_email_verification" {
                            self.functions.selected =
                                OPERATIONS.iter().find(|spec| spec.id == "email-verify");
                            "请输入邮箱收到的六位验证码".into()
                        } else {
                            "操作已完成".into()
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
                        if task == 32 && record["progress"].is_object() {
                            environments::environment_progress_label(&record["progress"]).into()
                        } else if self.functions.selected.is_some_and(is_query) {
                            "正在读取…".into()
                        } else if self
                            .functions
                            .selected
                            .is_some_and(|spec| spec.id == "cloud-login")
                        {
                            "等待手机确认 iCloud 登录结果…".into()
                        } else {
                            "正在执行…".into()
                        }
                    }
                };
                if record["status"] == "completed"
                    && let Some(spec) = self.functions.selected.filter(|spec| is_login_action(spec))
                    && self.functions.notified_results.insert(id.to_owned())
                {
                    self.functions.pending_results.push((spec, record.clone()));
                }
                if environment_completed {
                    self.finish_environment_operation(&record, cx);
                }
            }
        }
        cx.notify();
    }
    fn send_function_command(&self, command: FunctionCommand) {
        let id = match &command {
            FunctionCommand::Invoke { id, .. } | FunctionCommand::InvokeForAccount { id, .. } => {
                id.clone()
            }
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
    fn verification_input_active(&self) -> bool {
        self.functions.verification_submitted
            || self
                .functions
                .selected
                .is_some_and(|spec| spec.id == "email-verify")
            || self.functions.challenge.is_some()
    }
    fn function_input_value(&self, cx: &App) -> SharedString {
        if self.verification_input_active() {
            self.functions.verification_text.read(cx).value()
        } else {
            self.functions.text.read(cx).value()
        }
    }
    fn focus_function_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.verification_input_active() {
            self.functions
                .verification_text
                .update(cx, |input, cx| input.focus(window, cx));
        } else {
            self.functions
                .text
                .update(cx, |input, cx| input.focus(window, cx));
        }
    }
    pub(super) fn sync_function_notifications(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (spec, record) in std::mem::take(&mut self.functions.pending_results) {
            // Notify owns an immutable historical result, independent of subsequent requests.
            {
                let unknown = rv_session::functions::uncertain_result(31, &record);
                let mut note = if unknown {
                    Notification::info(
                        if record["error"]["message"]
                            .as_str()
                            .is_some_and(|message| !message.trim().is_empty())
                        {
                            format!(
                                "{}：结果待确认，{}",
                                spec.title,
                                function_error_message(&record)
                            )
                        } else {
                            format!("{}：结果待确认", spec.title)
                        },
                    )
                } else if record.get("error").is_some() {
                    Notification::error(format!(
                        "{}：{}",
                        spec.title,
                        function_error_message(&record)
                    ))
                } else {
                    Notification::success(result_summary(&record).0)
                };
                let response = function_response_text(&record);
                note = note.action(move |_, _, _| {
                    let response = response.clone();
                    Button::new("function-notification-details")
                        .debug_selector(|| "function-notification-details".into())
                        .ghost()
                        .icon(IconName::File)
                        .accessibility_label("原始结果")
                        .on_click(move |_, window, cx| {
                            let editor = cx.new(|cx| {
                                EditorState::new(window, cx)
                                    .language("json")
                                    .soft_wrap(false)
                                    .default_value(response.clone())
                            });
                            let copy = response.clone();
                            window.open_dialog(cx, move |dialog, _, _| {
                                let copy = copy.clone();
                                dialog
                                    .title(spec.title)
                                    .child(
                                        div()
                                            .debug_selector(|| {
                                                "function-notification-result".into()
                                            })
                                            .child(Editor::new(&editor).readonly(true).h(px(320.))),
                                    )
                                    .footer(
                                        gpui_component::dialog::DialogFooter::new().child(
                                            Button::new("function-notification-copy")
                                                .ghost()
                                                .icon(IconName::Copy)
                                                .accessibility_label("复制原始结果")
                                                .on_click(move |_, _, cx| {
                                                    cx.write_to_clipboard(
                                                        ClipboardItem::new_string(copy.clone()),
                                                    );
                                                }),
                                        ),
                                    )
                            });
                        })
                });
                self.functions.notification_close_task = Some(crate::ui::notify(note, window, cx));
            }
        }
    }

    pub(super) fn sync_function_result_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.functions.open || self.functions.editor_hidden || self.functions.busy {
            return;
        }
        let Some(record) = self.functions.result.as_ref() else {
            return;
        };
        if record["status"] != "completed" {
            return;
        }
        let text = function_response_text(record);
        // Do not reset selection or scroll position on unrelated window redraws.
        if self.functions.result_editor.read(cx).value().as_ref() != text {
            self.functions.result_editor.update(cx, |editor, cx| {
                editor.set_value(text, window, cx);
            });
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
        self.functions
            .cloud_password
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
        if !self.functions.available
            || self.phase != Phase::Connected
            || self.functions.account_loading
        {
            return;
        }
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
        if !account_action_visible(spec.id, self.functions.account_state.as_ref())
            && !self.functions.has_pending(spec)
        {
            return;
        }
        if self.functions.selected.is_some_and(|selected| {
            selected.id == spec.id || (selected.id == "email-verify" && spec.id == "email-add")
        }) && !self.functions.snapshot().finished()
        {
            self.functions.editor_hidden = false;
            cx.notify();
            return;
        }
        if self.functions.cloud_preparation.take().is_some() {
            self.functions.busy = false;
            self.functions.message = "登录准备已中断，请重新点击登录".into();
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
        let previous = self
            .functions
            .panels
            .remove(key)
            .filter(|panel| !panel.finished());
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
        self.functions.text.update(cx, |input, cx| {
            input.set_placeholder(
                match spec.id {
                    "store-login" => "账号----密码----验证码链接（可选）",
                    "cloud-login" => "登录密码（使用当前 App Store 账户）",
                    _ => spec.hint,
                },
                window,
                cx,
            );
        });
        if spec.id == "cloud-login" {
            self.functions
                .cloud_password
                .update(cx, |input, cx| input.focus(window, cx));
        } else {
            self.focus_function_input(window, cx);
        }
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
        if self.functions.uncertain && spec.task != 32 && !is_query(spec) {
            return;
        }
        if !account_action_visible(spec.id, self.functions.account_state.as_ref()) {
            self.functions.message = "账户状态已改变，请重新选择操作".into();
            cx.notify();
            return;
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
        let text = if spec.id == "cloud-login" {
            format!("密码={}", self.functions.cloud_password.read(cx).value())
        } else {
            self.function_input_value(cx).to_string()
        };
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
        if spec.id == "cloud-login" {
            self.prepare_cloud_login(text, cx);
            return;
        }
        self.send_function_command(
            if let Some(apple_id) = self.functions.expected_account.clone() {
                FunctionCommand::InvokeForAccount {
                    id: spec.id.into(),
                    text,
                    apple_id,
                }
            } else {
                FunctionCommand::Invoke {
                    id: spec.id.into(),
                    text,
                }
            },
        );
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
    fn prepare_cloud_login(&mut self, text: String, cx: &mut Context<Self>) {
        let spec = OPERATIONS
            .iter()
            .find(|spec| spec.id == "cloud-login")
            .unwrap();
        let Ok(parsed) = parse_operation(spec, &text) else {
            return;
        };
        let req = self.req.clone();
        self.functions.next_cloud_preparation =
            self.functions.next_cloud_preparation.wrapping_add(1).max(1);
        let token = self.functions.next_cloud_preparation;
        self.functions.cloud_preparation = Some(token);
        self.functions.busy = true;
        self.functions.result = None;
        self.functions.result_open = false;
        self.functions.verification_submitted = false;
        self.functions.message = "正在读取本地锁屏配置…".into();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let code = cx
                .background_executor()
                .spawn(async move { rv_core::load_unlock_code(&req) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.finish_cloud_login_preparation(token, parsed, code, cx);
            });
        })
        .detach();
    }

    fn finish_cloud_login_preparation(
        &mut self,
        token: u64,
        mut parsed: rv_session::functions::PreparedOperation,
        code: Result<Option<rv_core::UnlockCode>, rv_core::StoreError>,
        cx: &mut Context<Self>,
    ) {
        // Every result branch belongs to one still-current local preparation.
        // Switching panels or disconnecting invalidates the token before a new request.
        if self.functions.cloud_preparation != Some(token) {
            return;
        }
        self.functions.cloud_preparation = None;
        if self.phase != Phase::Connected
            || !self.functions.available
            || self
                .functions
                .selected
                .is_none_or(|spec| spec.id != "cloud-login")
            || !self.functions.busy
            || self.view_only()
        {
            self.functions.busy = false;
            self.functions.message = "登录准备已中断，请重新点击登录".into();
            cx.notify();
            return;
        }
        let error = match code {
            Ok(Some(code)) => {
                parsed.params["device_passcode"] =
                    serde_json::json!(String::from_utf8(code.digits().to_vec()).unwrap());
                if let Some(url) = parsed.code_url.take() {
                    parsed.params["code_url"] = serde_json::json!(url);
                }
                let text = parsed
                    .params
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(key, value)| format!("{key}={}", value.as_str().unwrap()))
                    .collect::<Vec<_>>()
                    .join("\n");
                self.functions.message = "正在读取当前账户并登录…".into();
                self.send_function_command(FunctionCommand::Invoke {
                    id: "cloud-login".into(),
                    text,
                });
                None
            }
            Ok(None) => {
                self.prompt_unlock_setting(cx);
                Some("请先设置本地锁屏密码，再点击登录".to_owned())
            }
            Err(_) => Some("读取本地锁屏密码失败，请检查锁屏设置".to_owned()),
        };
        if let Some(error) = error {
            self.apply_function_event(FunctionEvent::Error(error), cx);
        }
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
        let text = self.function_input_value(cx).to_string();
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
        self.functions.message = "正在提交验证码…".into();
        cx.notify();
    }
    pub(super) fn render_function_button(
        &self,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Button::new("function-menu-toggle")
            .debug_selector(|| "function-menu-toggle".into())
            .accessibility_label("功能")
            .ghost()
            .icon(IconName::LayoutDashboard)
            .when(!compact, |button| button.label("功能"))
            .when(compact, |button| button)
            .selected(self.functions.open)
            .on_click(cx.listener(|this, _, window, cx| this.toggle_functions(window, cx)))
    }
    fn toggle_functions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.functions.environment.open && !self.environment_locked() {
            self.return_function_menu(window, cx);
            return;
        }
        self.functions.open = !self.functions.open;
        self.notifications.open = false;
        self.mailbox.open = false;
        self.show_menu = false;
        self.info_open = false;
        let keys = self.keys.release_all();
        self.send_keys(keys);
        if self.functions.open {
            self.refresh_function_accounts(cx);
            self.assistant_open = false;
            if self.functions.selected.is_some() && !self.functions.editor_hidden {
                if self
                    .functions
                    .selected
                    .is_some_and(|spec| spec.id == "cloud-login")
                    && self.functions.challenge.is_none()
                {
                    self.functions
                        .cloud_password
                        .update(cx, |input, cx| input.focus(window, cx));
                } else {
                    self.focus_function_input(window, cx);
                }
            } else {
                self.focus.focus(window, cx);
            }
        } else {
            if self.functions.environment.open && !self.functions.busy {
                self.clear_function_text(window, cx);
            }
            self.focus.focus(window, cx);
        }
        cx.notify();
    }
    pub(super) fn function_uses_two_panels(&self) -> bool {
        self.functions.environment.open && self.function_editor_visible()
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
        if self.functions.category == "功能" {
            return self.render_registration_panel(width, height, cx);
        }
        let selected = self
            .functions
            .selected
            .filter(|_| !self.functions.editor_hidden);
        // Apple operations use one stable panel at every window size.
        let side_by_side = false;
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
        for spec in OPERATIONS.iter().filter(|spec| {
            spec.category == self.functions.category
                && spec.id != "email-verify"
                && self.function_menu_visible(spec)
        }) {
            let reason = self.function_unavailable_reason(spec);
            let (group, icon) = function_presentation(spec);
            if group != previous_group {
                if !previous_group.is_empty() {
                    rows.push(
                        div()
                            .my_1()
                            .border_t_1()
                            .border_color(theme::line(cx))
                            .into_any_element(),
                    );
                }
                previous_group = group;
            }
            let active = selected.is_some_and(|s| {
                s.id == spec.id || (s.id == "email-verify" && spec.id == "email-add")
            });
            rows.push(
                Button::new(spec.id)
                    .ghost()
                    .selected(active)
                    .w_full()
                    .accessibility_label(spec.title)
                    .debug_selector(move || format!("function-{}", spec.id))
                    .disabled(reason.is_some())
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .debug_selector(move || format!("function-menu-content-{}", spec.id))
                            .child(Icon::new(icon))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_left()
                                    .text_sm()
                                    .child(spec.title),
                            )
                            .child(Icon::new(IconName::ChevronRight)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_function(spec, window, cx)
                    }))
                    .into_any_element(),
            );
        }
        crate::ui::tool_panel(cx)
            .id("function-browser")
            .debug_selector(|| "function-browser".into())
            .w(px(360f32.min(width - 16.)))
            .max_h(px((height - 16.).max(100.)))
            .flex_shrink_0()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(self.render_function_navigation(cx))
            .when_some(self.functions.account_error.clone(), |panel, error| {
                panel.child(
                    div()
                        .debug_selector(|| "function-account-error".into())
                        .text_xs()
                        .text_color(theme::muted(cx))
                        .child(format!("账户读取失败：{error}")),
                )
            })
            .child(
                h_flex().min_h_0().child(
                    v_flex()
                        .id("function-list")
                        .debug_selector(|| "function-list".into())
                        .flex_1()
                        .min_w_0()
                        .max_h(px(400f32.min((height - 100.).max(60.))))
                        .overflow_y_scroll()
                        .children(rows)
                        .when(previous_group.is_empty(), |list| {
                            list.child(
                                div()
                                    .p_2()
                                    .text_sm()
                                    .text_color(theme::muted(cx))
                                    .child("暂无功能"),
                            )
                        }),
                ),
            )
    }
    fn render_function_navigation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let environment = self.functions.environment.open;
        let editor = self.functions.selected.filter(|spec| {
            spec.task == 31
                && !self.functions.editor_hidden
                && !environment
                && self.functions.category != "功能"
        });
        h_flex()
            .debug_selector(|| "function-navigation-bar".into())
            .gap_1()
            .items_center()
            .child(
                gpui_component::tab::TabBar::new("function-navigation")
                    .underline()
                    .flex_shrink_0()
                    .selected_index(if environment {
                        1
                    } else if self.functions.category == "功能" {
                        2
                    } else {
                        0
                    })
                    .children(
                        [
                            ("Apple", FunctionIcon::User),
                            ("App 环境", FunctionIcon::All),
                            ("功能", FunctionIcon::All),
                        ]
                        .into_iter()
                        .map(|(category, icon)| {
                            gpui_component::tab::Tab::new().aria_label(category).child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(Icon::new(icon))
                                    .child(
                                        div()
                                            .id(format!("function-tab-{category}"))
                                            .debug_selector(move || {
                                                format!("function-tab-{category}")
                                            })
                                            .child(category),
                                    ),
                            )
                        }),
                    )
                    .on_click(cx.listener(|this, index: &usize, window, cx| {
                        if *index == 2 {
                            this.functions.stash_panel();
                            this.functions.environment.open = false;
                            this.functions.category = "功能";
                            cx.notify();
                        } else if *index == 1 {
                            if !this.functions.environment.open {
                                this.open_app_environment(None, window, cx);
                            }
                        } else {
                            if this.functions.environment.open {
                                this.return_function_menu(window, cx);
                            }
                            this.functions.category = "Apple";
                            cx.notify();
                        }
                    })),
            )
            .child(div().flex_1())
            .when(
                !environment
                    && self.functions.category != "功能"
                    && (self.functions.selected.is_none() || self.functions.editor_hidden),
                |row| {
                    row.child(
                        Button::new("function-account-refresh")
                            .accessibility_label("刷新账户状态")
                            .debug_selector(|| "function-account-refresh".into())
                            .ghost()
                            .icon(FunctionIcon::Refresh)
                            .disabled(!self.functions.available || self.functions.account_loading)
                            .loading(self.functions.account_loading)
                            .text_color(theme::muted(cx))
                            .on_click(
                                cx.listener(|this, _, _, cx| this.refresh_function_accounts(cx)),
                            ),
                    )
                },
            )
            .when(editor.is_some_and(is_query), |row| {
                row.child(
                    crate::ui::icon_button(
                        "function-query-refresh",
                        FunctionIcon::Refresh,
                        "刷新资料",
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.execute_function(window, cx)))
                    .debug_selector(|| "function-query-refresh".into())
                    .disabled(!self.functions.available || self.functions.busy),
                )
            })
            .when(editor.is_some(), |row| {
                row.child(
                    Button::new("function-back")
                        .ghost()
                        .icon(IconName::ArrowLeft)
                        .accessibility_label("返回菜单")
                        .debug_selector(|| "function-back".into())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.close_function_editor(window, cx)
                        })),
                )
            })
            .child(
                Button::new("function-close")
                    .accessibility_label("关闭面板")
                    .debug_selector(|| "function-close".into())
                    .ghost()
                    .icon(Icon::new(IconName::Close))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.close_function_panel(window, cx);
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
        crate::ui::tool_panel(cx)
            .id("function-editor")
            .debug_selector(|| "function-editor".into())
            .w(px(360f32.min(width - 16.)))
            .max_h(px((height - if side_by_side { 56. } else { 16. }).max(100.)))
            .when(side_by_side, |panel| panel.mt(px(40.)))
            .overflow_y_scroll()
            .flex_shrink_0()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .when(selected.is_some_and(|spec| spec.task == 31), |panel| {
                panel.child(self.render_function_navigation(cx))
            })
            .when_some(selected, |panel, spec| {
                panel.child(
                    crate::ui::tool_section()
                        .child(
                            crate::ui::tool_actions()
                                .child(div().flex_1().min_w_0().text_sm().font_semibold().child(
                                    if spec.id == "apple-query" {
                                        "账户资料"
                                    } else if spec.id == "app-delete"
                                        && self.functions.environment.backup.as_ref().is_some_and(
                                            |backup| backup.id.starts_with("baseline:"),
                                        )
                                    {
                                        "删除基线"
                                    } else {
                                        spec.title
                                    },
                                ))
                                .when(query && spec.task != 31, |row| {
                                    row.child(
                                        crate::ui::icon_button(
                                            "function-query-refresh",
                                            FunctionIcon::Refresh,
                                            "刷新资料",
                                        )
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.execute_function(window, cx)
                                        }))
                                        .debug_selector(|| "function-query-refresh".into())
                                        .disabled(!live || locked),
                                    )
                                })
                                .when(
                                    self.functions.query_target().is_some()
                                        && (self.functions.uncertain
                                            || self.functions.email_needs_resume),
                                    |row| {
                                        row.child(
                                            Button::new("function-query")
                                                .debug_selector(|| "function-query-resume".into())
                                                .ghost()
                                                .icon(IconName::Redo)
                                                .loading(locked)
                                                .label("查询结果")
                                                .accessibility_label("查询原操作结果，不重新提交")
                                                .text_color(theme::muted(cx))
                                                .disabled(!live || locked)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    if let Some((task, operation_id)) =
                                                        this.functions.query_target()
                                                    {
                                                        this.send_function_command(
                                                            FunctionCommand::Resume {
                                                                task,
                                                                operation_id,
                                                            },
                                                        );
                                                        this.functions.busy = true;
                                                        this.functions.message =
                                                            "正在查询原操作结果…".into();
                                                        cx.notify();
                                                    }
                                                })),
                                        )
                                    },
                                )
                                .when(spec.task != 31, |row| {
                                    row.child(
                                        crate::ui::icon_button(
                                            "function-back",
                                            IconName::ArrowLeft,
                                            "返回",
                                        )
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.close_function_editor(window, cx);
                                        }))
                                        .debug_selector(|| "function-back".into()),
                                    )
                                })
                                .when(spec.task != 31, |row| {
                                    row.child(
                                        crate::ui::icon_button(
                                            "function-editor-close",
                                            IconName::Close,
                                            "关闭",
                                        )
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.close_function_panel(window, cx);
                                        }))
                                        .debug_selector(|| "function-editor-close".into())
                                        .icon(Icon::new(IconName::Close)),
                                    )
                                }),
                        )
                        .when(
                            spec.task == 31 && spec.id != "apple-query" && !operation_done,
                            |panel| panel.child(self.render_apple_operation_context(spec, cx)),
                        )
                        .when(
                            matches!(spec.id, "email-add" | "email-verify")
                                && self.functions.uncertain,
                            |row| {
                                row.child(
                                    Button::new("function-email-rebind")
                                        .ghost()
                                        .label("重新发送绑定")
                                        .debug_selector(|| "function-email-rebind".into())
                                        .text_color(theme::accent(cx))
                                        .disabled(!live || self.functions.busy)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if this.functions.busy {
                                                return;
                                            }
                                            let text =
                                                this.functions.text.read(cx).value().to_string();
                                            this.functions.email_rebinding = true;
                                            this.functions.email_rebind_operation = None;
                                            this.send_function_command(
                                                if let Some(apple_id) =
                                                    this.functions.expected_account.clone()
                                                {
                                                    FunctionCommand::InvokeForAccount {
                                                        id: "email-add".into(),
                                                        text,
                                                        apple_id,
                                                    }
                                                } else {
                                                    FunctionCommand::RebindEmail { text }
                                                },
                                            );
                                            this.functions.selected =
                                                OPERATIONS.iter().find(|s| s.id == "email-add");
                                            this.functions.busy = true;
                                            this.functions.result_open = false;
                                            this.functions.message =
                                                "正在核对邮箱并初始化新绑定…".into();
                                            cx.notify();
                                        })),
                                )
                            },
                        )
                        .when(self.functions.environment.open, |panel| {
                            panel.child(self.render_environment_context(cx))
                        })
                        .when(
                            !operation_done
                                && !self.functions.uncertain
                                && !self.functions.busy
                                && matches!(spec.id, "apple-logout" | "cloud-logout"),
                            |panel| {
                                panel.child(
                                    crate::ui::tool_actions()
                                        .justify_between()
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(theme::muted(cx))
                                                .child("本机数据"),
                                        )
                                        .child(
                                            gpui_component::radio::RadioGroup::horizontal(
                                                "logout-data-policy",
                                            )
                                            .disabled(!live || locked)
                                            .selected_index(
                                                match self.functions.local_data_policy {
                                                    Some("keep") => Some(0),
                                                    Some("remove") => Some(1),
                                                    _ => None,
                                                },
                                            )
                                            .children([("keep", "保留"), ("remove", "移除")].map(
                                                |(policy, label)| {
                                                    gpui_component::radio::Radio::new(format!(
                                                        "logout-data-{policy}"
                                                    ))
                                                    .label(label)
                                                    .debug_selector(move || {
                                                        format!("logout-data-{policy}")
                                                    })
                                                },
                                            ))
                                            .on_click(
                                                cx.listener(|this, index: &usize, _, cx| {
                                                    cx.stop_propagation();
                                                    this.functions.local_data_policy =
                                                        Some(if *index == 0 {
                                                            "keep"
                                                        } else {
                                                            "remove"
                                                        });
                                                    this.functions.message = "".into();
                                                    cx.notify();
                                                }),
                                            ),
                                        ),
                                )
                            },
                        )
                        .when(
                            !operation_done
                                && !self.functions.uncertain
                                && self.functions.challenge.is_none()
                                && !self.functions.verification_submitted
                                && self.function_has_text(spec),
                            |panel| {
                                if spec.id == "email-verify" {
                                    return panel.child(
                                        div()
                                            .w_full()
                                            .min_w_0()
                                            .debug_selector(|| "function-input-area".into())
                                            .child(
                                                Input::new(&self.functions.verification_text)
                                                    .readonly(self.functions.busy),
                                            ),
                                    );
                                }
                                if spec.id == "cloud-login"
                                    && self.functions.challenge.is_none()
                                    && !self.functions.verification_submitted
                                {
                                    return panel.child(
                                        div()
                                            .w_full()
                                            .min_w_0()
                                            .debug_selector(|| "function-input-area".into())
                                            .child(
                                                Input::new(&self.functions.cloud_password)
                                                    .readonly(self.functions.busy),
                                            ),
                                    );
                                }
                                panel.child(
                                    div()
                                        .w_full()
                                        .min_w_0()
                                        .debug_selector(|| "function-input-area".into())
                                        .child(
                                            Textarea::new(&self.functions.text)
                                                .readonly(self.functions.busy)
                                                .h(px(100.)),
                                        ),
                                )
                            },
                        )
                        .when(
                            self.functions.verification_received
                                && self.functions.challenge.is_none()
                                && (self.functions.busy || operation_done),
                            |panel| {
                                panel.child(
                                    h_flex()
                                        .debug_selector(|| "function-received-code".into())
                                        .gap_2()
                                        .items_center()
                                        .child(div().text_sm().child("验证码"))
                                        .child(
                                            Input::new(&self.functions.verification_text)
                                                .readonly(true)
                                                .w(px(112.)),
                                        ),
                                )
                            },
                        )
                        .when(
                            !operation_done
                                && !self.functions.uncertain
                                && !query
                                && !self.functions.busy
                                && self.functions.challenge.is_none()
                                && !self.functions.verification_submitted,
                            |panel| {
                                panel.child(
                                    crate::ui::tool_actions()
                                        .justify_end()
                                        .when(
                                            self.function_has_text(spec)
                                                && !locked
                                                && spec.id != "cloud-login",
                                            |row| {
                                                row.child(
                                                    Button::new("function-example")
                                                        .ghost()
                                                        .accessibility_label("复制示例")
                                                        .icon(IconName::Copy)
                                                        .text_color(theme::accent(cx))
                                                        .disabled(locked)
                                                        .on_click(cx.listener(
                                                            move |this, _, _, cx| {
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
                                                            },
                                                        )),
                                                )
                                            },
                                        )
                                        .when(spec.id == "email-verify", |row| {
                                            row.child(
                                                Button::new("function-email-refetch")
                                                    .ghost()
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
                                        .when(spec.task != 31, |row| {
                                            row.child(
                                                Button::new("function-editor-cancel")
                                                    .ghost()
                                                    .disabled(locked)
                                                    .label("取消")
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            this.functions.stash_panel();
                                                            this.functions.editor_hidden = true;
                                                            this.functions.environment.backup =
                                                                None;
                                                            this.focus.focus(window, cx);
                                                            cx.notify();
                                                        },
                                                    )),
                                            )
                                        })
                                        .when(self.functions.challenge.is_none(), |row| {
                                            row.child(
                                                Button::new("function-execute")
                                                    .loading(self.functions.busy)
                                                    .debug_selector(|| "function-execute".into())
                                                    .primary()
                                                    .label(
                                                        if matches!(
                                                            spec.id,
                                                            "store-login" | "cloud-login"
                                                        ) {
                                                            "登录"
                                                        } else {
                                                            match spec.id {
                                                                "app-save" => "保存",
                                                                "app-switch" => "切换",
                                                                "app-delete" => "确认删除",
                                                                "app-edit" => "保存修改",
                                                                "store-logout" => "退出 App Store",
                                                                "cloud-logout" => "退出 iCloud",
                                                                "apple-logout" => "退出账户",
                                                                "device-remove" => {
                                                                    "确认移除其它设备"
                                                                }
                                                                "name" => "保存姓名",
                                                                "forward" => "设置转发邮箱",
                                                                "authorization-stop" => "停止授权",
                                                                "email-add" => "添加并验证",
                                                                "email-verify" => "验证",
                                                                _ => "执行",
                                                            }
                                                        },
                                                    )
                                                    .primary()
                                                    .disabled(
                                                        !live
                                                            || locked
                                                            || (spec.id == "email-verify"
                                                                && self
                                                                    .functions
                                                                    .email_needs_resume)
                                                            || (matches!(
                                                                spec.id,
                                                                "apple-logout" | "cloud-logout"
                                                            ) && self
                                                                .functions
                                                                .local_data_policy
                                                                .is_none())
                                                            || (needs_app(spec)
                                                                && self
                                                                    .functions
                                                                    .target_app
                                                                    .is_none()),
                                                    )
                                                    .on_click(cx.listener(
                                                        |this, _, window, cx| {
                                                            this.confirm_apple_operation(window, cx)
                                                        },
                                                    )),
                                            )
                                        }),
                                )
                            },
                        ),
                )
            })
            .when(
                !operation_done
                    && !self.functions.uncertain
                    && (self.functions.challenge.is_some()
                        || self.functions.verification_submitted),
                |panel| {
                    panel.child(
                        crate::ui::tool_actions()
                            .debug_selector(|| "function-verification-row".into())
                            .child(
                                Input::new(&self.functions.verification_text)
                                    .flex_1()
                                    .min_w_0()
                                    .aria_label("六位验证码")
                                    .readonly(locked),
                            )
                            .when(self.functions.challenge.is_some(), |row| {
                                row.child(
                                    Button::new("function-read-code")
                                        .debug_selector(|| "function-read-code".into())
                                        .ghost()
                                        .icon(IconName::Redo)
                                        .loading(locked)
                                        .accessibility_label("获取验证码")
                                        .disabled(!live || locked)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if !this.functions.available
                                                || this.functions.busy
                                                || this.view_only()
                                            {
                                                return;
                                            }
                                            this.functions.busy = true;
                                            this.functions.message = "正在获取验证码…".into();
                                            this.send_function_command(FunctionCommand::FetchCode);
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("function-submit-code")
                                        .debug_selector(|| "function-submit-code".into())
                                        .primary()
                                        .icon(IconName::Check)
                                        .accessibility_label("提交验证码")
                                        .disabled(
                                            !live
                                                || locked
                                                || !rv_session::functions::valid_code(
                                                    self.functions
                                                        .verification_text
                                                        .read(cx)
                                                        .value()
                                                        .trim(),
                                                ),
                                        )
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.submit_function_code(window, cx)
                                        })),
                                )
                                .child(
                                    Button::new("function-cancel-verification")
                                        .ghost()
                                        .icon(IconName::Close)
                                        .accessibility_label("取消验证")
                                        .disabled(!live || locked)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let (Some((_, operation_id)), Some(challenge_id)) = (
                                                this.functions.operation.clone(),
                                                this.functions.challenge.clone(),
                                            ) {
                                                this.send_function_command(
                                                    FunctionCommand::CancelChallenge {
                                                        operation_id,
                                                        challenge_id,
                                                    },
                                                );
                                                this.functions.busy = true;
                                                this.functions.message = "正在取消验证…".into();
                                                cx.notify();
                                            }
                                        })),
                                )
                            }),
                    )
                },
            )
            .when(!live, |panel| {
                panel.child(div().text_xs().text_color(theme::muted(cx)).child(
                    if self.view_only() {
                        "只读连接不能执行手机操作"
                    } else {
                        "等待 ZXTouch 功能连接"
                    },
                ))
            })
            .when(
                self.functions.busy && self.functions.challenge.is_none(),
                |panel| {
                    panel.child(
                        div()
                            .w_full()
                            .min_w_0()
                            .debug_selector(|| "function-operation-message".into())
                            .child(function_loading(
                                "function-query-loading",
                                selected.map(|spec| spec.title).unwrap_or("手机操作"),
                                if self.functions.message.is_empty() {
                                    if query {
                                        "正在读取…"
                                    } else {
                                        "正在处理…"
                                    }
                                    .into()
                                } else {
                                    self.functions.message.clone()
                                },
                                cx,
                            )),
                    )
                },
            )
            .when(
                !completed && !self.functions.message.is_empty() && !self.functions.busy,
                |panel| {
                    let message = if self.functions.uncertain {
                        if self.functions.message.is_empty() {
                            SharedString::from("结果待确认")
                        } else {
                            self.functions.message.clone()
                        }
                    } else {
                        self.functions.message.clone()
                    };
                    let alert = if self.functions.uncertain {
                        Alert::warning("function-operation-status", message)
                    } else if self.functions.failed {
                        Alert::error("function-operation-status", message)
                    } else {
                        Alert::info("function-operation-status", message)
                    };
                    panel.child(
                        div()
                            .w_full()
                            .min_w_0()
                            .debug_selector(|| "function-operation-message".into())
                            .child(alert),
                    )
                },
            )
            .when(completed && !query, |panel| {
                let record = self.functions.result.as_ref().unwrap();
                let (title, detail) = result_summary(record);
                let alert = if detail.is_empty() {
                    Alert::success("function-result-status", title)
                } else {
                    Alert::success("function-result-status", detail).title(title)
                };
                panel.child(
                    div()
                        .debug_selector(|| "function-result-summary".into())
                        .child(alert),
                )
            })
            .when(query && completed, |panel| {
                panel.child(self.render_apple_query_result(cx))
            })
            .when(
                !self.functions.busy
                    && self
                        .functions
                        .result
                        .as_ref()
                        .is_some_and(|record| record["status"] == "completed"),
                |panel| {
                    panel.child(
                        crate::ui::tool_actions()
                            .debug_selector(|| "function-result-toolbar".into())
                            .justify_end()
                            .child(
                                Button::new("function-view-result")
                                    .debug_selector(|| "function-view-result".into())
                                    .ghost()
                                    .icon(IconName::SquareTerminal)
                                    .selected(self.functions.result_open)
                                    .text_color(theme::muted(cx))
                                    .accessibility_label(if self.functions.result_open {
                                        "收起原始结果"
                                    } else {
                                        "展开原始结果"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.functions.result_open = !this.functions.result_open;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("function-copy-result")
                                    .ghost()
                                    .icon(IconName::Copy)
                                    .text_color(theme::muted(cx))
                                    .accessibility_label("复制原始结果")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(result) = &this.functions.result {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                function_response_text(result),
                                            ));
                                        }
                                    })),
                            ),
                    )
                },
            )
            .when(
                self.functions
                    .result
                    .as_ref()
                    .is_some_and(|record| record["status"] == "completed")
                    && !self.functions.busy
                    && self.functions.result_open,
                |panel| {
                    panel.child(
                        div()
                            .id("function-result-text")
                            .debug_selector(|| "function-result-text".into())
                            .w_full()
                            .min_w_0()
                            .child(
                                Editor::new(&self.functions.result_editor)
                                    .readonly(true)
                                    .aria_label("ZXTouch 原始返回结果")
                                    .h(px((height - if side_by_side { 156. } else { 116. })
                                        .clamp(60., 320.))),
                            ),
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

fn domain_signed(state: Option<&serde_json::Value>, domain: &str) -> Option<bool> {
    let section = &state?[domain];
    match section["status"].as_str() {
        Some("signed_in") if section["account"].is_object() => Some(true),
        Some("not_signed_in") if section["account"].is_null() => Some(false),
        _ => None,
    }
}
fn account_action_requires_state(id: &str) -> bool {
    matches!(
        id,
        "store-login"
            | "store-logout"
            | "cloud-login"
            | "cloud-logout"
            | "apple-logout"
            | "findmy-off"
            | "apple-password-change"
    ) || apple_profile_action(id)
        || matches!(id, "device-list" | "device-remove")
}
fn apple_profile_action(id: &str) -> bool {
    matches!(
        id,
        "name"
            | "name-query"
            | "email-add"
            | "email-verify"
            | "email-query"
            | "forward"
            | "authorization-list"
            | "authorization-stop"
    )
}
fn account_action_known(id: &str, state: Option<&serde_json::Value>) -> bool {
    match id {
        "apple-password-change" => ["identity", "icloud", "app_store"]
            .iter()
            .all(|d| domain_signed(state, d).is_some()),
        "store-login" | "store-logout" => domain_signed(state, "app_store").is_some(),
        "cloud-login" => {
            domain_signed(state, "app_store") == Some(true)
                && state.is_some_and(|state| state["app_store"]["account"]["authenticated"] == true)
                && domain_signed(state, "icloud").is_some()
        }
        "cloud-logout" => domain_signed(state, "icloud").is_some(),
        "findmy-off" => {
            domain_signed(state, "icloud").is_some()
                && state
                    .and_then(|s| s["icloud"]["find_my_enabled"].as_bool())
                    .is_some()
        }
        "apple-logout" => ["app_store", "icloud", "identity"]
            .iter()
            .all(|d| domain_signed(state, d).is_some()),
        "device-list" | "device-remove" => domain_signed(state, "icloud").is_some(),
        id if apple_profile_action(id) => {
            domain_signed(state, "icloud").is_some() && domain_signed(state, "identity").is_some()
        }
        _ => true,
    }
}
fn account_action_visible(id: &str, state: Option<&serde_json::Value>) -> bool {
    match id {
        "apple-password-change" => ["identity", "icloud", "app_store"]
            .iter()
            .any(|d| domain_signed(state, d) == Some(true)),
        "store-login" => domain_signed(state, "app_store") == Some(false),
        "store-logout" => domain_signed(state, "app_store") == Some(true),
        "cloud-login" => {
            domain_signed(state, "icloud") == Some(false)
                && domain_signed(state, "app_store") == Some(true)
                && state.is_some_and(|state| state["app_store"]["account"]["authenticated"] == true)
        }
        "cloud-logout" => domain_signed(state, "icloud") == Some(true),
        "apple-logout" => ["app_store", "icloud", "identity"]
            .iter()
            .any(|d| domain_signed(state, d) == Some(true)),
        "findmy-off" => {
            domain_signed(state, "icloud") == Some(true)
                && state.and_then(|s| s["icloud"]["find_my_enabled"].as_bool()) == Some(true)
        }
        "device-list" | "device-remove" => domain_signed(state, "icloud") == Some(true),
        id if apple_profile_action(id) => {
            domain_signed(state, "identity") == Some(true)
                || domain_signed(state, "icloud") == Some(true)
        }
        _ => true,
    }
}

fn is_login_action(spec: &OperationSpec) -> bool {
    matches!(
        spec.id,
        "store-login" | "cloud-login" | "store-logout" | "cloud-logout" | "apple-logout"
    )
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
        "signOutAppStore" => "已退出 App Store",
        "signOutAppleID" => "已退出 Apple 账户",
        "signInAppStore" => "已登录 App Store",
        "changePassword" => "密码已修改",
        "enableICloud" => "已登录 iCloud",
        "disableFindMy" => "已关闭查找",
        "setName" => "姓名已更新",
        "setForwardingEmail" => "转发邮箱已更新",
        "beginAddingContactEmail"
            if record["result"]["outcome"] == "requires_email_verification" =>
        {
            "等待邮箱验证码"
        }
        "beginAddingContactEmail" => "联系邮箱已添加",
        "verifyContactEmail" => "邮箱已验证",
        "stopUsingAppleID" => "App 授权已停止",
        "removeOtherICloudDevices" => "其它设备已移除",
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

pub(super) fn function_error_message(record: &serde_json::Value) -> String {
    let fallback = if record["error"]["domain"] == "org.trollvnc.apple" {
        match record["error"]["code"].as_i64() {
            Some(1) => "手机暂不支持此操作",
            Some(2) => "手机服务权限不足",
            Some(4) => "请先关闭「查找」",
            Some(5) => "无法读取手机账户",
            Some(6) => "结果未确认，请先检查手机账户状态",
            Some(7) => "手机账户状态不一致",
            Some(8) => "账户验证未通过",
            Some(9) => "验证码无效或已过期",
            _ => "操作未完成",
        }
    } else if record["error"]["domain"] == "TVAppStateServices" {
        match record["error"]["reason"].as_str() {
            Some("restore_pending") => "当前环境恢复未完成",
            Some("name_required") => "当前环境尚未命名",
            Some("store_busy") => "备份目录正被占用",
            Some("operation_busy") => "手机操作仍在执行",
            Some("file_copy_failed") => match record["error"]["posix_code"].as_i64() {
                Some(1 | 13) => "恢复文件失败：权限不足",
                Some(28) => "恢复文件失败：存储空间不足",
                Some(30) => "恢复文件失败：目标为只读",
                Some(2) => "恢复文件失败：文件已不存在",
                Some(5) => "恢复文件失败：读写错误",
                Some(45 | 102) => "恢复文件失败：文件属性不受支持",
                _ => "恢复文件失败",
            },
            Some("archive_unavailable") => "所选备份已不存在",
            Some("invalid_archive") => "备份文件校验失败",
            Some("keychain_failed") => "凭据恢复失败",
            Some("identity_failed") => "设备身份恢复失败",
            _ => match record["error"]["code"].as_i64() {
                Some(1) => "无法准备手机环境服务",
                Some(2)
                    if record["progress"]["phase"] == "restore"
                        && record["progress"]["stage"] == "copy" =>
                {
                    "恢复文件失败"
                }
                Some(2) => "手机环境操作失败",
                Some(3) => "操作结束，但临时文件清理失败",
                Some(4) => "手机操作结果未确认",
                _ => "手机环境操作失败",
            },
        }
    } else {
        "操作未完成"
    };
    fallback.to_owned()
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

    fn account_fixture(store_signed: bool) -> serde_json::Value {
        serde_json::json!({
            "app_store":if store_signed { serde_json::json!({"status":"signed_in","account":{"apple_id":"demo@example.test","authenticated":true}}) }
                else { serde_json::json!({"status":"not_signed_in","account":null}) },
            "icloud":{"status":"signed_in","account":{"apple_id":"demo@example.test","enabled_dataclasses":["contacts"]},"find_my_enabled":true},
            "identity":{"status":"signed_in","account":{"apple_id":"demo@example.test"}}
        })
    }
    fn enable(view: &Entity<SessionView>, cx: &mut VisualTestContext) {
        view.update(cx, |view, cx| {
            view.functions.open = true;
            view.apply_function_event(FunctionEvent::Available(true), cx);
            view.apply_function_event(FunctionEvent::AccountState(Ok(account_fixture(false))), cx);
        });
    }
    #[test]
    fn user_errors_use_categories_instead_of_raw_service_messages() {
        let record = serde_json::json!({"error": {"domain":"org.trollvnc.apple", "code":9,
            "message":"AKAuthenticationError private backend failure"}});
        assert_eq!(super::function_error_message(&record), "验证码无效或已过期");
        assert_eq!(
            super::function_error_message(&serde_json::json!({"error":{"message":"SQL failure"}})),
            "操作未完成"
        );
    }

    #[test]
    fn menu_actions_follow_independent_account_domains() {
        use serde_json::json;
        let mut state = json!({"app_store":{"status":"signed_in","account":{"authenticated":true}},
            "icloud":{"status":"not_signed_in","account":null},"identity":{"status":"signed_in","account":{}}});
        assert!(!super::account_action_visible("store-login", Some(&state)));
        assert!(super::account_action_visible("store-logout", Some(&state)));
        assert!(super::account_action_visible("cloud-login", Some(&state)));
        assert!(!super::account_action_visible("cloud-logout", Some(&state)));
        assert!(!super::account_action_visible("findmy-off", Some(&state)));
        state["icloud"] = json!({"status":"signed_in","account":{"enabled_dataclasses":["contacts"]},"find_my_enabled":false});
        assert!(!super::account_action_visible("cloud-login", Some(&state)));
        assert!(super::account_action_visible("cloud-logout", Some(&state)));
        assert!(!super::account_action_visible("findmy-off", Some(&state)));
        state["icloud"]["find_my_enabled"] = json!(true);
        assert!(super::account_action_visible("findmy-off", Some(&state)));
        state["icloud"]["account"]["enabled_dataclasses"] = json!([]);
        assert!(!super::account_action_visible("cloud-login", Some(&state)));
        state["app_store"] = json!({"status":"conflict","account":null});
        assert!(!super::account_action_known("store-login", Some(&state)));
        assert!(!super::account_action_known("store-login", None));
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
    fn successful_logout_keeps_its_final_state_after_notification(cx: &mut TestAppContext) {
        let (view, cx, _) = setup(cx, false);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preview_function("logout-success", window, cx)
            });
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("function-result-summary").is_some());
        assert!(cx.debug_bounds("function-result-text").is_none());
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
    fn function_tabs_can_switch_repeatedly_without_hover_style_conflicts(cx: &mut TestAppContext) {
        let (view, cx, _) = setup(cx, false);
        enable(&view, cx);
        cx.update(|window, cx| window.draw(cx).clear(cx));
        for (category, selector) in [
            ("App 环境", "function-tab-App 环境"),
            ("Apple", "function-tab-Apple"),
            ("App 环境", "function-tab-App 环境"),
            ("Apple", "function-tab-Apple"),
        ] {
            let bounds = cx.debug_bounds(selector).unwrap();
            cx.simulate_click(bounds.center(), gpui::Modifiers::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            view.read_with(cx, |view, _| {
                assert_eq!(view.functions.environment.open, category == "App 环境");
            });
        }
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
        let list = cx.debug_bounds("function-list").unwrap();
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: list.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-260.))),
            modifiers: Default::default(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let bounds = cx.debug_bounds("function-authorization-list").unwrap();
        assert!(
            bounds.center().y < list.bottom(),
            "query must be scrolled into view: {bounds:?}"
        );
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
        assert!(cx.debug_bounds("function-query-content").is_some());
        assert!(cx.debug_bounds("function-result-text").is_none());
        let details = cx.debug_bounds("function-view-result").unwrap();
        cx.simulate_click(details.center(), gpui::Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
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
    fn completed_unknown_login_notifies_and_allows_an_explicit_new_attempt(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        while peer.commands.try_recv().is_ok() {}
        view.update(cx, |view, cx| {
            view.functions.selected = OPERATIONS.iter().find(|spec| spec.id == "cloud-login");
            view.apply_function_event(FunctionEvent::VerificationCodeReceived("053322".into()), cx);
            view.apply_function_event(
                FunctionEvent::State {
                    task: 31,
                    record: serde_json::json!({
                        "operation_id":"cloud-old", "method":"enableICloud", "status":"completed",
                        "result":{"outcome":"unknown","signed_in_domains":["identity","media"]}
                    }),
                },
                cx,
            );
            assert_eq!(view.functions.pending_results.len(), 1);
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("function-input-area").is_none());
        assert!(cx.debug_bounds("function-execute").is_none());
        assert!(cx.debug_bounds("function-query-resume").is_some());
        assert!(cx.debug_bounds("function-result-text").is_none());
        assert!(cx.debug_bounds("function-notification-details").is_some());
        assert!(
            peer.commands.try_recv().is_err(),
            "a notification must not replay login"
        );
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                assert!(view.functions.uncertain);
                assert!(view.functions.result.is_some());
                let spec = OPERATIONS
                    .iter()
                    .find(|spec| spec.id == "cloud-login")
                    .unwrap();
                view.select_function(spec, window, cx);
                assert!(!view.functions.uncertain);
                assert!(view.functions.result.is_none());
                assert!(view.functions.operation.is_none());
                view.functions.next_cloud_preparation += 1;
                let token = view.functions.next_cloud_preparation;
                view.functions.cloud_preparation = Some(token);
                view.functions.busy = true;
                let spec = view.functions.selected.unwrap();
                let parsed = super::parse_operation(spec, "new-password").unwrap();
                view.finish_cloud_login_preparation(
                    token,
                    parsed,
                    Ok(Some(rv_core::UnlockCode::new("001234".into()).unwrap())),
                    cx,
                );
            })
        });
        assert!(matches!(peer.commands.try_recv().map(unscoped).unwrap(),
            rv_session::SessionCommand::Function(FunctionCommand::Invoke {id, text})
                if id == "cloud-login" && text.contains("new-password")));
        // Viewing the immutable historical result must not replace the running new request.
        let bounds = cx.debug_bounds("function-notification-details").unwrap();
        cx.simulate_mouse_move(bounds.center(), None, gpui::Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let bounds = cx.debug_bounds("function-notification-details").unwrap();
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("function-notification-result").is_some());
        view.read_with(cx, |view, _| assert!(view.functions.busy));
        assert!(peer.commands.try_recv().is_err());
    }

    #[gpui::test]
    fn completed_cloud_result_stops_loading_even_after_account_refresh(cx: &mut TestAppContext) {
        let (view, cx, _) = setup(cx, false);
        enable(&view, cx);
        view.update(cx, |view, cx| {
            view.functions.selected = OPERATIONS.iter().find(|spec| spec.id == "cloud-login");
            view.functions.busy = true;
            view.apply_function_event(
                FunctionEvent::AccountState(Ok(serde_json::json!({
                    "icloud":{"status":"signed_in","account":{"enabled_dataclasses":["contacts"]}}
                }))),
                cx,
            );
            // A saved account alone is not the native operation's terminal reply.
            assert!(view.functions.busy);
            view.apply_function_event(
                FunctionEvent::ForOperation {
                    id: "cloud-login".into(),
                    event: Box::new(FunctionEvent::State {
                        task: 31,
                        record: serde_json::json!({"operation_id":"cloud", "method":"enableICloud",
                        "status":"completed", "result":{"outcome":"succeeded"}}),
                    }),
                },
                cx,
            );
            assert!(!view.functions.busy);
            assert!(!view.functions.uncertain);
            view.apply_function_event(FunctionEvent::AccountStateLoading, cx);
            assert!(!view.functions.busy);
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("function-query-loading").is_none());
        assert!(cx.debug_bounds("function-result-text").is_none());
        assert!(cx.debug_bounds("function-input-area").is_none());
        assert!(cx.debug_bounds("function-result-summary").is_some());
    }

    #[gpui::test]
    fn completed_logout_is_fresh_after_login_and_can_run_again(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        while peer.commands.try_recv().is_ok() {}
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let logout = OPERATIONS
                    .iter()
                    .find(|spec| spec.id == "store-logout")
                    .unwrap();
                let login = OPERATIONS
                    .iter()
                    .find(|spec| spec.id == "store-login")
                    .unwrap();
                view.apply_function_event(
                    FunctionEvent::AccountState(Ok(account_fixture(true))),
                    cx,
                );
                view.select_function(logout, window, cx);
                view.execute_function(window, cx);
                view.apply_function_event(FunctionEvent::State { task:31, record:serde_json::json!({
                "operation_id":"logout-1", "method":"signOutAppStore", "status":"completed",
                "result":{"outcome":"succeeded"}
            })}, cx);
                assert!(!view.functions.busy);
                view.apply_function_event(
                    FunctionEvent::AccountState(Ok(account_fixture(false))),
                    cx,
                );
                view.select_function(login, window, cx);
                view.functions.text.update(cx, |input, cx| {
                    input.set_value("demo@test----test-secret", window, cx)
                });
                view.execute_function(window, cx);
                view.apply_function_event(FunctionEvent::State { task:31, record:serde_json::json!({
                "operation_id":"login-1", "method":"signInAppStore", "status":"completed",
                "result":{"outcome":"succeeded"}
            })}, cx);
                view.apply_function_event(
                    FunctionEvent::AccountState(Ok(account_fixture(true))),
                    cx,
                );
                view.select_function(logout, window, cx);
                assert!(view.functions.result.is_none());
                assert!(!view.functions.result_open);
                view.execute_function(window, cx);
                assert!(view.functions.busy);
                // A second click while this new operation runs must not create another invocation.
                view.select_function(logout, window, cx);
                view.execute_function(window, cx);
            })
        });
        let mut ids = Vec::new();
        while let Ok(command) = peer.commands.try_recv().map(unscoped) {
            if let rv_session::SessionCommand::Function(FunctionCommand::Invoke { id, .. }) =
                command
            {
                ids.push(id);
            }
        }
        assert_eq!(ids, ["store-logout", "store-login", "store-logout"]);
    }

    #[gpui::test]
    fn unknown_logout_remains_visible_after_account_state_changes(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        while peer.commands.try_recv().is_ok() {}
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let logout = OPERATIONS
                    .iter()
                    .find(|spec| spec.id == "store-logout")
                    .unwrap();
                let query = OPERATIONS
                    .iter()
                    .find(|spec| spec.id == "name-query")
                    .unwrap();
                view.apply_function_event(
                    FunctionEvent::AccountState(Ok(account_fixture(true))),
                    cx,
                );
                view.select_function(logout, window, cx);
                view.apply_function_event(FunctionEvent::State {task:31, record:serde_json::json!({
                "operation_id":"logout-unknown", "method":"signOutAppStore", "status":"running"
            })}, cx);
                view.apply_function_event(FunctionEvent::Uncertain("结果待确认".into()), cx);
                view.apply_function_event(
                    FunctionEvent::AccountState(Ok(account_fixture(false))),
                    cx,
                );
                view.select_function(query, window, cx);
                view.select_function(logout, window, cx);
                assert!(view.functions.uncertain);
                assert_eq!(
                    view.functions.operation.as_ref().unwrap().1,
                    "logout-unknown"
                );
                assert!(view.functions.has_pending(logout));
                view.execute_function(window, cx);
            })
        });
        // Switching to a read-only query is allowed; no duplicate logout is issued.
        while let Ok(command) = peer.commands.try_recv().map(unscoped) {
            assert!(!matches!(command,
                rv_session::SessionCommand::Function(FunctionCommand::Invoke {id, ..}) if id == "store-logout"));
        }
    }

    #[gpui::test]
    fn code_timeout_keeps_manual_challenge_and_hides_unrelated_actions(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        while peer.commands.try_recv().is_ok() {}
        cx.update(|window, cx| view.update(cx, |view, cx| {
            let login = OPERATIONS.iter().find(|spec| spec.id == "store-login").unwrap();
            view.select_function(login, window, cx);
            view.apply_function_event(FunctionEvent::State { task:31, record:serde_json::json!({
                "operation_id":"login-pending", "method":"signInAppStore", "status":"requires_verification_code",
                "challenge":{"challenge_id":"challenge"}
            })}, cx);
            view.apply_function_event(FunctionEvent::CodeFetchWaiting { remaining_seconds:0 }, cx);
            assert!(!view.functions.busy);
            assert!(!view.functions.uncertain);
            assert_eq!(view.functions.challenge.as_deref(), Some("challenge"));
            assert_eq!(view.functions.message, "请手动输入验证码");
            view.select_function(login, window, cx);
            assert_eq!(view.functions.operation.as_ref().unwrap().1, "login-pending");
        }));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("function-verification-row").is_some());
        assert!(cx.debug_bounds("function-read-code").is_some());
        assert!(cx.debug_bounds("function-submit-code").is_some());
        assert!(cx.debug_bounds("function-query-resume").is_none());
        assert!(cx.debug_bounds("function-input-area").is_none());
        assert!(cx.debug_bounds("function-execute").is_none());
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.functions
                    .verification_text
                    .update(cx, |input, cx| input.set_value("053322", window, cx));
                view.submit_function_code(window, cx);
            })
        });
        assert!(matches!(peer.commands.try_recv().map(unscoped).unwrap(),
            rv_session::SessionCommand::Function(FunctionCommand::SubmitCode {operation_id, code, ..})
                if operation_id == "login-pending" && code == "053322"));
    }

    #[gpui::test]
    fn stale_cloud_preparation_cannot_submit_or_open_settings(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        while peer.commands.try_recv().is_ok() {}
        view.update(cx, |view, cx| {
            let spec = OPERATIONS
                .iter()
                .find(|spec| spec.id == "cloud-login")
                .unwrap();
            view.functions.selected = Some(spec);
            view.functions.busy = true;
            view.functions.cloud_preparation = Some(2);
            for code in [
                None,
                Some(rv_core::UnlockCode::new("001234".into()).unwrap()),
            ] {
                let parsed = super::parse_operation(spec, "old-password").unwrap();
                view.finish_cloud_login_preparation(1, parsed, Ok(code), cx);
                assert_eq!(view.functions.cloud_preparation, Some(2));
                assert!(view.functions.busy);
                assert!(!view.unlock.editing());
            }
            let parsed = super::parse_operation(spec, "old-password").unwrap();
            view.finish_cloud_login_preparation(
                1,
                parsed,
                Err(rv_core::UnlockCode::new("invalid".into()).unwrap_err()),
                cx,
            );
            assert_eq!(view.functions.cloud_preparation, Some(2));
            assert!(view.functions.busy);
            view.functions.disconnected();
            assert!(view.functions.cloud_preparation.is_none());
            assert!(!view.functions.busy);
            assert!(!view.functions.uncertain);
        });
        assert!(peer.commands.try_recv().is_err());
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
                view.apply_function_event(
                    FunctionEvent::AccountState(Ok(account_fixture(false))),
                    cx,
                );
                view.select_function(&OPERATIONS[0], window, cx);
            });
            window.draw(cx).clear(cx);
        });
        assert!(cx.debug_bounds("function-browser").is_none());
        let editor = cx.debug_bounds("function-editor").unwrap();
        let execute = cx.debug_bounds("function-execute").unwrap();
        assert!(editor.right() <= px(1100.) && editor.bottom() <= px(760.));
        assert!(execute.bottom() <= editor.bottom());
        let phone = view.read_with(cx, |view, _| view.image_box.get());
        assert!(phone.size.width > px(0.) && phone.right() < editor.left());
        assert!(cx.debug_bounds("function-back").is_some());
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
