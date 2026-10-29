//! Store -> conversation -> thread. Rendering reads local DTOs and cached image files only.
use super::*;
use crate::{assets::FunctionIcon, theme};
use gpui::prelude::FluentBuilder as _;
use gpui_component::{
    Disableable, Icon, IconName, Selectable,
    badge::Badge,
    button::{Button, ButtonVariants},
    h_flex,
    input::Textarea,
    list::ListItem,
    menu::{DropdownMenu, PopupMenuItem},
    spinner::Spinner,
    tab::{Tab, TabBar},
    v_flex,
};
impl AddressBookApp {
    pub(in crate::app) fn render_chat(&self, height: f32, cx: &mut Context<Self>) -> AnyElement {
        let stores = v_flex()
            .w(px(190.))
            .max_w(relative(0.24))
            .h_full()
            .min_h_0()
            .border_r_1()
            .border_color(theme::line(cx))
            .child(
                crate::ui::tool_actions()
                    .min_h_8()
                    .child(crate::typography::label("店铺", cx)),
            )
            .child(
                v_flex()
                    .id("chat-stores")
                    .debug_selector(|| "chat-stores".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(self.chat.accounts.is_empty(), |el| {
                        el.flex()
                            .items_center()
                            .justify_center()
                            .child(crate::ui::empty_icon(cx))
                    })
                    .children(self.chat.accounts.iter().map(|account| {
                        let id = account.id.clone();
                        let avatar = match account.avatar.as_ref().and_then(|i| i.path.clone()) {
                            Some(path) => gpui_component::avatar::Avatar::new().src(path),
                            None => self.app_avatar(&account.bundle),
                        };
                        ListItem::new(SharedString::from(format!("chat-store-{}", id.as_str())))
                            .selected(self.chat.account.as_ref() == Some(&id))
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .w_full()
                                    .child(avatar)
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_ellipsis()
                                                    .child(account.title.clone()),
                                            )
                                            .child(crate::typography::caption(
                                                account.market.clone(),
                                                cx,
                                            )),
                                    )
                                    .when_some(
                                        if self.chat.notifications_open {
                                            self.chat.notification_unread(&account.id)
                                        } else {
                                            account.unread
                                        },
                                        |row, count| row.child(Badge::new().count(count)),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.filter_chat_account(Some(id.clone()), window, cx)
                            }))
                    })),
            );
        let conversations = v_flex()
            .w(px(260.))
            .max_w(relative(0.3))
            .h_full()
            .min_h_0()
            .border_r_1()
            .border_color(theme::line(cx))
            .child(
                TabBar::new("message-sections")
                    .underline()
                    .selected_index(usize::from(self.chat.notifications_open))
                    .on_click(cx.listener(|this, index: &usize, window, cx| {
                        this.switch_message_section(*index, window, cx)
                    }))
                    .child(
                        Tab::new()
                            .prefix(Icon::new(FunctionIcon::Chat))
                            .label("会话")
                            .when_some(self.chat.account.as_ref(), |tab, id| {
                                let count = self
                                    .chat
                                    .accounts
                                    .iter()
                                    .find(|a| a.id == *id)
                                    .and_then(|a| a.unread);
                                tab.when_some(count.filter(|n| *n > 0), |tab, count| {
                                    tab.suffix(Badge::new().count(count))
                                })
                            }),
                    )
                    .child(
                        Tab::new()
                            .prefix(Icon::new(IconName::Bell))
                            .label("通知")
                            .when_some(self.chat.account.as_ref(), |tab, id| {
                                let count = self.chat.notification_unread(id);
                                tab.when_some(count.filter(|n| *n > 0), |tab, count| {
                                    tab.suffix(Badge::new().count(count))
                                })
                            }),
                    ),
            )
            .when(!self.chat.notifications_open, |el| {
                el.child(
                    crate::ui::tool_actions()
                        .child(div().flex_1().child(crate::typography::label("会话", cx)))
                        .child(
                            Button::new("chat-unread")
                                .ghost()
                                .icon(FunctionIcon::Mail)
                                .selected(self.chat.unread_only)
                                .tooltip("仅未读会话")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.chat.unread_only = !this.chat.unread_only;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("chat-refresh")
                                .ghost()
                                .icon(FunctionIcon::Refresh)
                                .tooltip("刷新会话")
                                .loading(self.chat.account.as_ref().is_some_and(|id|self.chat_syncing(id,&ChatSync::Inbox)))
                                .disabled(
                                    self.read_only
                                        || self.chat.account.as_ref().is_some_and(|id|self.chat_syncing(id,&ChatSync::Inbox))
                                        || !self.chat.accounts.iter().any(|a| {
                                            Some(&a.id) == self.chat.account.as_ref() && a.can_send
                                        }),
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.refresh_chat(cx))),
                        ),
                )
            })
            .when(self.chat.notifications_open,|el| {
                let busy=self.chat.account.as_ref().is_some_and(|id|self.home.manual_syncs.contains(&(id.clone(),rv_business::Operation::Notifications)));
                let available=self.chat.account.as_ref().is_some_and(|id|self.chat.accounts.iter().any(|a|&a.id==id&&a.can_send));
                el.child(crate::ui::tool_actions()
                    .child(div().flex_1().child(crate::typography::label("通知",cx)))
                    .child(Button::new("chat-notification-refresh")
                        .debug_selector(||"chat-notification-refresh".into())
                        .ghost().icon(FunctionIcon::Refresh)
                        .tooltip("刷新通知").accessibility_label("刷新通知")
                        .loading(busy).disabled(self.read_only||!available||busy)
                        .on_click(cx.listener(|this,_,_,cx| {
                            if let Some(id)=this.chat.account.clone(){this.run_shop_sync(id,rv_business::Operation::Notifications,cx);}
                        }))))
            })
            .child(if self.chat.notifications_open {
                self.render_notification_list(cx)
            } else {
                v_flex()
                    .id("chat-conversations")
                    .debug_selector(|| "chat-conversations".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(self.chat.visible().next().is_none(), |el| {
                        el.flex()
                            .items_center()
                            .justify_center()
                            .child(if self.chat.loading {
                                Spinner::new().into_any_element()
                            } else {
                                crate::ui::empty_icon(cx).into_any_element()
                            })
                    })
                    .children(self.chat.visible().map(|c| self.render_conversation(c, cx)))
                    .into_any_element()
            });
        v_flex()
            .id("chat-panel")
            .debug_selector(|| "chat-panel".into())
            .h(px(height))
            .min_h_0()
            .child(
                h_flex()
                    .size_full()
                    .min_h_0()
                    .child(stores)
                    .child(conversations)
                    .child(if self.chat.notifications_open {
                        self.render_notification_detail(cx)
                    } else {
                        self.render_chat_thread(cx)
                    }),
            )
            .into_any_element()
    }
    fn render_conversation(&self, c: &Conversation, cx: &mut Context<Self>) -> AnyElement {
        let id = c.id.clone();
        ListItem::new(SharedString::from(format!("conversation-{}", c.id)))
            .selected(self.chat.selected.as_ref() == Some(&c.id))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .child(plugins::avatar(c))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_sm()
                                            .text_ellipsis()
                                            .child(c.title.clone()),
                                    )
                                    .child(crate::typography::caption(
                                        plugins::stamp(c.updated_at),
                                        cx,
                                    )),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_xs()
                                            .text_color(theme::muted(cx))
                                            .text_ellipsis()
                                            .child(c.preview.clone()),
                                    )
                                    .when(c.unread, |row| row.child(Badge::new().dot().count(1))),
                            ),
                    ),
            )
            .on_click(
                cx.listener(move |this, _, window, cx| this.select_chat(id.clone(), window, cx)),
            )
            .into_any_element()
    }
    fn render_chat_thread(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(c) = self.chat.selected() else {
            return v_flex()
                .flex_1()
                .h_full()
                .items_center()
                .justify_center()
                .child(crate::ui::empty_icon(cx))
                .into_any_element();
        };
        let can_access=!self.read_only && self.chat.accounts.iter().any(|a|a.id==c.app_id&&a.can_send);
        let can_reply=can_access && c.allow_reply==Some(true);
        let conversation = c.id.clone();
        let app = c.app_id.clone();
        let owner = cx.entity();
        let customer = self.chat.thread.customer.as_ref();
        let language = customer
            .and_then(|c| c.language.as_deref())
            .map(rv_business::chat::translation::language_name)
            .unwrap_or("语言");
        let country = customer.and_then(|c| c.country_code.as_ref()).cloned();
        let header = crate::ui::tool_actions()
            .child(plugins::avatar(c))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(crate::typography::label(c.title.clone(), cx))
                    .when_some(country, |el, country| {
                        el.child(crate::typography::caption(country, cx))
                    }),
            )
            .child(
                Button::new("chat-language")
                    .ghost()
                    .icon(FunctionIcon::Languages)
                    .label(language.to_string())
                    .disabled(self.read_only || customer.is_none())
                    .tooltip("客户沟通语言；翻译接口待接入，当前按原文发送")
                    .dropdown_menu(move |mut menu, _, _| {
                        for (code, name) in [
                            ("zh", "中文"),
                            ("en", "English"),
                            ("fr", "Français"),
                            ("es", "Español"),
                            ("de", "Deutsch"),
                            ("it", "Italiano"),
                            ("pt", "Português"),
                            ("nl", "Nederlands"),
                        ] {
                            let owner = owner.clone();
                            let key = conversation.clone();
                            menu = menu.item(PopupMenuItem::new(name).on_click(move |_, _, cx| {
                                owner.update(cx, |this, cx| {
                                    if let Err(error) = rv_business::chat::set_customer_language(
                                        this.book.paths(),
                                        &key,
                                        code,
                                    ) {
                                        this.chat_error(error, cx);
                                    }
                                    this.reload_chat_thread(cx);
                                });
                            }));
                        }
                        menu
                    }),
            )
            .child({
                let id=c.app_id.clone();let operation=ChatSync::Thread(c.id.clone());
                let busy=self.chat_syncing(&id,&operation);
                Button::new("chat-thread-refresh").debug_selector(||"chat-thread-refresh".into()).ghost().icon(FunctionIcon::Refresh)
                    .tooltip("刷新消息").accessibility_label("刷新消息")
                    .loading(busy).disabled(!can_access||busy)
                    .on_click(cx.listener(move |this,_,_,cx|this.run_chat_sync(id.clone(),operation.clone(),true,cx)))
            })
            .child(
                Button::new("chat-read")
                    .ghost()
                    .icon(IconName::Ellipsis)
                    .tooltip("会话操作")
                    .disabled(!can_access)
                    .dropdown_menu({
                        let owner = cx.entity();
                        let conversation = c.id.clone();
                        move |menu, _, _| {
                            let owner = owner.clone();
                            let key = conversation.clone();
                            let id = app.clone();
                            menu.item(PopupMenuItem::new("标记已读").on_click(move |_, _, cx| {
                                owner.update(cx, |this, cx| {
                                    this.run_chat_sync(id.clone(),ChatSync::MarkRead(key.clone()),true,cx);
                                })
                            }))
                        }
                    }),
            );
        let total = self.chat.thread.messages.len() + self.chat.thread.pending.len();
        let transcript = div()
            .id("chat-transcript")
            .debug_selector(|| "chat-transcript".into())
            .flex_1()
            .min_h_0()
            .when(total == 0, |el| {
                el.flex()
                    .items_center()
                    .justify_center()
                    .child(if self.chat.thread_loading {
                        Spinner::new().into_any_element()
                    } else {
                        crate::ui::empty_icon(cx).into_any_element()
                    })
            })
            .when(total > 0, |el| {
                el.child(
                    gpui::list(
                        self.chat.scroll.clone(),
                        cx.processor(|this, index, _, cx| {
                            let Some(c) = this.chat.selected() else {
                                return div().into_any_element();
                            };
                            let Some(message) = this
                                .chat
                                .thread
                                .messages
                                .iter()
                                .chain(&this.chat.thread.pending)
                                .nth(index)
                            else {
                                return div().into_any_element();
                            };
                            let mut row = h_flex().w_full().px_3().py_2().items_center().child(
                                div().flex_1().min_w_0().child(
                                    this.chat
                                        .registry
                                        .renderer(&c.bundle)
                                        .message(message, c, cx),
                                ),
                            );
                            if index >= this.chat.thread.messages.len()
                                && matches!(
                                    message.status.as_str(),
                                    "pending" | "failed" | "unknown"
                                )
                            {
                                let conversation = c.id.clone();
                                let id = message.id.clone();
                                let owner = cx.entity();
                                row = row.child(
                                    Button::new(SharedString::from(format!(
                                        "chat-cancel-{}",
                                        message.id
                                    )))
                                    .ghost()
                                    .icon(IconName::Close)
                                    .tooltip("取消待发消息，不会重新发送")
                                    .disabled(this.read_only)
                                    .on_click(
                                        move |_, _, cx| {
                                            owner.update(cx, |this, cx| {
                                                if let Err(error) = cancel_queued(
                                                    this.book.paths(),
                                                    &conversation,
                                                    &id,
                                                ) {
                                                    this.chat_error(error, cx);
                                                }
                                                this.reload_chat_thread(cx);
                                            })
                                        },
                                    ),
                                );
                            }
                            row.into_any_element()
                        }),
                    )
                    .size_full(),
                )
            });
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .min_h_0()
            .child(header)
            .when(!c.items.is_empty(), |el| {
                el.child(
                    v_flex()
                        .px_3()
                        .py_2()
                        .border_b_1()
                        .border_color(theme::line(cx))
                        .children(
                            c.items.iter().map(|item| {
                                self.chat.registry.renderer(&c.bundle).context(item, cx)
                            }),
                        ),
                )
            })
            .child(transcript)
            .child(
                h_flex()
                    .gap_2()
                    .p_3()
                    .items_end()
                    .border_t_1()
                    .border_color(theme::line(cx))
                    .child(
                        Button::new("chat-send-image")
                            .debug_selector(|| "chat-send-image".into())
                            .ghost()
                            .icon(FunctionIcon::Folder)
                            .tooltip("发送图片")
                            .accessibility_label("发送图片")
                            .loading(self.chat.image_preparing)
                            .disabled(
                                !can_reply
                                    || self.chat.image_preparing
                                    || !strategy(&c.bundle)
                                        .is_some_and(|plugin| plugin.capabilities().upload),
                            )
                            .on_click(
                                cx.listener(|this, _, window, cx| this.pick_chat_image(window, cx)),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Textarea::new(&self.chat.composer).disabled(!can_reply)),
                    )
                    .child(
                        Button::new("chat-send")
                            .primary()
                            .icon(FunctionIcon::Send)
                            .debug_selector(|| "chat-send".into())
                            .loading(self.chat_syncing(&c.app_id,&ChatSync::SendPending(c.id.clone())))
                            .tooltip(if can_reply {
                                "发送原文"
                            } else {
                                "令牌不可用或此会话不允许回复"
                            })
                            .disabled(!can_reply)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.send_chat(window, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }
}
