use super::*;
use gpui_component::{
    WindowExt,
    button::Button,
    form::{Field, Form},
    group_box::{GroupBox, GroupBoxVariants as _},
    input::Input,
    scroll::ScrollableElement,
    tab::{Tab, TabBar},
};
use rv_core::workspace::{MailboxAccount, WorkspaceStore};
use rv_session::{FunctionCommand, FunctionEvent};

#[derive(Clone, Copy, PartialEq, Eq)]
enum MailboxTab {
    Fetch,
    Accounts,
}

pub(super) struct MailboxUI {
    pub(super) open: bool,
    tab: MailboxTab,
    accounts: Vec<MailboxAccount>,
    selected: Option<String>,
    busy: bool,
    code: Option<String>,
    message: Option<String>,
    email: Entity<InputState>,
    client_id: Entity<InputState>,
    refresh_token: Entity<InputState>,
}

impl MailboxUI {
    pub fn new(window: &mut Window, cx: &mut Context<SessionView>) -> Self {
        Self {
            open: false,
            tab: MailboxTab::Fetch,
            accounts: Vec::new(),
            selected: None,
            busy: false,
            code: None,
            message: None,
            email: cx.new(|cx| InputState::new(window, cx).placeholder("name@outlook.com")),
            client_id: cx.new(|cx| InputState::new(window, cx).placeholder("OAuth 客户端 ID")),
            refresh_token: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder("OAuth 刷新令牌")
            }),
        }
    }
}

impl SessionView {
    pub(super) fn toggle_mailbox_panel(&mut self, cx: &mut Context<Self>) {
        self.mailbox.open = !self.mailbox.open;
        if self.mailbox.open {
            self.load_mailboxes();
        }
        cx.notify();
    }

    fn load_mailboxes(&mut self) {
        self.mailbox.message = None;
        let result = self
            .store_paths
            .as_ref()
            .ok_or_else(|| "本地数据目录不可用".to_string())
            .and_then(|paths| {
                WorkspaceStore::open(paths)
                    .and_then(|store| store.mailbox_accounts())
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(accounts) => {
                if self
                    .mailbox
                    .selected
                    .as_ref()
                    .is_none_or(|id| !accounts.iter().any(|account| &account.id == id))
                {
                    self.mailbox.selected = accounts.first().map(|account| account.id.clone());
                }
                self.mailbox.accounts = accounts;
            }
            Err(error) => self.mailbox.message = Some(format!("读取邮箱列表失败：{error}")),
        }
    }

    fn save_mailbox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let email = self.mailbox.email.read(cx).value().trim().to_owned();
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
            self.mailbox.message = Some(error);
            cx.notify();
            return;
        }
        let Some(paths) = self.store_paths.as_ref() else {
            self.mailbox.message = Some("本地数据目录不可用".into());
            cx.notify();
            return;
        };
        let result = WorkspaceStore::open(paths)
            .and_then(|mut store| store.save_mailbox_account(&email, &client_id, &refresh_token));
        match result {
            Ok(account) => {
                self.mailbox.selected = Some(account.id);
                self.mailbox
                    .email
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.mailbox
                    .client_id
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.mailbox
                    .refresh_token
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.load_mailboxes();
                self.mailbox.message = Some("邮箱已保存；OAuth 凭据已加密存入本地 SQLite".into());
            }
            Err(error) => self.mailbox.message = Some(format!("保存失败：{error}")),
        }
        cx.notify();
    }

    fn delete_mailbox(&mut self, id: &str, cx: &mut Context<Self>) {
        let result = self
            .store_paths
            .as_ref()
            .ok_or_else(|| "本地数据目录不可用".to_string())
            .and_then(|paths| {
                WorkspaceStore::open(paths)
                    .and_then(|mut store| store.delete_mailbox_account(id))
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(_) => {
                self.load_mailboxes();
                self.mailbox.message = Some("邮箱已删除".into());
            }
            Err(error) => self.mailbox.message = Some(format!("删除失败：{error}")),
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
                .description(format!("删除 {email} 的本地 OAuth 凭据。"))
                .confirm()
                .on_ok(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.delete_mailbox(&id, cx));
                    true
                })
        });
    }

    fn fetch_mail_code(&mut self, cx: &mut Context<Self>) {
        if self.mailbox.busy {
            return;
        }
        let Some(account) = self.mailbox.selected.as_deref().and_then(|id| {
            self.mailbox
                .accounts
                .iter()
                .find(|account| account.id == id)
        }) else {
            self.mailbox.message = Some("请先选择邮箱".into());
            cx.notify();
            return;
        };
        let text = format!(
            "邮箱={}\n客户端ID={}\n刷新令牌={}",
            account.email, account.client_id, account.refresh_token
        );
        self.mailbox.busy = true;
        self.mailbox.code = None;
        self.mailbox.message = Some("正在读取最新邮件验证码…".into());
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
                self.mailbox.message = Some("已读取最新邮件中的验证码".into());
            }
            FunctionEvent::Error(error) | FunctionEvent::Uncertain(error) => {
                self.mailbox.busy = false;
                self.mailbox.message = Some(error);
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
            .on_click(cx.listener(|this, _, _, cx| this.toggle_mailbox_panel(cx)))
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
                    .child("从所选 Outlook / Hotmail 邮箱读取最新邮件中的六位验证码。"),
            )
            .child(
                GroupBox::new()
                    .outline()
                    .children(self.mailbox.accounts.iter().map(|account| {
                        let id = account.id.clone();
                        Button::new(format!("mailbox-select-{id}"))
                            .ghost()
                            .w_full()
                            .justify_start()
                            .label(account.email.clone())
                            .selected(self.mailbox.selected.as_deref() == Some(&id))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.mailbox.selected = Some(id.clone());
                                this.mailbox.code = None;
                                this.mailbox.message = None;
                                cx.notify();
                            }))
                    }))
                    .when(self.mailbox.accounts.is_empty(), |box_| {
                        box_.child(
                            div()
                                .p_2()
                                .text_sm()
                                .text_color(theme::muted(cx))
                                .child("还没有邮箱，请到“邮箱列表”添加。"),
                        )
                    }),
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
                            .disabled(self.mailbox.busy || self.mailbox.selected.is_none())
                            .on_click(cx.listener(|this, _, _, cx| this.fetch_mail_code(cx))),
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
