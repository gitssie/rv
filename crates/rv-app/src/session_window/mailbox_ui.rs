use super::*;
use gpui_component::{
    WindowExt,
    button::Button,
    checkbox::Checkbox,
    form::{Field, Form},
    group_box::{GroupBox, GroupBoxVariants as _},
    input::{Input, Textarea, TextareaState},
    scroll::ScrollableElement,
    select::{Select, SelectItem, SelectState},
    tab::{Tab, TabBar},
};
use rv_core::workspace::{MailboxAccount, WorkspaceStore};
use rv_session::{FunctionCommand, FunctionEvent};

#[derive(Clone, Copy, PartialEq, Eq)]
enum MailboxTab {
    Fetch,
    Accounts,
}

#[derive(Clone)]
struct MailboxChoice {
    id: String,
    email: String,
}

impl SelectItem for MailboxChoice {
    type Value = String;

    fn title(&self) -> SharedString {
        self.email.clone().into()
    }

    fn value(&self) -> &String {
        &self.id
    }
}

pub(super) struct MailboxUI {
    pub(super) open: bool,
    tab: MailboxTab,
    accounts: Vec<MailboxAccount>,
    select: Entity<SelectState<Vec<MailboxChoice>>>,
    credentials_text: Entity<TextareaState>,
    save_on_fetch: bool,
    busy: bool,
    code: Option<String>,
    message: Option<String>,
    email: Entity<InputState>,
    password: Entity<InputState>,
    client_id: Entity<InputState>,
    refresh_token: Entity<InputState>,
}

impl MailboxUI {
    pub fn new(window: &mut Window, cx: &mut Context<SessionView>) -> Self {
        Self {
            open: false,
            tab: MailboxTab::Fetch,
            accounts: Vec::new(),
            select: cx.new(|cx| SelectState::new(Vec::<MailboxChoice>::new(), None, window, cx)),
            credentials_text: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .rows(5)
                    .placeholder("邮箱----密码----客户端ID----刷新令牌")
            }),
            save_on_fetch: true,
            busy: false,
            code: None,
            message: None,
            email: cx.new(|cx| InputState::new(window, cx).placeholder("name@outlook.com")),
            password: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder("邮箱密码")
            }),
            client_id: cx.new(|cx| InputState::new(window, cx).placeholder("OAuth 客户端 ID")),
            refresh_token: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder("OAuth 刷新令牌")
            }),
        }
    }
}

fn mailbox_validation_message(error: &str) -> String {
    if error.contains("尚未接入") {
        "暂不支持此邮箱自动获取验证码".into()
    } else {
        "邮箱资料不完整或格式有误，请检查后重试".into()
    }
}

fn mailbox_fetch_error_message(error: &str) -> String {
    if error.contains("超时")
        || error.contains("没有可用验证码")
        || error.contains("未收到新的邮箱验证码")
    {
        "暂时没有收到验证码，请稍后重试".into()
    } else if error.contains("账户与填写的邮箱不一致") {
        "邮箱资料与授权账户不一致，请检查邮箱地址".into()
    } else {
        "获取验证码失败，请检查邮箱资料后重试".into()
    }
}

impl SessionView {
    pub(super) fn toggle_mailbox_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mailbox.open = !self.mailbox.open;
        if self.mailbox.open {
            self.load_mailboxes(window, cx);
        }
        cx.notify();
    }

    fn load_mailboxes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mailbox.message = None;
        let result = self
            .store_paths
            .as_ref()
            .ok_or_else(|| "store paths unavailable".to_string())
            .and_then(|paths| {
                WorkspaceStore::open(paths)
                    .and_then(|store| store.mailbox_accounts())
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(accounts) => {
                let previous = self.mailbox.select.read(cx).selected_value().cloned();
                let selected = previous
                    .filter(|id| accounts.iter().any(|account| &account.id == id))
                    .or_else(|| accounts.first().map(|account| account.id.clone()));
                let choices = accounts
                    .iter()
                    .map(|account| MailboxChoice {
                        id: account.id.clone(),
                        email: account.email.clone(),
                    })
                    .collect();
                self.mailbox.select.update(cx, |select, cx| {
                    select.set_items(choices, window, cx);
                    if let Some(selected) = &selected {
                        select.set_selected_value(selected, window, cx);
                    } else {
                        select.set_selected_index(None, window, cx);
                    }
                });
                self.mailbox.accounts = accounts;
            }
            Err(_) => self.mailbox.message = Some("邮箱列表加载失败，请重试".into()),
        }
    }

    fn save_mailbox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let email = self.mailbox.email.read(cx).value().trim().to_owned();
        let password = self.mailbox.password.read(cx).value().to_string();
        let client_id = self.mailbox.client_id.read(cx).value().trim().to_owned();
        let refresh_token = self
            .mailbox
            .refresh_token
            .read(cx)
            .value()
            .trim()
            .to_owned();
        if let Err(error) =
            rv_session::functions::validate_mailbox_credentials(&email, &client_id, &refresh_token)
        {
            self.mailbox.message = Some(mailbox_validation_message(&error));
            cx.notify();
            return;
        }
        let Some(paths) = self.store_paths.as_ref() else {
            self.mailbox.message = Some("邮箱无法保存，请重试".into());
            cx.notify();
            return;
        };
        let result = WorkspaceStore::open(paths).and_then(|mut store| {
            store.save_mailbox_account(&email, &password, &client_id, &refresh_token)
        });
        match result {
            Ok(account) => {
                let selected = account.id;
                self.mailbox
                    .email
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.mailbox
                    .password
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.mailbox
                    .client_id
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.mailbox
                    .refresh_token
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.load_mailboxes(window, cx);
                self.mailbox.select.update(cx, |select, cx| {
                    select.set_selected_value(&selected, window, cx)
                });
                self.mailbox.message = Some("邮箱已保存".into());
            }
            Err(_) => self.mailbox.message = Some("邮箱保存失败，请检查资料后重试".into()),
        }
        cx.notify();
    }

    fn delete_mailbox(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let result = self
            .store_paths
            .as_ref()
            .ok_or_else(|| "store paths unavailable".to_string())
            .and_then(|paths| {
                WorkspaceStore::open(paths)
                    .and_then(|mut store| store.delete_mailbox_account(id))
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(_) => {
                self.load_mailboxes(window, cx);
                self.mailbox.message = Some("邮箱已删除".into());
            }
            Err(_) => self.mailbox.message = Some("邮箱删除失败，请重试".into()),
        }
        cx.notify();
    }

    fn confirm_delete_mailbox(
        &mut self,
        id: String,
        email: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entity = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let entity = entity.clone();
            let id = id.clone();
            alert
                .title("删除邮箱？")
                .description(format!("删除 {email} 后，它会从邮箱列表中移除。"))
                .confirm()
                .on_ok(move |_, window, cx| {
                    entity.update(cx, |this, cx| this.delete_mailbox(&id, window, cx));
                    true
                })
        });
    }

    fn fetch_mail_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mailbox.busy {
            return;
        }
        let pasted = self.mailbox.credentials_text.read(cx).value().to_string();
        let (text, selected_message) = if pasted.trim().is_empty() {
            let selected = self.mailbox.select.read(cx).selected_value().cloned();
            let Some(account) = selected
                .as_deref()
                .and_then(|id| {
                    self.mailbox
                        .accounts
                        .iter()
                        .find(|account| account.id == id)
                })
                .cloned()
            else {
                self.mailbox.message = Some("请选择已保存邮箱，或粘贴邮箱资料".into());
                cx.notify();
                return;
            };
            (format_mailbox_text(&account), None)
        } else {
            let Ok((email, password, client_id, refresh_token)) = parse_mailbox_text(&pasted)
            else {
                self.mailbox.message = Some("邮箱资料格式不正确，请按示例填写".into());
                cx.notify();
                return;
            };
            if let Err(error) = rv_session::functions::validate_mailbox_credentials(
                &email,
                &client_id,
                &refresh_token,
            ) {
                self.mailbox.message = Some(mailbox_validation_message(&error));
                cx.notify();
                return;
            }
            let message = if self.mailbox.save_on_fetch {
                let Some(paths) = self.store_paths.as_ref() else {
                    self.mailbox.message = Some("邮箱无法保存，请重试".into());
                    cx.notify();
                    return;
                };
                let result = WorkspaceStore::open(paths).and_then(|mut store| {
                    store.save_mailbox_account(&email, &password, &client_id, &refresh_token)
                });
                match result {
                    Ok(account) => {
                        let id = account.id;
                        self.load_mailboxes(window, cx);
                        self.mailbox
                            .select
                            .update(cx, |select, cx| select.set_selected_value(&id, window, cx));
                        Some("邮箱已保存，正在获取验证码…".to_owned())
                    }
                    Err(_) => {
                        self.mailbox.message = Some("邮箱保存失败，请检查资料后重试".into());
                        cx.notify();
                        return;
                    }
                }
            } else {
                None
            };
            (
                format!(
                    "邮箱={email}\n密码={password}\n客户端ID={client_id}\n刷新令牌={refresh_token}"
                ),
                message,
            )
        };
        self.mailbox.busy = true;
        self.mailbox.code = None;
        self.mailbox.message =
            Some(selected_message.unwrap_or_else(|| "正在获取邮件验证码…".into()));
        self.handle.function(FunctionCommand::Scoped {
            id: "mailbox-panel".into(),
            command: Box::new(FunctionCommand::ReadEmailCode { text }),
        });
        cx.notify();
    }

    pub(super) fn apply_mailbox_event(
        &mut self,
        event: FunctionEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let FunctionEvent::ForOperation { id, event } = event else {
            return false;
        };
        if id != "mailbox-panel" {
            return false;
        }
        match *event {
            FunctionEvent::Progress(message) => {
                self.mailbox.busy = true;
                self.mailbox.message = Some(message);
            }
            FunctionEvent::EmailCodeFetched(code) => {
                self.mailbox.busy = false;
                self.mailbox.code = Some(code);
                self.mailbox.message = Some("验证码已获取".into());
            }
            FunctionEvent::Error(error) | FunctionEvent::Uncertain(error) => {
                self.mailbox.busy = false;
                self.mailbox.message = Some(mailbox_fetch_error_message(&error));
            }
            _ => return true,
        }
        cx.notify();
        true
    }

    pub(super) fn render_mailbox_launcher(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Button::new("mailbox-open")
            .ghost()
            .icon(IconName::Inbox)
            .label("邮箱验证码与邮箱列表")
            .selected(self.mailbox.open)
            .on_click(cx.listener(|this, _, window, cx| this.toggle_mailbox_panel(window, cx)))
    }

    pub(super) fn render_mailbox_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        crate::ui::tool_panel(cx)
            .id("mailbox-panel")
            .occlude()
            .w_full()
            .max_w_full()
            .max_h(px(500.))
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .child(div().text_sm().font_semibold().child("邮箱验证码"))
                    .child(div().flex_1())
                    .child(
                        crate::ui::icon_button("mailbox-close", IconName::Close, "关闭面板")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.mailbox.open = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                TabBar::new("mailbox-tabs")
                    .underline()
                    .selected_index(if self.mailbox.tab == MailboxTab::Fetch {
                        0
                    } else {
                        1
                    })
                    .children(["获取邮件", "邮箱列表"].map(|label| Tab::new().child(label)))
                    .on_click(cx.listener(|this, index: &usize, _, cx| {
                        this.mailbox.tab = if *index == 0 {
                            MailboxTab::Fetch
                        } else {
                            MailboxTab::Accounts
                        };
                        cx.notify();
                    })),
            )
            .child(match self.mailbox.tab {
                MailboxTab::Fetch => self.render_mail_fetch(cx),
                MailboxTab::Accounts => self.render_mail_accounts(cx),
            })
    }

    fn render_mail_fetch(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .p_3()
            .gap_3()
            .child(
                div()
                    .text_xs()
                    .text_color(theme::muted(cx))
                    .child("选择已保存邮箱，或粘贴四段邮箱资料后获取最新邮件中的六位验证码。"),
            )
            .child(
                Form::vertical()
                    .child(
                        Field::new().label("已保存邮箱").child(
                            Select::new(&self.mailbox.select)
                                .accessibility_label("选择邮箱")
                                .placeholder("选择要读取的邮箱")
                                .disabled(self.mailbox.accounts.is_empty() || self.mailbox.busy),
                        ),
                    )
                    .child(
                        Field::new()
                            .label("邮箱资料")
                            .child(Textarea::new(&self.mailbox.credentials_text).overflow_hidden()),
                    )
                    .child(
                        Field::new().child(
                            Checkbox::new("mailbox-save-on-fetch")
                                .checked(self.mailbox.save_on_fetch)
                                .label("获取时保存邮箱资料（默认开启）")
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    this.mailbox.save_on_fetch = *checked;
                                    cx.notify();
                                })),
                        ),
                    ),
            )
            .child(
                crate::ui::tool_actions()
                    .justify_between()
                    .child(self.mailbox.message.clone().map_or_else(
                        || div().text_xs().text_color(theme::muted(cx)).child(" "),
                        |message| div().text_xs().text_color(theme::muted(cx)).child(message),
                    ))
                    .child(
                        Button::new("mailbox-fetch-code")
                            .primary()
                            .loading(self.mailbox.busy)
                            .label("获取验证码")
                            .disabled(self.mailbox.busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.fetch_mail_code(window, cx)),
                            ),
                    ),
            )
            .when_some(self.mailbox.code.clone(), |el, code| {
                el.child(
                    GroupBox::new().outline().child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_sm().font_semibold().child(code.clone()))
                            .child(div().flex_1())
                            .child(
                                Button::new("mailbox-copy-code")
                                    .ghost()
                                    .icon(IconName::Copy)
                                    .accessibility_label("复制验证码")
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            code.clone(),
                                        ));
                                    }),
                            ),
                    ),
                )
            })
            .into_any_element()
    }

    fn render_mail_accounts(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .p_3()
            .gap_3()
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .child(
                GroupBox::new()
                    .outline()
                    .child(div().text_sm().font_semibold().child("添加邮箱"))
                    .child(
                        Form::vertical()
                            .child(
                                Field::new()
                                    .label("邮箱")
                                    .child(Input::new(&self.mailbox.email)),
                            )
                            .child(
                                Field::new()
                                    .label("邮箱密码")
                                    .child(Input::new(&self.mailbox.password)),
                            )
                            .child(
                                Field::new()
                                    .label("OAuth 客户端 ID")
                                    .child(Input::new(&self.mailbox.client_id)),
                            )
                            .child(
                                Field::new()
                                    .label("OAuth 刷新令牌")
                                    .child(Input::new(&self.mailbox.refresh_token)),
                            ),
                    )
                    .child(
                        Button::new("mailbox-save-account")
                            .primary()
                            .label("保存邮箱")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.save_mailbox(window, cx)),
                            ),
                    ),
            )
            .child(
                GroupBox::new()
                    .outline()
                    .child(div().text_sm().font_semibold().child("已保存邮箱"))
                    .children(self.mailbox.accounts.iter().map(|account| {
                        let id = account.id.clone();
                        let email = account.email.clone();
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(div().flex_1().min_w_0().text_sm().child(email.clone()))
                            .child(
                                Button::new(format!("mailbox-delete-{id}"))
                                    .ghost()
                                    .icon(IconName::Delete)
                                    .accessibility_label(format!("删除 {}", account.email))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.confirm_delete_mailbox(
                                            id.clone(),
                                            email.clone(),
                                            window,
                                            cx,
                                        )
                                    })),
                            )
                    }))
                    .when(self.mailbox.accounts.is_empty(), |box_| {
                        box_.child(
                            div()
                                .p_2()
                                .text_sm()
                                .text_color(theme::muted(cx))
                                .child("暂无邮箱"),
                        )
                    }),
            )
            .when_some(self.mailbox.message.clone(), |el, message| {
                el.child(div().text_xs().text_color(theme::muted(cx)).child(message))
            })
            .into_any_element()
    }
}

fn parse_mailbox_text(text: &str) -> Result<(String, String, String, String), ()> {
    let text = text.trim();
    let (email, password, client_id, refresh_token) = if !text.contains(['\n', '\r']) {
        let fields: Vec<_> = text.split("----").map(str::trim).collect();
        if fields.len() != 4 {
            return Err(());
        }
        (fields[0], fields[1], fields[2], fields[3])
    } else {
        let mut email = None;
        let mut password = None;
        let mut client_id = None;
        let mut refresh_token = None;
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                return Err(());
            };
            let slot = match key.trim() {
                "邮箱" | "email" => &mut email,
                "密码" | "password" => &mut password,
                "客户端ID" | "client_id" => &mut client_id,
                "刷新令牌" | "refresh_token" => &mut refresh_token,
                _ => return Err(()),
            };
            if slot.replace(value.trim()).is_some() {
                return Err(());
            }
        }
        (
            email.ok_or(())?,
            password.unwrap_or_default(),
            client_id.ok_or(())?,
            refresh_token.ok_or(())?,
        )
    };
    if email.is_empty() || client_id.is_empty() || refresh_token.is_empty() {
        return Err(());
    }
    Ok((
        email.to_owned(),
        password.to_owned(),
        client_id.to_owned(),
        refresh_token.to_owned(),
    ))
}

fn format_mailbox_text(account: &MailboxAccount) -> String {
    format!(
        "邮箱={}\n密码={}\n客户端ID={}\n刷新令牌={}",
        account.email, account.password, account.client_id, account.refresh_token
    )
}

#[cfg(test)]
mod tests {
    use super::parse_mailbox_text;

    #[test]
    fn parses_four_part_and_named_mailbox_credentials() {
        assert_eq!(
            parse_mailbox_text("mail@example.test----pw----client----refresh").unwrap(),
            (
                "mail@example.test".into(),
                "pw".into(),
                "client".into(),
                "refresh".into()
            )
        );
        assert_eq!(
            parse_mailbox_text(
                "邮箱=mail@example.test\n密码=p----w\n客户端ID=client\n刷新令牌=refresh----part"
            )
            .unwrap(),
            (
                "mail@example.test".into(),
                "p----w".into(),
                "client".into(),
                "refresh----part".into()
            )
        );
    }
}
