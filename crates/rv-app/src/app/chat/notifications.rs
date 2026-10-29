//! Local notification selection and read-only rendering. No HTTP or read receipts.
use super::*;
use crate::theme;
use gpui::prelude::FluentBuilder as _;
use gpui_component::{badge::Badge, h_flex, list::ListItem, spinner::Spinner, v_flex};
use rv_business::notifications::Notification;

impl ChatState {
    pub(super) fn visible_notifications(&self) -> impl Iterator<Item = &Notification> {
        self.notifications
            .iter()
            .filter(|n| Some(&n.app_id) == self.account.as_ref())
    }
    pub(super) fn selected_notification(&self) -> Option<&Notification> {
        let remembered = self
            .account
            .as_ref()
            .and_then(|id| self.remembered_notifications.get(id));
        self.visible_notifications()
            .find(|n| Some(&n.remote_id) == remembered)
            .or_else(|| self.visible_notifications().next())
    }
    pub(super) fn notification_unread(&self, app: &AppId) -> Option<usize> {
        self.accounts
            .iter()
            .find(|account| account.id == *app)
            .and_then(|account| account.notification_unread)
    }
}
impl AddressBookApp {
    pub(super) fn switch_message_section(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.chat.notifications_open = index == 1;
        if !self.chat.notifications_open && self.chat.selected.is_none() {
            if let Some(id) = self
                .chat
                .account
                .as_ref()
                .and_then(|app| self.chat.remembered.get(app))
                .filter(|id| self.chat.conversations.iter().any(|c| c.id == **id))
                .cloned()
            {
                let body = draft(self.book.paths(), &id).unwrap_or_default();
                self.chat.selected = Some(id);
                self.chat
                    .composer
                    .update(cx, |state, cx| state.set_value(body, window, cx));
                self.reload_chat_thread(cx);
            }
        }
        cx.notify();
    }
    pub(super) fn render_notification_list(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .id("chat-notifications")
            .debug_selector(|| "chat-notifications".into())
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .when(self.chat.visible_notifications().next().is_none(), |el| {
                el.flex()
                    .items_center()
                    .justify_center()
                    .child(if self.chat.loading {
                        Spinner::new().into_any_element()
                    } else {
                        crate::ui::empty_icon(cx).into_any_element()
                    })
            })
            .children(self.chat.visible_notifications().map(|n| {
                let app = n.app_id.clone();
                let remote = n.remote_id.clone();
                ListItem::new(SharedString::from(format!(
                    "notification-{}-{}",
                    app.as_str(),
                    remote
                )))
                .selected(
                    self.chat
                        .selected_notification()
                        .is_some_and(|selected| selected.remote_id == n.remote_id),
                )
                .child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap_1()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_sm()
                                        .text_ellipsis()
                                        .child(n.title.clone()),
                                )
                                .when(n.unread == Some(true), |row| {
                                    row.child(Badge::new().dot().count(1))
                                }),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::muted(cx))
                                .text_ellipsis()
                                .child(n.body.clone()),
                        )
                        .when_some(n.created_at, |el, time| {
                            el.child(crate::typography::caption(plugins::stamp(time), cx))
                        }),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.chat
                        .remembered_notifications
                        .insert(app.clone(), remote.clone());
                    cx.notify();
                }))
            }))
            .into_any_element()
    }
    pub(super) fn render_notification_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(n) = self.chat.selected_notification() else {
            return v_flex()
                .flex_1()
                .h_full()
                .items_center()
                .justify_center()
                .child(crate::ui::empty_icon(cx))
                .into_any_element();
        };
        let account = self.chat.accounts.iter().find(|a| a.id == n.app_id);
        let mut metadata = account.map(|a| a.title.clone()).unwrap_or_default();
        if let Some(time) = n.created_at {
            metadata.push_str(&format!(" · {}", plugins::stamp(time)));
        }
        v_flex()
            .id("notification-detail")
            .debug_selector(|| "notification-detail".into())
            .flex_1()
            .min_w_0()
            .h_full()
            .min_h_0()
            .child(
                crate::ui::tool_actions()
                    .child(gpui_component::Icon::new(gpui_component::IconName::Bell))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(crate::typography::label("通知", cx)),
                    )
                    .child(crate::typography::caption("只读", cx)),
            )
            .child(
                v_flex()
                    .id("notification-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_3()
                    .gap_3()
                    .child(crate::typography::label(n.title.clone(), cx))
                    .child(crate::typography::caption(metadata, cx))
                    .when_some(n.image_path.clone(), |el, path| {
                        el.child(
                            img(path)
                                .w(px(96.))
                                .h(px(96.))
                                .object_fit(ObjectFit::Contain),
                        )
                    })
                    .child(div().text_sm().child(n.body.clone()))
                    .when(!n.note.is_empty(), |el| {
                        el.child(div().text_sm().child(n.note.clone()))
                    })
                    .when_some(n.item_id.as_ref(), |el, id| {
                        el.child(crate::typography::caption(format!("商品：{id}"), cx))
                    })
                    .when_some(n.order_id.as_ref(), |el, id| {
                        el.child(crate::typography::caption(format!("订单：{id}"), cx))
                    })
                    .when_some(n.link.as_ref(), |el, link| {
                        el.child(crate::typography::caption(link.clone(), cx))
                    }),
            )
            .into_any_element()
    }
}
