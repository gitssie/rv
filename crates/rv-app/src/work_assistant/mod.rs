//! Local workspace. It emits explicit phone commands; it never owns a VNC session.
mod drafts;
mod images;
mod notifications;
mod persistence;
mod templates;
mod widgets;
use crate::theme;
use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::{
    Disableable, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{InputEvent, InputState, TextareaState},
    v_flex,
};
use rv_core::assistant::{
    AssistantService, Draft, DraftImage, Template, format_price, parse_price,
};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub enum AssistantEvent {
    Close,
    SendText(String),
    Import(Vec<DraftImage>),
    RetryImport(Vec<DraftImage>),
    CancelImport,
    RefreshNotifications,
    Subscribe(bool),
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Drafts,
    Templates,
    Notifications,
}
pub struct WorkAssistant {
    pub service: AssistantService,
    tab: Tab,
    drafts: Vec<Draft>,
    draft: Option<Draft>,
    images: Vec<DraftImage>,
    templates: Vec<Template>,
    title: Entity<InputState>,
    description: Entity<TextareaState>,
    price: Entity<InputState>,
    stock: Entity<InputState>,
    template_name: Entity<InputState>,
    template_content: Entity<TextareaState>,
    template_category: Entity<InputState>,
    template: Template,
    busy: bool,
    save_busy: bool,
    pending_save: Option<persistence::PendingSave>,
    price_invalid: bool,
    dirty: bool,
    edit_time: Instant,
    error: Option<String>,
    save_error: Option<String>,
    message: String,
    pub connected: bool,
    pub writable: bool,
    pub import_busy: bool,
    pub notifications_supported: bool,
    pub notifications_enabled: bool,
    snapshot: Option<rv_session::NotificationSnapshot>,
    vinted_only: bool,
    confirm_delete: bool,
    pub retry_import_available: bool,
    confirm_import_retry: bool,
    confirm_template_delete: bool,
    _subscriptions: Vec<Subscription>,
    focus: FocusHandle,
}
impl EventEmitter<AssistantEvent> for WorkAssistant {}
impl WorkAssistant {
    pub fn new(service: AssistantService, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let field = |placeholder: &str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder.to_owned()))
        };
        let title = field("商品标题", window, cx);
        let price = field("18.00", window, cx);
        let stock = field("例如 A-023，仅保存在本地", window, cx);
        let description = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 6)
                .placeholder("商品描述")
        });
        let template_name = field("文案名称", window, cx);
        let template_category = field("分类，例如：议价回复", window, cx);
        let template_content = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 6)
                .placeholder("常用文案内容")
        });
        let mut subscriptions = Vec::new();
        for input in [&title, &price, &stock] {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                |this, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.capture(cx);
                        cx.notify();
                    }
                },
            ));
        }
        subscriptions.push(cx.subscribe_in(
            &description,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.capture(cx);
                    cx.notify();
                }
            },
        ));
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        if this.dirty
                            && !this.save_busy
                            && !this.busy
                            && this.save_error.is_none()
                            && this.edit_time.elapsed() >= Duration::from_millis(500)
                        {
                            this.save(cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        Self {
            service,
            tab: Tab::Drafts,
            drafts: vec![],
            draft: None,
            images: vec![],
            templates: vec![],
            title,
            description,
            price,
            stock,
            template_name,
            template_content,
            template_category,
            template: Template::default(),
            busy: false,
            save_busy: false,
            pending_save: None,
            price_invalid: false,
            dirty: false,
            edit_time: Instant::now(),
            error: None,
            save_error: None,
            message: "正在载入本地资料…".into(),
            connected: false,
            writable: false,
            import_busy: false,
            notifications_supported: false,
            notifications_enabled: true,
            snapshot: None,
            vinted_only: false,
            confirm_delete: false,
            retry_import_available: false,
            confirm_import_retry: false,
            confirm_template_delete: false,
            _subscriptions: subscriptions,
            focus: cx.focus_handle(),
        }
    }
    pub fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.save_busy {
            return;
        }
        self.busy = true;
        let service = self.service.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { service.call(|s| Ok((s.list_drafts()?, s.templates()?))) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok((drafts, templates)) => {
                        this.drafts = drafts;
                        this.templates = templates;
                        this.error = None;
                        this.save_error = None;
                        this.message = "本地资料已载入".into();
                        if let Some(d) = this.drafts.first().cloned() {
                            this.select_draft(d, window, cx);
                        } else {
                            this.draft = None;
                            this.images.clear();
                        }
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub fn set_connection(
        &mut self,
        connected: bool,
        writable: bool,
        import_busy: bool,
        notifications_supported: bool,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if (
            self.connected,
            self.writable,
            self.import_busy,
            self.notifications_supported,
            self.notifications_enabled,
        ) != (
            connected,
            writable,
            import_busy,
            notifications_supported,
            enabled,
        ) {
            self.connected = connected;
            self.writable = writable;
            self.import_busy = import_busy;
            self.notifications_supported = notifications_supported;
            self.notifications_enabled = enabled;
            cx.notify();
        }
    }
    pub fn receive_notifications(
        &mut self,
        snapshot: rv_session::NotificationSnapshot,
        cx: &mut Context<Self>,
    ) {
        self.snapshot = Some(snapshot);
        cx.notify();
    }
    pub fn feedback(&mut self, message: String, cx: &mut Context<Self>) {
        self.message = message;
        cx.notify();
    }
    pub fn flush(&mut self, cx: &mut Context<Self>) {
        if self.dirty && !self.save_busy {
            self.save(cx);
        }
    }
    fn capture(&mut self, cx: &App) {
        if let Some(d) = &mut self.draft {
            d.title = self.title.read(cx).value().to_string();
            d.description = self.description.read(cx).value().to_string();
            d.stock_note = self.stock.read(cx).value().to_string();
            match parse_price(self.price.read(cx).value().as_ref()) {
                Ok(price) => {
                    d.price_minor = price;
                    d.price_input = None;
                    self.price_invalid = false;
                }
                Err(_) => {
                    self.price_invalid = true;
                    d.price_input = Some(self.price.read(cx).value().to_string());
                }
            }
            self.dirty = true;
            self.edit_time = Instant::now();
        }
    }
    fn send(&mut self, text: String, cx: &mut Context<Self>) {
        if self.connected && self.writable && !text.is_empty() {
            cx.emit(AssistantEvent::SendText(text));
        }
    }
}
impl Drop for WorkAssistant {
    fn drop(&mut self) {
        if (self.dirty || self.pending_save.is_some())
            && let Some(draft) = self.draft.clone()
        {
            if let Some(pending) = &self.pending_save {
                pending.close(draft, &self.service);
            } else {
                persistence::persist(&self.service, draft, None);
            }
        }
    }
}
impl Render for WorkAssistant {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("work-assistant")
            .debug_selector(|| "work-assistant".into())
            .size_full()
            .bg(theme::card(cx))
            .text_color(theme::ink(cx))
            .text_sm()
            .track_focus(&self.focus)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                h_flex()
                    .p_3()
                    .gap_2()
                    .child(div().flex_1().font_semibold().child("工作助手"))
                    .child(
                        Button::new("assistant-close")
                            .debug_selector(|| "assistant-close".into())
                            .ghost()
                            .small()
                            .label("×")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.flush(cx);
                                cx.emit(AssistantEvent::Close);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .px_3()
                    .gap_1()
                    .border_b_1()
                    .border_color(theme::line(cx))
                    .children(
                        [
                            (Tab::Drafts, "商品草稿"),
                            (Tab::Templates, "常用文案"),
                            (Tab::Notifications, "通知"),
                        ]
                        .map(|(tab, label)| {
                            Button::new(label)
                                .ghost()
                                .small()
                                .selected(self.tab == tab)
                                .label(label)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.tab = tab;
                                    this.confirm_delete = false;
                                    this.confirm_template_delete = false;
                                    cx.notify();
                                }))
                        }),
                    ),
            )
            .child(
                v_flex()
                    .id("assistant-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_3()
                    .gap_3()
                    .child(match self.tab {
                        Tab::Drafts => self.render_drafts(cx),
                        Tab::Templates => self.render_templates(cx),
                        Tab::Notifications => self.render_notifications(cx),
                    }),
            )
            .child(
                v_flex()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(theme::line(cx))
                    .gap_1()
                    .child(
                        div()
                            .whitespace_normal()
                            .text_xs()
                            .text_color(
                                if self.error.is_some()
                                    || self.save_error.is_some()
                                    || self.price_invalid
                                {
                                    theme::danger(cx)
                                } else {
                                    theme::muted(cx)
                                },
                            )
                            .child(if self.price_invalid {
                                "价格无效；原始输入与其他字段会保存在本地，修正后才能发送价格"
                                    .into()
                            } else {
                                self.save_error
                                    .clone()
                                    .or_else(|| self.error.clone())
                                    .unwrap_or_else(|| {
                                        if self.save_busy {
                                            "保存中…".into()
                                        } else if self.dirty {
                                            "有未保存修改".into()
                                        } else {
                                            self.message.clone()
                                        }
                                    })
                            }),
                    )
                    .when(self.error.is_some() || self.save_error.is_some(), |el| {
                        el.child(
                            Button::new("assistant-reload")
                                .outline()
                                .small()
                                .label("重新载入（放弃未保存修改）")
                                .disabled(self.busy || self.save_busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.dirty = false;
                                    this.error = None;
                                    this.save_error = None;
                                    this.load(window, cx);
                                })),
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
impl WorkAssistant {
    pub(crate) fn notification_snapshot(&self) -> Option<&rv_session::NotificationSnapshot> {
        self.snapshot.as_ref()
    }
}
