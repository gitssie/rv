use super::mailbox_service::{self, MailboxDraft, Request, parse_mailbox_text};
use super::*;
use gpui_component::{
    WindowExt,
    alert::Alert,
    button::Button,
    checkbox::Checkbox,
    form::{Field, Form},
    input::{Input, Textarea, TextareaState},
    radio::{Radio, RadioGroup},
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum MailboxActivity {
    Load,
    Save,
    Delete,
    Fetch,
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
    activity: Option<MailboxActivity>,
    failed: bool,
    code_input: Entity<InputState>,
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
                this.mailbox.failed = false;
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
                    .rows(4)
                    .placeholder("粘贴邮箱资料")
            }),
            save_on_fetch: true,
            busy: false,
            activity: None,
            failed: false,
            code_input: cx.new(|cx| InputState::new(window, cx)),
            code: None,
            message: None,
            email: cx.new(|cx| InputState::new(window, cx).placeholder("name@outlook.com")),
            password: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder("邮箱密码")
            }),
            client_id: cx.new(|cx| InputState::new(window, cx).placeholder("填写客户端 ID")),
            refresh_token: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder("填写刷新令牌")
            }),
        }
    }
}

impl SessionView {
    pub(super) fn toggle_mailbox_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mailbox.open {
            self.close_mailbox_panel(window, cx);
            return;
        }
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
            self.focus_mailbox_input(window, cx);
        }
        cx.notify();
    }

    fn focus_mailbox_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        match (self.mailbox.tab, self.mailbox.source) {
            (MailboxTab::Accounts, _) => self
                .mailbox
                .email
                .update(cx, |input, cx| input.focus(window, cx)),
            (MailboxTab::Fetch, MailboxSource::Saved) => {
                self.mailbox
                    .select
                    .update(cx, |select, cx| select.focus(window, cx));
            }
            _ => self
                .mailbox
                .credentials_text
                .update(cx, |input, cx| input.focus(window, cx)),
        }
    }
    fn close_mailbox_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mailbox.open = false;
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn run_mailbox_request(&mut self, request: Request, cx: &mut Context<Self>) {
        if self.mailbox.busy {
            return;
        }
        let clear_form = matches!(request, Request::Save(_));
        let loading_accounts = matches!(request, Request::Load);
        self.mailbox.failed = false;
        self.mailbox.activity = Some(match &request {
            Request::Load => MailboxActivity::Load,
            Request::Save(_) => MailboxActivity::Save,
            Request::Delete(_) => MailboxActivity::Delete,
            Request::Fetch { .. } => MailboxActivity::Fetch,
        });
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
                this.mailbox.activity = None;
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
                        this.mailbox.failed = outcome.failed;
                        if let Some(code) = outcome.code {
                            this.mailbox.code_input.update(cx, |input, cx| {
                                input.set_value(code.1.clone(), window, cx)
                            });
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
                        this.mailbox.failed = true;
                        this.mailbox.message = Some(message.into());
                    }
                }
                if loading_accounts && this.mailbox.open {
                    this.focus_mailbox_input(window, cx);
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
                    self.mailbox.failed = true;
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
                    self.mailbox.failed = true;
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

    pub(super) fn render_mailbox_panel(
        &self,
        width: f32,
        height: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        crate::ui::tool_panel(cx)
            .id("mailbox-panel")
            .debug_selector(|| "mailbox-panel".into())
            .absolute()
            .top(px(8.))
            .right(px(8.))
            .w(px(360f32.min((width - 16.).max(100.))))
            .max_h(px((height - 16.).max(100.)))
            .overflow_y_scroll()
            .flex_shrink_0()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                crate::ui::tool_actions()
                    .gap_1()
                    .child(
                        TabBar::new("mailbox-tabs")
                            .underline()
                            .flex_shrink_0()
                            .selected_index(if self.mailbox.tab == MailboxTab::Fetch {
                                0
                            } else {
                                1
                            })
                            .children(
                                [
                                    ("获取邮件", IconName::Inbox),
                                    ("邮箱列表", IconName::Folder),
                                ]
                                .map(|(label, icon)| {
                                    Tab::new().aria_label(label).child(
                                        h_flex()
                                            .gap_2()
                                            .items_center()
                                            .child(Icon::new(icon))
                                            .child(label),
                                    )
                                }),
                            )
                            .on_click(cx.listener(|this, index: &usize, window, cx| {
                                this.mailbox.tab = if *index == 0 {
                                    MailboxTab::Fetch
                                } else {
                                    MailboxTab::Accounts
                                };
                                this.focus_mailbox_input(window, cx);
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        crate::ui::icon_button("mailbox-close", IconName::Close, "关闭邮箱")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_mailbox_panel(window, cx)
                            })),
                    ),
            )
            .child(match self.mailbox.tab {
                MailboxTab::Fetch => self.render_mail_fetch(cx),
                MailboxTab::Accounts => self.render_mail_accounts(cx),
            })
            .when(self.mailbox.busy, |panel| {
                panel.child(crate::ui::loading(
                    "mailbox-loading",
                    "正在处理…",
                    self.mailbox.message.clone().unwrap_or_default().into(),
                    cx,
                ))
            })
            .when(!self.mailbox.busy, |panel| {
                panel.when_some(self.mailbox.message.clone(), |panel, message| {
                    panel.child(if self.mailbox.failed {
                        Alert::error("mailbox-status", message)
                    } else {
                        Alert::success("mailbox-status", message)
                    })
                })
            })
    }

    fn render_mail_fetch(&self, cx: &mut Context<Self>) -> AnyElement {
        crate::ui::tool_section()
            .child(
                RadioGroup::horizontal("mailbox-source")
                    .disabled(self.mailbox.busy)
                    .selected_index(Some(if self.mailbox.source == MailboxSource::Saved {
                        0
                    } else {
                        1
                    }))
                    .children(
                        [
                            ("mailbox-source-saved", "选择邮箱"),
                            ("mailbox-source-pasted", "粘贴资料"),
                        ]
                        .map(|(id, label)| Radio::new(id).label(label)),
                    )
                    .on_click(cx.listener(|this, index: &usize, window, cx| {
                        if this.mailbox.busy {
                            return;
                        }
                        this.mailbox.source = if *index == 0 {
                            MailboxSource::Saved
                        } else {
                            MailboxSource::Pasted
                        };
                        this.mailbox.code = None;
                        this.mailbox.message = None;
                        this.mailbox.failed = false;
                        this.focus_mailbox_input(window, cx);
                        cx.notify();
                    })),
            )
            .when(self.mailbox.source == MailboxSource::Saved, |panel| {
                panel
                    .child(
                        Form::vertical().child(
                            Field::new().label("邮箱").child(
                                Select::new(&self.mailbox.select)
                                    .accessibility_label("选择邮箱")
                                    .placeholder("请选择邮箱")
                                    .disabled(
                                        self.mailbox.accounts.is_empty() || self.mailbox.busy,
                                    ),
                            ),
                        ),
                    )
                    .when(
                        self.mailbox.accounts.is_empty() && !self.mailbox.busy,
                        |panel| {
                            panel.child(crate::ui::empty_state(
                                "暂无邮箱，可切换到粘贴资料获取邮件。",
                                cx,
                            ))
                        },
                    )
            })
            .when(self.mailbox.source == MailboxSource::Pasted, |panel| {
                panel.child(
                    Form::vertical().child(
                        Field::new()
                            .label("邮箱资料")
                            .description("格式：邮箱----密码----客户端ID----刷新令牌")
                            .child(
                                div()
                                    .w_full()
                                    .min_w_0()
                                    .debug_selector(|| "mailbox-input-area".into())
                                    .child(
                                        Textarea::new(&self.mailbox.credentials_text)
                                            .aria_label("邮箱资料")
                                            .readonly(self.mailbox.busy)
                                            .h(px(100.)),
                                    ),
                            ),
                    ),
                )
            })
            .child(
                crate::ui::tool_actions()
                    .when(self.mailbox.source == MailboxSource::Pasted, |row| {
                        row.child(
                            Checkbox::new("mailbox-save-on-fetch")
                                .checked(self.mailbox.save_on_fetch)
                                .disabled(self.mailbox.busy)
                                .label("保存邮箱")
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    this.mailbox.save_on_fetch = *checked;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        Button::new("mailbox-fetch-code")
                            .primary()
                            .loading(self.mailbox.activity == Some(MailboxActivity::Fetch))
                            .label("获取邮件")
                            .disabled(
                                self.mailbox.busy
                                    || (self.mailbox.source == MailboxSource::Saved
                                        && self.mailbox.accounts.is_empty()),
                            )
                            .on_click(
                                cx.listener(|this, _, window, cx| this.fetch_mail_code(window, cx)),
                            ),
                    ),
            )
            .when_some(self.mailbox.code.clone(), |panel, (email, code)| {
                panel.child(
                    crate::ui::tool_section()
                        .child(crate::typography::caption(email, cx).text_ellipsis())
                        .child(
                            crate::ui::tool_actions()
                                .child(crate::typography::label("验证码", cx))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(Input::new(&self.mailbox.code_input).readonly(true)),
                                )
                                .child(
                                    Button::new("mailbox-copy-code")
                                        .ghost()
                                        .icon(IconName::Copy)
                                        .accessibility_label("复制验证码")
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                code.clone(),
                                            ))
                                        }),
                                ),
                        ),
                )
            })
            .into_any_element()
    }

    fn render_mail_accounts(&self, cx: &mut Context<Self>) -> AnyElement {
        crate::ui::tool_section()
            .child(div().text_sm().font_semibold().child("添加邮箱"))
            .child(
                Form::vertical()
                    .child(
                        Field::new()
                            .label("邮箱")
                            .child(Input::new(&self.mailbox.email).readonly(self.mailbox.busy)),
                    )
                    .child(
                        Field::new()
                            .label("邮箱密码")
                            .child(Input::new(&self.mailbox.password).readonly(self.mailbox.busy)),
                    )
                    .child(
                        Field::new()
                            .label("客户端 ID")
                            .child(Input::new(&self.mailbox.client_id).readonly(self.mailbox.busy)),
                    )
                    .child(Field::new().label("刷新令牌").child(
                        Input::new(&self.mailbox.refresh_token).readonly(self.mailbox.busy),
                    )),
            )
            .child(
                crate::ui::tool_actions().justify_end().child(
                    Button::new("mailbox-save-account")
                        .primary()
                        .label("保存邮箱")
                        .loading(self.mailbox.activity == Some(MailboxActivity::Save))
                        .disabled(self.mailbox.busy)
                        .on_click(cx.listener(|this, _, window, cx| this.save_mailbox(window, cx))),
                ),
            )
            .child(div().text_sm().font_semibold().child("已保存邮箱"))
            .children(self.mailbox.accounts.iter().map(|account| {
                let id = account.id.clone();
                let email = account.email.clone();
                crate::ui::tool_actions()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .text_ellipsis()
                            .child(email.clone()),
                    )
                    .child(
                        Button::new(format!("mailbox-delete-{id}"))
                            .ghost()
                            .icon(IconName::Delete)
                            .accessibility_label(format!("删除 {email}"))
                            .disabled(self.mailbox.busy)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm_delete_mailbox(id.clone(), email.clone(), window, cx)
                            })),
                    )
            }))
            .when(self.mailbox.accounts.is_empty(), |panel| {
                panel.child(crate::ui::empty_state("暂无邮箱", cx))
            })
            .into_any_element()
    }
}
