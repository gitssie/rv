use super::mailbox_service::{self, MailboxDraft, Request, parse_mailbox_text};
use super::*;
use gpui_component::{
    WindowExt,
    button::Button,
    checkbox::Checkbox,
    form::{Field, Form},
    group_box::{GroupBox, GroupBoxVariants as _},
    input::{Input, Textarea, TextareaState},
    scroll::ScrollableElement,
    select::{Select, SelectEvent, SelectItem, SelectState},
    tab::{Tab, TabBar},
};
use rv_core::workspace::MailboxAccount;

#[derive(Clone, Copy, PartialEq, Eq)]
enum MailboxTab {
    Fetch,
    Accounts,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MailboxSource {
    Saved,
    Pasted,
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
    source: MailboxSource,
    accounts: Vec<MailboxAccount>,
    select: Entity<SelectState<Vec<MailboxChoice>>>,
    credentials_text: Entity<TextareaState>,
    save_on_fetch: bool,
    busy: bool,
    code: Option<(String, String)>,
    _select_subscription: Subscription,
    message: Option<String>,
    email: Entity<InputState>,
    password: Entity<InputState>,
    client_id: Entity<InputState>,
    refresh_token: Entity<InputState>,
}

impl MailboxUI {
    pub fn new(window: &mut Window, cx: &mut Context<SessionView>) -> Self {
        let select = cx.new(|cx| SelectState::new(Vec::<MailboxChoice>::new(), None, window, cx));
        let subscription = cx.subscribe(
            &select,
            |this, _, _: &SelectEvent<Vec<MailboxChoice>>, cx| {
                this.mailbox.code = None;
                this.mailbox.message = None;
                cx.notify();
            },
        );
        Self {
            _select_subscription: subscription,
            open: false,
            tab: MailboxTab::Fetch,
            source: MailboxSource::Saved,
            accounts: Vec::new(),
            select,
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

impl SessionView {
    pub(super) fn toggle_mailbox_panel(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.mailbox.open = !self.mailbox.open;
        if self.mailbox.open {
            let released = self.keys.release_all();
            self.send_keys(released);
            self.command_paste_pressed = false;
            self.functions.open = false;
            self.apps.picker_open = false;
            self.show_menu = false;
            self.session_menu = None;
            self.info_open = false;
            self.notifications.open = false;
            self.run_mailbox_request(Request::Load, cx);
        }
        cx.notify();
    }

    fn run_mailbox_request(&mut self, request: Request, cx: &mut Context<Self>) {
        if self.mailbox.busy {
            return;
        }
        let clear_form = matches!(request, Request::Save(_));
        if matches!(request, Request::Fetch { .. }) {
            self.mailbox.code = None;
        }
        self.mailbox.busy = true;
        self.mailbox.message = Some(
            match &request {
                Request::Fetch { .. } => "正在获取邮件验证码…",
                Request::Load => "正在加载邮箱…",
                Request::Save(_) => "正在保存邮箱…",
                Request::Delete(_) => "正在删除邮箱…",
            }
            .into(),
        );
        let paths = self.store_paths.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { mailbox_service::run(paths, request) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.mailbox.busy = false;
                match result {
                    Ok(outcome) => {
                        if let Some(accounts) = outcome.accounts {
                            let selected = outcome
                                .selected
                                .or_else(|| this.mailbox.select.read(cx).selected_value().cloned())
                                .filter(|id| accounts.iter().any(|account| &account.id == id))
                                .or_else(|| accounts.first().map(|account| account.id.clone()));
                            let choices = accounts
                                .iter()
                                .map(|account| MailboxChoice {
                                    id: account.id.clone(),
                                    email: account.email.clone(),
                                })
                                .collect();
                            this.mailbox.select.update(cx, |select, cx| {
                                select.set_items(choices, window, cx);
                                if let Some(selected) = &selected {
                                    select.set_selected_value(selected, window, cx);
                                } else {
                                    select.set_selected_index(None, window, cx);
                                }
                            });
                            if accounts.is_empty() {
                                this.mailbox.source = MailboxSource::Pasted;
                            }
                            this.mailbox.accounts = accounts;
                        }
                        this.mailbox.message = outcome.message;
                        if let Some(code) = outcome.code {
                            this.mailbox.code = Some(code);
                        }
                        if clear_form {
                            for input in [
                                &this.mailbox.email,
                                &this.mailbox.password,
                                &this.mailbox.client_id,
                                &this.mailbox.refresh_token,
                            ] {
                                input.update(cx, |input, cx| input.set_value("", window, cx));
                            }
                        }
                    }
                    Err(message) => {
                        this.mailbox.message = Some(message.into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn save_mailbox(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let mailbox = MailboxDraft {
            email: self.mailbox.email.read(cx).value().trim().to_owned(),
            password: self.mailbox.password.read(cx).value().to_string(),
            client_id: self.mailbox.client_id.read(cx).value().trim().to_owned(),
            refresh_token: self
                .mailbox
                .refresh_token
                .read(cx)
                .value()
                .trim()
                .to_owned(),
        };
        self.run_mailbox_request(Request::Save(mailbox), cx);
    }

    fn delete_mailbox(&mut self, id: &str, _window: &mut Window, cx: &mut Context<Self>) {
        self.run_mailbox_request(Request::Delete(id.to_owned()), cx);
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

    fn fetch_mail_code(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.mailbox.busy {
            return;
        }
        let mailbox = match self.mailbox.source {
            MailboxSource::Saved => {
                let selected = self.mailbox.select.read(cx).selected_value();
                let Some(account) = self
                    .mailbox
                    .accounts
                    .iter()
                    .find(|account| Some(&account.id) == selected)
                else {
                    self.mailbox.message = Some("请选择邮箱".into());
                    cx.notify();
                    return;
                };
                MailboxDraft {
                    email: account.email.clone(),
                    password: account.password.clone(),
                    client_id: account.client_id.clone(),
                    refresh_token: account.refresh_token.clone(),
                }
            }
            MailboxSource::Pasted => {
                let text = self.mailbox.credentials_text.read(cx).value();
                let Ok((email, password, client_id, refresh_token)) = parse_mailbox_text(&text)
                else {
                    self.mailbox.message = Some("邮箱资料格式不正确，请按示例填写".into());
                    cx.notify();
                    return;
                };
                MailboxDraft {
                    email,
                    password,
                    client_id,
                    refresh_token,
                }
            }
        };
        self.run_mailbox_request(
            Request::Fetch {
                mailbox,
                save: self.mailbox.source == MailboxSource::Pasted && self.mailbox.save_on_fetch,
            },
            cx,
        );
    }

    pub(super) fn render_mailbox_launcher(&self, cx: &mut Context<Self>) -> impl IntoElement {
        crate::ui::icon_button("mailbox-open", IconName::Inbox, "邮箱")
            .selected(self.mailbox.open)
            .on_click(cx.listener(|this, _, window, cx| this.toggle_mailbox_panel(window, cx)))
    }

    pub(super) fn render_mailbox_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        crate::ui::tool_panel(cx)
            .id("mailbox-panel")
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .w_full()
            .max_w_full()
            .max_h_full()
            .overflow_y_scrollbar()
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .child(div().text_sm().font_semibold().child("邮箱"))
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
                TabBar::new("mailbox-source")
                    .selected_index(if self.mailbox.source == MailboxSource::Saved {
                        0
                    } else {
                        1
                    })
                    .children(["选择邮箱", "粘贴资料"].map(|label| Tab::new().child(label)))
                    .on_click(cx.listener(|this, index: &usize, _, cx| {
                        if !this.mailbox.busy {
                            this.mailbox.source = if *index == 0 {
                                MailboxSource::Saved
                            } else {
                                MailboxSource::Pasted
                            };
                            this.mailbox.code = None;
                            this.mailbox.message = None;
                            cx.notify();
                        }
                    })),
            )
            .child(
                Form::vertical()
                    .when(self.mailbox.source == MailboxSource::Saved, |form| {
                        form.child(
                            Field::new().label("已保存邮箱").child(
                                Select::new(&self.mailbox.select)
                                    .accessibility_label("选择邮箱")
                                    .placeholder("选择要读取的邮箱")
                                    .disabled(
                                        self.mailbox.accounts.is_empty() || self.mailbox.busy,
                                    ),
                            ),
                        )
                    })
                    .when(self.mailbox.source == MailboxSource::Pasted, |form| {
                        form.child(
                            Field::new().label("邮箱资料").child(
                                Textarea::new(&self.mailbox.credentials_text)
                                    .disabled(self.mailbox.busy),
                            ),
                        )
                        .child(
                            Field::new().child(
                                Checkbox::new("mailbox-save-on-fetch")
                                    .checked(self.mailbox.save_on_fetch)
                                    .disabled(self.mailbox.busy)
                                    .label("保存邮箱")
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.mailbox.save_on_fetch = *checked;
                                        cx.notify();
                                    })),
                            ),
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
                            .label("获取邮件")
                            .disabled(self.mailbox.busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.fetch_mail_code(window, cx)),
                            ),
                    ),
            )
            .when_some(self.mailbox.code.clone(), |el, (email, code)| {
                el.child(
                    GroupBox::new().outline().child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(crate::typography::caption(email, cx))
                                    .child(div().text_sm().font_semibold().child(code.clone())),
                            )
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
                                Field::new().label("邮箱").child(
                                    Input::new(&self.mailbox.email).disabled(self.mailbox.busy),
                                ),
                            )
                            .child(Field::new().label("邮箱密码").child(
                                Input::new(&self.mailbox.password).disabled(self.mailbox.busy),
                            ))
                            .child(Field::new().label("OAuth 客户端 ID").child(
                                Input::new(&self.mailbox.client_id).disabled(self.mailbox.busy),
                            ))
                            .child(Field::new().label("OAuth 刷新令牌").child(
                                Input::new(&self.mailbox.refresh_token).disabled(self.mailbox.busy),
                            )),
                    )
                    .child(
                        Button::new("mailbox-save-account")
                            .disabled(self.mailbox.busy)
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
                                    .disabled(self.mailbox.busy)
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
