//! Presentation of Apple operations. Native receipts remain available in details.
use super::*;

fn account_id<'a>(state: Option<&'a serde_json::Value>, domain: &str) -> Option<&'a str> {
    let state = state?;
    (domain_signed(Some(state), domain) == Some(true))
        .then(|| state[domain]["account"]["apple_id"].as_str())
        .flatten()
        .filter(|id| !id.is_empty())
}

fn operation_account<'a>(
    spec: &OperationSpec,
    state: Option<&'a serde_json::Value>,
) -> Option<&'a str> {
    match spec.id {
        "store-login" | "apple-query" => None,
        "cloud-login" | "store-logout" => account_id(state, "app_store"),
        "cloud-logout" | "findmy-off" | "device-list" => account_id(state, "icloud"),
        "apple-logout" => account_id(state, "identity")
            .or_else(|| account_id(state, "icloud"))
            .or_else(|| account_id(state, "app_store")),
        _ => account_id(state, "identity").or_else(|| account_id(state, "icloud")),
    }
}

fn operation_scope(id: &str) -> &'static str {
    match id {
        "store-login" => "登录 App Store，使用下方粘贴的账户资料。",
        "cloud-login" => "使用当前 App Store 账户登录 iCloud。",
        "store-logout" => "仅退出 App Store。",
        "cloud-logout" => "退出 iCloud；请选择本机数据的处理方式。",
        "apple-logout" => "退出 iCloud、Apple 身份与 App Store；请选择本机数据的处理方式。",
        "findmy-off" => "关闭当前 iCloud 账户的「查找我的 iPhone」。",
        "forward" => "更改该账户全部隐藏邮箱的统一转发地址。",
        "authorization-list" => "读取授权列表，选择 App 后可停止授权。",
        "authorization-stop" => "停止指定 App 使用此 Apple 账户登录。",
        "device-list" => "读取关联设备；本机将在列表中标明。",
        "device-remove" => "移除此账户的其它关联设备，保留本机。",
        "email-add" | "email-verify" => "为当前账户添加联系邮箱并完成验证。",
        "apple-query" => "读取手机当前的账户资料。",
        "name" | "name-query" => "当前 Apple 账户的姓名资料。",
        "email-query" => "当前 Apple 账户的联系邮箱。",
        _ => "",
    }
}

fn profile_status(section: &serde_json::Value) -> &'static str {
    match section["status"].as_str() {
        Some("not_signed_in") => "未登录",
        Some("error") | Some("conflict") => "读取失败",
        _ => "未取得资料",
    }
}

fn account_detail_row(
    id: &'static str,
    label: &'static str,
    value: String,
    cx: &App,
) -> impl IntoElement {
    h_flex()
        .items_start()
        .gap_3()
        .min_w_0()
        .debug_selector(move || format!("apple-state-{id}"))
        .child(crate::typography::label(label, cx).text_color(theme::muted(cx)))
        .child(div().flex_1().min_w_0().text_right().text_sm().child(value))
}

impl SessionView {
    pub(super) fn function_menu_visible(&self, spec: &OperationSpec) -> bool {
        // Keep live work reachable, but never offer its inverse at the same time.
        let pending = |spec: &OperationSpec| {
            if self.functions.panel_key() == Some(spec.id) {
                self.functions.has_pending(spec) && !self.functions.snapshot().finished()
            } else {
                self.functions
                    .panels
                    .get(spec.id)
                    .is_some_and(|panel| panel.resumable() && !panel.finished())
            }
        };
        if pending(spec) {
            return true;
        }
        let inverse = match spec.id {
            "store-login" => Some("store-logout"),
            "store-logout" => Some("store-login"),
            "cloud-login" => Some("cloud-logout"),
            "cloud-logout" => Some("cloud-login"),
            _ => None,
        };
        if inverse
            .and_then(|id| OPERATIONS.iter().find(|spec| spec.id == id))
            .is_some_and(pending)
        {
            return false;
        }
        account_action_visible(spec.id, self.functions.account_state.as_ref())
            && (!account_action_requires_state(spec.id)
                || account_action_known(spec.id, self.functions.account_state.as_ref()))
    }

    pub(super) fn confirm_apple_operation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(spec) = self.functions.selected else {
            return;
        };
        if !matches!(spec.id, "authorization-stop" | "device-remove") {
            self.execute_function(window, cx);
            return;
        }
        if self.functions.busy || self.functions.uncertain {
            return;
        }
        let text = self.function_input_value(cx).to_string();
        if let Err(error) = parse_operation(spec, &text) {
            self.functions.message = error.into();
            cx.notify();
            return;
        }
        let account = self.functions.expected_account.clone().or_else(|| {
            operation_account(spec, self.functions.account_state.as_ref()).map(str::to_owned)
        });
        if account.as_deref().is_some_and(|expected| {
            operation_account(spec, self.functions.account_state.as_ref())
                .is_none_or(|current| !current.eq_ignore_ascii_case(expected))
        }) {
            self.functions.message = "账户状态已变化，请重新查询后再操作".into();
            cx.notify();
            return;
        }
        let description = format!(
            "目标账户：{}\n{}{}",
            account.as_deref().unwrap_or("提交前重新读取手机当前账户"),
            operation_scope(spec.id),
            if spec.id == "authorization-stop" {
                format!("\n{text}")
            } else {
                String::new()
            }
        );
        let owner = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let owner = owner.clone();
            let account = account.clone();
            let text = text.clone();
            alert
                .title(spec.title)
                .description(description.clone())
                .confirm()
                .button_props(
                    gpui_component::dialog::DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text("确认执行")
                        .cancel_text("返回")
                        .ok_variant(gpui_component::button::ButtonVariant::Danger),
                )
                .on_ok(move |_, window, cx| {
                    owner.update(cx, |this, cx| {
                        if this
                            .functions
                            .selected
                            .is_some_and(|selected| selected.id == spec.id)
                            && this.function_input_value(cx).as_ref() == text
                            && operation_account(spec, this.functions.account_state.as_ref())
                                .zip(account.as_deref())
                                .is_some_and(|(current, expected)| {
                                    current.eq_ignore_ascii_case(expected)
                                })
                        {
                            this.functions.expected_account = account.clone();
                            this.execute_function(window, cx);
                        } else {
                            this.functions.message = "操作目标已变化，请重新核对后提交".into();
                            cx.notify();
                        }
                    });
                    true
                })
        });
    }

    pub(super) fn function_unavailable_reason(&self, spec: &OperationSpec) -> Option<String> {
        if self.functions.has_pending(spec) {
            return None;
        }
        if self.view_only() {
            return Some("仅查看连接，无法操作".into());
        }
        if !self.functions.available || self.phase != Phase::Connected {
            return Some("等待手机功能连接".into());
        }
        if !account_action_requires_state(spec.id) {
            return None;
        }
        if self.functions.account_loading {
            return Some("正在读取账户…".into());
        }
        let state = self.functions.account_state.as_ref();
        if account_action_visible(spec.id, state) && !account_action_known(spec.id, state) {
            return Some(
                if spec.id == "cloud-login" && domain_signed(state, "app_store") == Some(false) {
                    "请先登录 App Store"
                } else {
                    "账户状态未确认，请刷新"
                }
                .into(),
            );
        }
        if !account_action_visible(spec.id, state) {
            return Some(
                match spec.id {
                    "store-login" => "请先退出当前 App Store 账户",
                    "store-logout" => "尚未登录 App Store",
                    "cloud-login" => "已登录 iCloud",
                    "cloud-logout" => "尚未登录 iCloud",
                    "apple-logout" => "尚未登录 Apple 账户",
                    "findmy-off" if domain_signed(state, "icloud") == Some(false) => {
                        "尚未登录 iCloud"
                    }
                    "findmy-off" => "查找已关闭",
                    _ => "当前无法执行",
                }
                .into(),
            );
        }
        None
    }

    pub(super) fn render_apple_operation_context(
        &self,
        spec: &'static OperationSpec,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let login_account = (spec.id == "store-login" && self.functions.challenge.is_some())
            .then(|| parse_operation(spec, self.functions.text.read(cx).value().as_ref()).ok())
            .flatten()
            .and_then(|parsed| parsed.params["apple_id"].as_str().map(str::to_owned));
        let account = login_account
            .as_deref()
            .or(self.functions.expected_account.as_deref())
            .or_else(|| {
                self.functions
                    .result
                    .as_ref()
                    .filter(|_| is_query(spec))
                    .and_then(|record| record["result"]["apple_id"].as_str())
            })
            .or_else(|| operation_account(spec, self.functions.account_state.as_ref()));
        crate::ui::tool_section()
            .debug_selector(|| "function-apple-context".into())
            .when(
                (spec.id != "store-login" || account.is_some()) && spec.id != "apple-query",
                |column| {
                    column.child(crate::typography::label(
                        account.unwrap_or("账户未确认").to_owned(),
                        cx,
                    ))
                },
            )
            .when_some(self.functions.account_error.clone(), |column, error| {
                column.child(
                    crate::typography::caption(format!("账户读取失败：{error}"), cx)
                        .text_color(theme::muted(cx)),
                )
            })
            .when(
                cfg!(target_os = "windows") && matches!(spec.id, "email-add" | "email-verify"),
                |column| {
                    column
                        .child(self.render_mailbox_launcher(cx))
                        .when(self.mailbox.open, |column| {
                            column.child(self.render_mailbox_panel(cx))
                        })
                },
            )
    }

    /// Open a focused form from a query, without submitting any mutation.
    fn open_apple_followup(
        &mut self,
        id: &'static str,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(spec) = OPERATIONS.iter().find(|spec| spec.id == id) else {
            return;
        };
        let Some(account) = self
            .functions
            .result
            .as_ref()
            .and_then(|record| record["result"]["apple_id"].as_str())
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
        else {
            self.functions.message = "结果缺少账户标识，请刷新查询后再操作".into();
            cx.notify();
            return;
        };
        if operation_account(spec, self.functions.account_state.as_ref())
            .is_none_or(|current| !current.eq_ignore_ascii_case(&account))
        {
            self.functions.message = "账户状态已变化，请刷新查询后再操作".into();
            cx.notify();
            return;
        }
        // Never overwrite another draft or an operation waiting for verification/recovery.
        let existing = self.functions.has_pending(spec)
            || self
                .functions
                .panels
                .get(id)
                .is_some_and(|panel| !panel.finished());
        self.select_function(spec, window, cx);
        if !existing
            && self
                .functions
                .selected
                .is_some_and(|selected| selected.id == id)
        {
            self.functions.expected_account = Some(account);
            self.functions
                .text
                .update(cx, |input, cx| input.set_value(text, window, cx));
        } else if existing && !self.functions.busy {
            self.functions.message = "已恢复该功能的原操作，请核对目标后继续".into();
            cx.notify();
        }
    }

    pub(super) fn render_apple_query_result(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(spec) = self.functions.selected.filter(|spec| spec.task == 31) else {
            return div().into_any_element();
        };
        let Some(result) = self
            .functions
            .result
            .as_ref()
            .map(|record| &record["result"])
        else {
            return div().into_any_element();
        };
        let mut content = crate::ui::tool_section()
            .min_w_0()
            .debug_selector(|| "function-query-content".into());
        let followup_account = result["apple_id"].as_str().filter(|id| !id.is_empty());
        let current = operation_account(spec, self.functions.account_state.as_ref());
        let stale = followup_account
            .zip(current)
            .is_none_or(|(previous, current)| !previous.eq_ignore_ascii_case(current));
        if stale && spec.id != "apple-query" {
            content = content.child(
                div().debug_selector(|| "apple-query-stale".into()).child(
                    crate::typography::caption("账户状态未匹配此结果，请刷新查询后再操作。", cx)
                        .text_color(theme::muted(cx)),
                ),
            );
        }
        match spec.id {
            "apple-query" => {
                for (domain, label) in [
                    ("identity", "Apple 账户"),
                    ("app_store", "App Store"),
                    ("icloud", "iCloud"),
                ] {
                    let status = match domain_signed(Some(result), domain) {
                        Some(true) => "已登录",
                        Some(false) => "未登录",
                        None => "状态未确认",
                    };
                    content = content.child(
                        v_flex()
                            .gap_1()
                            .min_w_0()
                            .py_1()
                            .debug_selector(move || format!("apple-state-{domain}"))
                            .child(
                                h_flex()
                                    .justify_between()
                                    .child(crate::typography::label(label, cx))
                                    .child(
                                        crate::typography::label(status, cx)
                                            .text_color(theme::muted(cx)),
                                    ),
                            )
                            .when_some(account_id(Some(result), domain), |row, account| {
                                row.child(
                                    crate::typography::caption(account.to_owned(), cx)
                                        .text_color(theme::muted(cx)),
                                )
                            }),
                    );
                }
                let settings = &result["apple_settings"];
                let name = &settings["name"];
                let email = &settings["email"];
                let full_name = [
                    name["data"]["first_name"].as_str(),
                    name["data"]["last_name"].as_str(),
                ]
                .into_iter()
                .filter(|_| name["status"] == "available")
                .flatten()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
                content = content.child(div().border_t_1().border_color(theme::line(cx)).mt_1());
                content = content.child(account_detail_row(
                    "name",
                    "姓名",
                    if full_name.is_empty() {
                        profile_status(name).into()
                    } else {
                        full_name
                    },
                    cx,
                ));
                if email["status"] == "available" {
                    let addresses = email["data"]["reachable_emails"]
                        .as_array()
                        .map(|values| {
                            values
                                .iter()
                                .filter_map(|value| value.as_str())
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .filter(|value| !value.is_empty())
                        .or_else(|| email["data"]["primary_email"].as_str().map(str::to_owned))
                        .unwrap_or_else(|| "暂无联系邮箱".into());
                    content = content.child(account_detail_row("email", "联系邮箱", addresses, cx));
                    content = content.child(account_detail_row(
                        "forwarding",
                        "转发邮箱",
                        email["data"]["forwarding_email"]
                            .as_str()
                            .filter(|value| !value.is_empty())
                            .unwrap_or("未设置")
                            .into(),
                        cx,
                    ));
                } else {
                    content = content.child(account_detail_row(
                        "email",
                        "联系邮箱",
                        profile_status(email).into(),
                        cx,
                    ));
                }
                for (id, label, section, key, unit) in [
                    (
                        "authorizations",
                        "App 授权",
                        &settings["sign_in_with_apple"],
                        "apps",
                        "个",
                    ),
                    (
                        "devices",
                        "关联设备",
                        &result["icloud"]["devices"],
                        "devices",
                        "台",
                    ),
                ] {
                    let value = section["data"][key]
                        .as_array()
                        .filter(|_| section["status"] == "available")
                        .map(|items| format!("{} {unit}", items.len()))
                        .unwrap_or_else(|| profile_status(section).into());
                    content = content.child(account_detail_row(id, label, value, cx));
                }
            }
            "authorization-list" => {
                if let Some(apps) = result["apps"].as_array() {
                    if apps.is_empty() {
                        content = content.child("此账户暂无 App 授权");
                    }
                    for (index, app) in apps.iter().enumerate() {
                        let Some(bundle) = app["bundle_id"]
                            .as_str()
                            .filter(|id| !id.is_empty() && !id.contains(['\n', '\r']))
                        else {
                            continue;
                        };
                        let bundle = bundle.to_owned();
                        let name = app["name"]
                            .as_str()
                            .filter(|name| !name.is_empty())
                            .unwrap_or(&bundle)
                            .to_owned();
                        content = content.child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .child(crate::typography::label(name, cx))
                                        .child(
                                            crate::typography::caption(bundle.clone(), cx)
                                                .text_color(theme::muted(cx)),
                                        ),
                                )
                                .child(
                                    Button::new(("apple-stop-authorization", index))
                                        .label("停止授权")
                                        .ghost()
                                        .disabled(stale)
                                        .debug_selector(move || {
                                            format!("apple-stop-authorization-{index}")
                                        })
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.open_apple_followup(
                                                "authorization-stop",
                                                format!("App={bundle}"),
                                                window,
                                                cx,
                                            );
                                        })),
                                ),
                        );
                    }
                } else {
                    content = content.child("未取得授权列表");
                }
            }
            "device-list" => {
                if let Some(devices) = result["devices"].as_array() {
                    if devices.is_empty() {
                        content = content.child("设备列表为空");
                    }
                    let others = devices
                        .iter()
                        .filter(|device| device["current_device"] == false)
                        .count();
                    for device in devices {
                        content = content.child(
                            v_flex()
                                .gap_1()
                                .child(crate::typography::label(
                                    format!(
                                        "{}{}",
                                        device["name"].as_str().unwrap_or("未命名设备"),
                                        if device["current_device"] == true {
                                            " · 本机"
                                        } else {
                                            ""
                                        }
                                    ),
                                    cx,
                                ))
                                .child(
                                    crate::typography::caption(
                                        device["model"].as_str().unwrap_or("").to_owned(),
                                        cx,
                                    )
                                    .text_color(theme::muted(cx)),
                                ),
                        );
                    }
                    if others > 0 {
                        content = content.child(
                            Button::new("apple-remove-other-devices")
                                .debug_selector(|| "apple-remove-other-devices".into())
                                .label(format!("移除其它设备（{others}）"))
                                .ghost()
                                .disabled(stale)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_apple_followup(
                                        "device-remove",
                                        String::new(),
                                        window,
                                        cx,
                                    );
                                })),
                        );
                    }
                } else {
                    content = content.child("未取得设备列表");
                }
            }
            "name-query" => {
                for (key, label) in [("first_name", "名"), ("last_name", "姓")] {
                    if let Some(value) = result[key].as_str() {
                        content = content
                            .child(crate::typography::label(format!("{label}：{value}"), cx));
                    }
                }
                let first = result["first_name"].as_str().unwrap_or_default();
                let last = result["last_name"].as_str().unwrap_or_default();
                let text = format!("名={first}\n姓={last}");
                content = content.child(
                    Button::new("apple-edit-name")
                        .label("修改姓名")
                        .ghost()
                        .disabled(stale)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_apple_followup("name", text.clone(), window, cx);
                        })),
                );
            }
            "email-query" => {
                if let Some(emails) = result["reachable_emails"].as_array() {
                    if emails.is_empty() {
                        content = content.child("暂无联系邮箱");
                    }
                    for email in emails {
                        // The native contract uses strings; do not infer an empty list for unsupported data.
                        content = content.child(crate::typography::label(
                            email
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| email.to_string()),
                            cx,
                        ));
                    }
                } else {
                    content = content.child("未取得联系邮箱");
                }
                content = content.child(
                    Button::new("apple-add-email")
                        .label("添加联系邮箱")
                        .ghost()
                        .disabled(stale)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_apple_followup("email-add", String::new(), window, cx);
                        })),
                );
            }
            _ => {
                content = content.child("账户资料已读取");
            }
        }
        content.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::super::{FunctionCommand, FunctionEvent, OPERATIONS, SessionView};
    use super::operation_account;
    use crate::session_window::ui_tests::setup;
    use gpui::{Entity, TestAppContext, VisualTestContext, px, size};
    use gpui_component::WindowExt;
    use serde_json::json;

    fn ready(view: &Entity<SessionView>, cx: &mut VisualTestContext) {
        view.update(cx, |view, cx| {
            view.functions.open = true;
            view.apply_function_event(FunctionEvent::Available(true), cx);
            view.apply_function_event(FunctionEvent::AccountState(Ok(json!({
                "app_store":{"status":"signed_in","account":{"apple_id":"store@example.test","authenticated":true}},
                "icloud":{"status":"signed_in","account":{"apple_id":"cloud@example.test","enabled_dataclasses":["contacts"]},"find_my_enabled":true},
                "identity":{"status":"signed_in","account":{"apple_id":"cloud@example.test"}}
            }))), cx);
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"));
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn assert_no_function_commands(peer: &mut rv_session::SessionTestPeer) {
        while let Ok(command) = peer.commands.try_recv() {
            assert!(
                !matches!(command, rv_session::SessionCommand::Function(_)),
                "unexpected command: {command:?}"
            );
        }
    }

    #[gpui::test]
    fn menu_shows_only_actions_for_current_accounts(cx: &mut TestAppContext) {
        let (view, cx, _) = setup(cx, false);
        ready(&view, cx);
        assert!(cx.debug_bounds("function-store-login").is_none());
        assert!(cx.debug_bounds("function-store-logout").is_some());
        assert!(cx.debug_bounds("function-cloud-login").is_none());
        assert!(cx.debug_bounds("function-cloud-logout").is_some());
        view.read_with(cx, |view, _| {
            assert_eq!(
                view.function_unavailable_reason(&OPERATIONS[0]).as_deref(),
                Some("请先退出当前 App Store 账户")
            );
            let spec = OPERATIONS
                .iter()
                .find(|spec| spec.id == "cloud-login")
                .unwrap();
            assert_eq!(
                operation_account(spec, view.functions.account_state.as_ref()),
                Some("store@example.test")
            );
            let spec = OPERATIONS
                .iter()
                .find(|spec| spec.id == "cloud-logout")
                .unwrap();
            assert_eq!(
                operation_account(spec, view.functions.account_state.as_ref()),
                Some("cloud@example.test")
            );
        });
        view.update(cx, |view, cx| {
            view.apply_function_event(FunctionEvent::AccountState(Err("测试读取失败".into())), cx)
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("function-account-error").is_some());
        assert!(cx.debug_bounds("function-store-login").is_none());
        assert!(cx.debug_bounds("function-store-logout").is_none());
    }

    #[gpui::test]
    fn menu_hides_icloud_dependencies_and_keeps_login_logout_exclusive(cx: &mut TestAppContext) {
        let (view, cx, _) = setup(cx, false);
        ready(&view, cx);
        for store in [false, true] {
            for cloud in [false, true] {
                let section = |signed| {
                    if signed {
                        json!({"status":"signed_in","account":{"apple_id":"demo@example.test","authenticated":true,"enabled_dataclasses":["contacts"]}})
                    } else {
                        json!({"status":"not_signed_in","account":null})
                    }
                };
                view.update(cx, |view, cx| {
                    view.apply_function_event(
                        FunctionEvent::AccountState(Ok(json!({
                            "app_store":section(store), "icloud":section(cloud),
                            // Identity survives iCloud logout: account settings stay, cloud devices do not.
                            "identity":section(true)
                        }))),
                        cx,
                    )
                });
                cx.update(|window, cx| window.draw(cx).clear(cx));
                for (selector, visible) in [
                    ("function-store-login", !store),
                    ("function-store-logout", store),
                    ("function-cloud-login", store && !cloud),
                    ("function-cloud-logout", cloud),
                    ("function-name", true),
                    ("function-email-add", true),
                    ("function-forward", true),
                    ("function-authorization-list", true),
                    ("function-device-list", cloud),
                    ("function-device-remove", cloud),
                    ("function-findmy-off", false),
                ] {
                    assert_eq!(
                        cx.debug_bounds(selector).is_some(),
                        visible,
                        "{selector}: store={store}, cloud={cloud}"
                    );
                }
            }
        }
    }

    #[gpui::test]
    fn menu_and_operation_keep_the_same_navigation_and_insets(cx: &mut TestAppContext) {
        let (view, cx, _) = setup(cx, false);
        ready(&view, cx);
        let menu = cx.debug_bounds("function-browser").unwrap();
        let navigation = cx.debug_bounds("function-navigation-bar").unwrap();
        let close = cx.debug_bounds("function-close").unwrap();
        let tab = cx.debug_bounds("function-tab-Apple").unwrap();
        let menu_content = cx.debug_bounds("function-menu-content-name").unwrap();
        click(cx, "function-name");
        let editor = cx.debug_bounds("function-editor").unwrap();
        assert_eq!(menu.left(), editor.left());
        assert_eq!(menu.top(), editor.top());
        assert_eq!(menu.size.width, editor.size.width);
        assert_eq!(
            navigation,
            cx.debug_bounds("function-navigation-bar").unwrap()
        );
        assert_eq!(close, cx.debug_bounds("function-close").unwrap());
        assert_eq!(tab, cx.debug_bounds("function-tab-Apple").unwrap());
        let input = cx.debug_bounds("function-input-area").unwrap();
        assert_eq!(menu_content.left(), input.left());
        assert_eq!(menu_content.right(), input.right());
        assert!(cx.debug_bounds("function-account-refresh").is_none());
    }

    #[gpui::test]
    fn account_query_shows_data_and_partial_errors_without_opening_raw_json(
        cx: &mut TestAppContext,
    ) {
        let (view, cx, _) = setup(cx, false);
        ready(&view, cx);
        cx.update(|window, cx| view.update(cx, |view, cx| {
            view.select_function(OPERATIONS.iter().find(|spec| spec.id == "apple-query").unwrap(), window, cx);
            view.apply_function_event(FunctionEvent::State {task:31, record:json!({
                "operation_id":"aggregate", "method":"queryState", "status":"completed", "result":{
                    "identity":{"status":"signed_in","account":{"apple_id":"demo@example.test"}},
                    "app_store":{"status":"signed_in","account":{"apple_id":"demo@example.test","authenticated":true}},
                    "icloud":{"status":"not_signed_in","account":null,"devices":{"status":"not_signed_in","data":null}},
                    "apple_settings":{"status":"signed_in","apple_id":"demo@example.test",
                        "name":{"status":"error","data":null,"error":{"code":5}},
                        "email":{"status":"available","data":{"primary_email":"demo@example.test","reachable_emails":["demo@example.test"],"forwarding_email":null}},
                        "sign_in_with_apple":{"status":"error","data":null,"error":{"code":5}}
                    }
                }
            })},cx);
        }));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        for selector in [
            "apple-state-identity",
            "apple-state-app_store",
            "apple-state-icloud",
            "apple-state-name",
            "apple-state-email",
            "apple-state-forwarding",
            "apple-state-authorizations",
            "apple-state-devices",
        ] {
            assert!(cx.debug_bounds(selector).is_some(), "{selector}");
        }
        assert!(cx.debug_bounds("function-apple-context").is_none());
        assert!(cx.debug_bounds("function-result-text").is_none());
        assert!(cx.debug_bounds("function-query-refresh").is_some());
        assert!(cx.debug_bounds("function-account-refresh").is_none());
    }

    #[gpui::test]
    fn pending_login_stays_reachable_without_exposing_logout(cx: &mut TestAppContext) {
        let (view, cx, _) = setup(cx, false);
        ready(&view, cx);
        cx.update(|window, cx| view.update(cx, |view, cx| {
            view.preview_function("login-challenge", window, cx);
            view.close_function_editor(window, cx);
            view.apply_function_event(FunctionEvent::AccountState(Ok(json!({
                "app_store":{"status":"signed_in","account":{"apple_id":"demo@example.com","authenticated":true}},
                "icloud":{"status":"not_signed_in","account":null},
                "identity":{"status":"not_signed_in","account":null}
            }))), cx);
        }));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("function-store-login").is_some());
        assert!(cx.debug_bounds("function-store-logout").is_none());
        click(cx, "function-store-login");
        view.read_with(cx, |view, _| assert!(view.functions.challenge.is_some()));
    }

    #[gpui::test]
    fn apple_form_stays_single_and_preserves_draft_on_back_and_close(cx: &mut TestAppContext) {
        let (view, cx, _) = setup(cx, false);
        ready(&view, cx);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let spec = OPERATIONS.iter().find(|spec| spec.id == "name").unwrap();
                view.select_function(spec, window, cx);
                view.functions.text.update(cx, |input, cx| {
                    input.set_value("名=Alice\n姓=Smith", window, cx)
                });
            })
        });
        for (width, height) in [(1100., 760.), (640., 400.), (900., 480.)] {
            cx.simulate_resize(size(px(width), px(height)));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(cx.debug_bounds("function-browser").is_none());
            let bounds = cx.debug_bounds("function-editor").unwrap();
            assert!(bounds.size.width <= px(360.));
            assert!(bounds.right() <= px(width));
            assert!(bounds.bottom() <= px(height));
            assert!(cx.debug_bounds("function-back").is_some());
        }
        click(cx, "function-back");
        assert!(cx.debug_bounds("function-browser").is_some());
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.select_function(
                    OPERATIONS.iter().find(|spec| spec.id == "name").unwrap(),
                    window,
                    cx,
                );
            })
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        click(cx, "function-close");
        cx.update(|window, cx| view.update(cx, |view, cx| view.toggle_functions(window, cx)));
        view.read_with(cx, |view, cx| {
            assert_eq!(view.functions.text.read(cx).value(), "名=Alice\n姓=Smith");
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        click(cx, "function-back");
        view.read_with(cx, |view, _| assert!(view.functions.editor_hidden));
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.simulate_keystrokes("escape");
        view.read_with(cx, |view, cx| {
            assert!(!view.functions.open);
            assert_eq!(view.functions.text.read(cx).value(), "名=Alice\n姓=Smith");
        });
    }

    #[gpui::test]
    fn authorization_query_opens_prefilled_form_and_requires_confirmation(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        ready(&view, cx);
        cx.update(|window, cx| view.update(cx, |view, cx| {
            view.select_function(OPERATIONS.iter().find(|spec| spec.id == "authorization-list").unwrap(), window, cx);
            view.apply_function_event(FunctionEvent::State {task:31,record:json!({
                "operation_id":"apps", "method":"querySignInWithAppleApps", "status":"completed",
                "result":{"apple_id":"cloud@example.test","apps":[{"name":"Vinted","bundle_id":"fr.vinted","client_id":"fr.vinted.app"}]}
            })}, cx);
        }));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("function-query-content").is_some());
        assert!(cx.debug_bounds("function-result-text").is_none());
        while peer.commands.try_recv().is_ok() {}
        click(cx, "apple-stop-authorization-0");
        view.read_with(cx, |view, cx| {
            assert_eq!(view.functions.selected.unwrap().id, "authorization-stop");
            assert_eq!(view.functions.text.read(cx).value(), "App=fr.vinted");
            assert!(!view.functions.busy);
        });
        assert_no_function_commands(&mut peer);
        click(cx, "function-execute");
        cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
        assert_no_function_commands(&mut peer);
        cx.update(|window, cx| {
            window.dispatch_action(Box::new(gpui_component::dialog::Cancel), cx)
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
        assert_no_function_commands(&mut peer);
        click(cx, "function-execute");
        cx.update(|window, cx| {
            window.dispatch_action(
                Box::new(gpui_component::dialog::Confirm { secondary: false }),
                cx,
            )
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let mut invokes = 0;
        while let Ok(command) = peer.commands.try_recv() {
            if let rv_session::SessionCommand::Function(FunctionCommand::Scoped {
                command, ..
            }) = command
            {
                match *command {
                    FunctionCommand::InvokeForAccount { id, text, apple_id } => {
                        assert_eq!(id, "authorization-stop");
                        assert_eq!(text, "App=fr.vinted");
                        assert_eq!(apple_id, "cloud@example.test");
                        invokes += 1;
                    }
                    other => panic!("unexpected {other:?}"),
                }
            }
        }
        assert_eq!(invokes, 1);
    }

    #[gpui::test]
    fn device_query_marks_current_and_opens_removal_form(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        ready(&view, cx);
        cx.update(|window, cx| view.update(cx, |view, cx| {
            view.select_function(OPERATIONS.iter().find(|spec| spec.id == "device-list").unwrap(), window, cx);
            view.apply_function_event(FunctionEvent::State {task:31,record:json!({
                "operation_id":"devices", "method":"queryICloudDevices", "status":"completed",
                "result":{"apple_id":"cloud@example.test","devices":[{"name":"iPhone","model":"iPhone 15","current_device":true},
                    {"name":"Old iPhone","model":"iPhone 12","current_device":false}]}
            })}, cx);
        }));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        while peer.commands.try_recv().is_ok() {}
        click(cx, "apple-remove-other-devices");
        view.read_with(cx, |view, _| {
            assert_eq!(view.functions.selected.unwrap().id, "device-remove")
        });
        assert_no_function_commands(&mut peer);
        click(cx, "function-execute");
        cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
        assert_no_function_commands(&mut peer);
    }

    #[gpui::test]
    fn old_query_cannot_open_a_followup_for_a_new_account(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        ready(&view, cx);
        cx.update(|window, cx| view.update(cx, |view, cx| {
            view.select_function(OPERATIONS.iter().find(|spec| spec.id == "authorization-list").unwrap(), window, cx);
            view.apply_function_event(FunctionEvent::State {task:31,record:json!({
                "operation_id":"apps", "method":"querySignInWithAppleApps", "status":"completed",
                "result":{"apple_id":"previous@example.test","apps":[{"name":"Vinted","bundle_id":"fr.vinted"}]}
            })}, cx);
        }));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        while peer.commands.try_recv().is_ok() {}
        assert!(cx.debug_bounds("apple-query-stale").is_some());
        click(cx, "apple-stop-authorization-0");
        view.read_with(cx, |view, _| {
            assert_eq!(view.functions.selected.unwrap().id, "authorization-list")
        });
        assert_no_function_commands(&mut peer);
        // Also guard the handler itself, independent of the disabled button.
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.open_apple_followup("authorization-stop", "App=fr.vinted".into(), window, cx);
                assert_eq!(view.functions.selected.unwrap().id, "authorization-list");
            })
        });
        assert_no_function_commands(&mut peer);
    }

    #[gpui::test]
    fn manual_destructive_form_binds_the_confirmed_account(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        ready(&view, cx);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.select_function(
                    OPERATIONS
                        .iter()
                        .find(|spec| spec.id == "device-remove")
                        .unwrap(),
                    window,
                    cx,
                );
            })
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        while peer.commands.try_recv().is_ok() {}
        click(cx, "function-execute");
        cx.update(|window, cx| {
            window.dispatch_action(
                Box::new(gpui_component::dialog::Confirm { secondary: false }),
                cx,
            )
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let mut count = 0;
        while let Ok(command) = peer.commands.try_recv() {
            if let rv_session::SessionCommand::Function(FunctionCommand::Scoped {
                command, ..
            }) = command
            {
                assert!(
                    matches!(*command, FunctionCommand::InvokeForAccount {id, apple_id, ..}
                    if id == "device-remove" && apple_id == "cloud@example.test")
                );
                count += 1;
            }
        }
        assert_eq!(count, 1);
    }

    #[gpui::test]
    fn email_recovery_controls_fit_inside_a_narrow_panel(cx: &mut TestAppContext) {
        let (view, cx, _) = setup(cx, false);
        cx.simulate_resize(size(px(640.), px(400.)));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preview_function("email-unknown", window, cx)
            });
            window.draw(cx).clear(cx);
        });
        let panel = cx.debug_bounds("function-editor").unwrap();
        for selector in [
            "function-back",
            "function-query-resume",
            "function-email-rebind",
        ] {
            let bounds = cx.debug_bounds(selector).unwrap();
            assert!(
                bounds.left() >= panel.left() && bounds.right() <= panel.right(),
                "{selector}: {bounds:?}"
            );
            assert!(bounds.bottom() <= panel.bottom(), "{selector}: {bounds:?}");
        }
    }

    #[gpui::test]
    fn followup_restores_existing_draft_and_changed_confirmation_cannot_submit(
        cx: &mut TestAppContext,
    ) {
        let (view, cx, mut peer) = setup(cx, false);
        ready(&view, cx);
        cx.update(|window, cx| view.update(cx, |view, cx| {
            view.select_function(OPERATIONS.iter().find(|spec| spec.id == "authorization-stop").unwrap(), window, cx);
            view.functions.text.update(cx, |input, cx| input.set_value("App=old.example", window, cx));
            view.functions.expected_account = Some("cloud@example.test".into());
            view.select_function(OPERATIONS.iter().find(|spec| spec.id == "authorization-list").unwrap(), window, cx);
            view.apply_function_event(FunctionEvent::State {task:31,record:json!({
                "operation_id":"apps", "method":"querySignInWithAppleApps", "status":"completed",
                "result":{"apple_id":"cloud@example.test","apps":[{"name":"Vinted","bundle_id":"fr.vinted"}]}
            })}, cx);
        }));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        while peer.commands.try_recv().is_ok() {}
        click(cx, "apple-stop-authorization-0");
        view.read_with(cx, |view, cx| {
            assert_eq!(view.functions.text.read(cx).value(), "App=old.example");
            assert_eq!(
                view.functions.expected_account.as_deref(),
                Some("cloud@example.test")
            );
        });
        click(cx, "function-execute");
        cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
        view.update(cx, |view, cx| {
            let mut state = view.functions.account_state.clone().unwrap();
            for domain in ["identity", "icloud"] {
                state[domain]["account"]["apple_id"] = json!("new@example.test");
            }
            view.apply_function_event(FunctionEvent::AccountState(Ok(state)), cx);
        });
        cx.update(|window, cx| {
            window.dispatch_action(
                Box::new(gpui_component::dialog::Confirm { secondary: false }),
                cx,
            )
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_no_function_commands(&mut peer);
        view.read_with(cx, |view, _| assert!(!view.functions.busy));
    }
}
