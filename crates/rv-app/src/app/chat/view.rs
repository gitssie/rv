use super::*;
use crate::theme;
use gpui::prelude::FluentBuilder as _;
use gpui_component::{
    Disableable, Selectable, StyledExt, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::Textarea,
    menu::{DropdownMenu, PopupMenuItem},
    v_flex,
};
impl AddressBookApp {
    pub(in crate::app) fn render_chat(&self, height: f32, cx: &mut Context<Self>) -> AnyElement {
        let accounts = self.chat.accounts.clone();
        let entity = cx.entity();
        let account_label = self
            .chat
            .account
            .as_ref()
            .and_then(|id| accounts.iter().find(|a| a.id == *id))
            .map(|a| a.title.clone())
            .unwrap_or("所有账号".into());
        let list = v_flex()
            .w(px(285.))
            .max_w(relative(0.38))
            .h_full()
            .border_r_1()
            .border_color(theme::line(cx))
            .child(
                h_flex()
                    .p_2()
                    .gap_2()
                    .child(
                        Button::new("chat-account")
                            .outline()
                            .label(account_label)
                            .dropdown_menu(move |menu, _, _| {
                                let all = entity.clone();
                                let mut menu = menu.item(PopupMenuItem::new("所有账号").on_click(
                                    move |_, _, cx| {
                                        all.update(cx, |this, cx| {
                                            this.filter_chat_account(None, cx)
                                        })
                                    },
                                ));
                                for account in &accounts {
                                    let view = entity.clone();
                                    let id = account.id.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(account.title.clone()).on_click(
                                            move |_, _, cx| {
                                                view.update(cx, |this, cx| {
                                                    this.filter_chat_account(Some(id.clone()), cx)
                                                })
                                            },
                                        ),
                                    );
                                }
                                menu
                            }),
                    )
                    .child(
                        Button::new("chat-unread")
                            .ghost()
                            .label("未读")
                            .selected(self.chat.unread_only)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.chat.unread_only = !this.chat.unread_only;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("chat-conversations")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(self.chat.visible().next().is_none(), |el| {
                        el.child(div().p_3().text_xs().text_color(theme::muted(cx)).child(
                            if self.chat.loading {
                                "读取中…"
                            } else if self.chat.unread_only {
                                "暂无未读会话"
                            } else {
                                "暂无会话，读取账号资料后点击刷新"
                            },
                        ))
                    })
                    .children(self.chat.visible().map(|c| self.render_conversation(c, cx))),
            )
            .child(
                h_flex()
                    .p_2()
                    .items_center()
                    .border_t_1()
                    .border_color(theme::line(cx))
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(theme::muted(cx))
                            .child(format!("{} 个会话", self.chat.visible().count())),
                    )
                    .child(
                        Button::new("chat-refresh")
                            .ghost()
                            .icon(crate::assets::FunctionIcon::Refresh)
                            .accessibility_label("刷新会话")
                            .tooltip("刷新会话")
                            .disabled(self.read_only)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let ids = this
                                    .chat
                                    .accounts
                                    .iter()
                                    .filter(|a| {
                                        this.chat.account.as_ref().is_none_or(|id| a.id == *id)
                                    })
                                    .map(|a| a.id.clone())
                                    .collect::<Vec<_>>();
                                for id in ids {
                                    this.dispatch_chat(id, ChatAction::Inbox, cx);
                                }
                                this.reload_chat(cx);
                            })),
                    ),
            );
        v_flex()
            .id("chat-panel")
            .text_sm()
            .debug_selector(|| "chat-panel".into())
            .h(px(height))
            .min_h_0()
            .relative()
            .when_some(self.chat.error.as_ref(), |el, error| {
                el.child(
                    div()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .text_color(theme::danger(cx))
                        .child(error.clone()),
                )
            })
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(list)
                    .child(self.render_chat_thread(cx)),
            )
            .into_any_element()
    }
    fn render_conversation(&self, c: &Conversation, cx: &mut Context<Self>) -> AnyElement {
        let id = c.id.clone();
        let (app, symbol, accent) = self.chat.registry.renderer(&c.bundle).identity();
        h_flex()
            .id(SharedString::from(format!("conversation-{}", c.id)))
            .px_3()
            .py_2()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme::line(cx))
            .when(self.chat.selected.as_ref() == Some(&c.id), |el| {
                el.bg(theme::selected(cx))
            })
            .cursor_pointer()
            .on_click(
                cx.listener(move |this, _, window, cx| this.select_chat(id.clone(), window, cx)),
            )
            .child(plugins::avatar(c, 34., cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_ellipsis()
                                    .font_semibold()
                                    .child(c.title.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::muted(cx))
                                    .child(plugins::stamp(c.updated_at)),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                div()
                                    .size(px(14.))
                                    .rounded(theme::radius_small(cx))
                                    .bg(theme::brand_color(accent))
                                    .text_color(theme::accent_fg(cx))
                                    .text_xs()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(symbol),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_ellipsis()
                                    .text_color(theme::muted(cx))
                                    .child(format!("{} · {app}", c.account)),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_xs()
                                    .text_ellipsis()
                                    .text_color(theme::muted(cx))
                                    .child(c.preview.clone()),
                            )
                            .when(c.unread, |el| {
                                el.child(div().size(px(6.)).rounded_full().bg(theme::accent(cx)))
                            }),
                    ),
            )
            .into_any_element()
    }
    fn render_chat_thread(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(c) = self.chat.selected() else {
            return v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_color(theme::muted(cx))
                .child("选择一个会话")
                .into_any_element();
        };
        let plugin = self.chat.registry.renderer(&c.bundle);
        let (app, _, _) = plugin.identity();
        let capability = rv_business::chat::strategy(&c.bundle)
            .map(|p| p.capabilities())
            .unwrap_or_default();
        let busy = self.chat.busy.contains(&c.app_id) || self.home.jobs.contains(&c.app_id);
        let unresolved = self
            .chat
            .thread
            .pending
            .iter()
            .any(|m| ["sending", "submitted", "unknown"].contains(&m.status.as_str()));
        let account = self.chat.accounts.iter().find(|a| a.id == c.app_id);
        let can_reply = account.is_some_and(|a| a.has_auth)
            && !self.read_only
            && capability.text
            && c.allow_reply == Some(true)
            && !busy
            && !unresolved;
        let read_id = c.app_id.clone();
        let remote = c.remote_id.clone();
        let mut content = v_flex().flex_1().min_w_0().h_full().child(
            h_flex()
                .px_3()
                .py_2()
                .gap_2()
                .items_center()
                .border_b_1()
                .border_color(theme::line(cx))
                .child(plugins::avatar(c, 34., cx))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(div().font_semibold().text_ellipsis().child(c.title.clone()))
                        .child(div().text_xs().text_color(theme::muted(cx)).child(format!(
                                "{app} · {}{}",
                                c.account,
                                account
                                    .filter(|a| !a.market.is_empty())
                                    .map(|a| format!(" · {}", a.market))
                                    .unwrap_or_default()
                            ))),
                )
                .when_some(account, |el, account| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(theme::muted(cx))
                            .child(format!("会话{}", account.session_status)),
                    )
                })
                .when(c.unread && capability.mark_read, |el| {
                    el.child(
                        Button::new("chat-mark-read")
                            .ghost()
                            .label("标为已读")
                            .disabled(self.read_only || busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.dispatch_chat(
                                    read_id.clone(),
                                    ChatAction::MarkRead(remote.clone()),
                                    cx,
                                )
                            })),
                    )
                }),
        );
        if let Some(item) = c.items.first() {
            let item_preview = item.clone();
            content = content.child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(theme::line(cx))
                    .child(div().flex_1().min_w_0().child(plugin.context(item, cx)))
                    .child(
                        Button::new("chat-item-detail")
                            .outline()
                            .label("查看商品")
                            .on_click(move |_, window, cx| {
                                let item = item_preview.clone();
                                window.open_dialog(cx, move |dialog, _, cx| {
                                    dialog
                                        .width(px(320.))
                                        .title(item.title.clone())
                                        .child(plugins::thumbnail(item.image.as_ref(), 180., cx))
                                        .children(
                                            item.price
                                                .iter()
                                                .map(|price| div().child(price.clone())),
                                        )
                                });
                            }),
                    ),
            );
        }
        content = content.child(
            div()
                .id("chat-messages")
                .track_scroll(&self.chat.scroll)
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p_3()
                .child(
                    v_flex()
                        .gap_3()
                        .when(self.chat.thread.messages.is_empty(), |el| {
                            el.child(div().text_xs().text_color(theme::muted(cx)).child(if busy {
                                "读取消息…"
                            } else {
                                "暂无本地消息"
                            }))
                        })
                        .children(
                            self.chat
                                .thread
                                .messages
                                .iter()
                                .map(|m| plugin.message(m, cx)),
                        )
                        .children(self.chat.thread.pending.iter().map(|m| {
                            let key = c.id.clone();
                            let id = m.id.clone();
                            v_flex().gap_1().child(plugin.message(m, cx)).when(
                                m.status != "sending",
                                |el| {
                                    el.child(
                                        h_flex().justify_end().child(
                                            Button::new(SharedString::from(format!(
                                                "dismiss-message-{}",
                                                m.id
                                            )))
                                            .ghost()
                                            .label("核对后清除本地记录")
                                            .disabled(self.read_only)
                                            .tooltip("不会重新发送消息")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if let Err(error) =
                                                    rv_business::chat::dismiss_pending(
                                                        this.book.paths(),
                                                        &key,
                                                        &id,
                                                    )
                                                {
                                                    this.chat.error = Some(error);
                                                }
                                                this.reload_chat_thread(cx);
                                            })),
                                        ),
                                    )
                                },
                            )
                        })),
                ),
        );
        if c.allow_reply != Some(true) {
            content = content.child(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(theme::muted(cx))
                    .child("此会话不允许回复"),
            );
        }
        content
            .child(
                v_flex()
                    .p_3()
                    .gap_2()
                    .border_t_1()
                    .border_color(theme::line(cx))
                    .child(Textarea::new(&self.chat.composer).disabled(!can_reply))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .text_xs()
                                    .text_color(theme::muted(cx))
                                    .child("Enter 发送 · Shift+Enter 换行"),
                            )
                            .child(
                                Button::new("chat-send")
                                    .primary()
                                    .label("发送")
                                    .disabled(!can_reply)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.send_chat(window, cx)
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }
}
