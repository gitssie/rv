//! Store -> conversation -> thread. Rendering reads local DTOs and cached image files only.
use super::*;
use crate::{assets::FunctionIcon, theme};
use gpui::prelude::FluentBuilder as _;
use gpui_component::{
    Disableable, IconName, Sizable,
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
    fn chat_avatar(&self,c:&Conversation)->gpui_component::avatar::Avatar {
        if c.kind=="notice" {self.app_avatar(&c.bundle)} else {plugins::avatar(c)}
    }

    pub(super) fn render_chat_stores(&self, cx: &mut Context<Self>) -> AnyElement {
         v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
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
                            .rounded(theme::radius(cx))
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
                                                    .text_ellipsis().whitespace_nowrap()
                                                    .child(SharedString::from((&account.title).to_string())),
                                            )
                                            .child(div().text_xs().text_color(theme::muted(cx)).child(SharedString::from((&account.market).to_string()))),
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
            ).into_any_element()
    }
    pub(super) fn render_chat_conversations(&self, cx: &mut Context<Self>) -> AnyElement {
        let notifications=self.chat.notifications_open;
        let busy=self.chat.account.as_ref().is_some_and(|id| if notifications {
            self.home.manual_syncs.contains(&(id.clone(),rv_business::Operation::Notifications))
        } else { self.chat_syncing(id,&ChatSync::Inbox) });
        let available=self.chat.account.as_ref().is_some_and(|id|self.chat.accounts.iter().any(|a|&a.id==id&&a.can_send));
        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(
                h_flex().w_full().items_center().gap_2().px_5().py_1()
                .child(TabBar::new("message-sections")
                    .segmented()
                    .selected_index(usize::from(self.chat.notifications_open))
                    .on_click(cx.listener(|this, index: &usize, window, cx| {
                        this.switch_message_section(*index, window, cx)
                    }))
                    .child(
                        Tab::new()
                            .icon(FunctionIcon::Chat).aria_label("会话")
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
                            .icon(IconName::Bell).aria_label("通知")
                            .when_some(self.chat.account.as_ref(), |tab, id| {
                                let count = self.chat.notification_unread(id);
                                tab.when_some(count.filter(|n| *n > 0), |tab, count| {
                                    tab.suffix(Badge::new().count(count))
                                })
                            }),
                    ))
                .child(div().flex_1())
                .child(Button::new("chat-list-refresh").ghost().icon(FunctionIcon::Refresh)

                    .loading(busy).disabled(self.read_only||!available||busy)
                    .on_click(cx.listener(move|this,_,_,cx| {
                        if notifications {
                            if let Some(id)=this.chat.account.clone(){this.run_shop_sync(id,rv_business::Operation::Notifications,cx);}
                        }else{this.refresh_chat(cx);}
                    }))),
            )
            .child(if self.chat.notifications_open {
                self.render_notification_list(cx)
            } else {
                v_flex()
                    .id("chat-conversations")
                    .p_2()
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
            }).into_any_element()
    }
    pub(super) fn render_chat_content(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.chat.notifications_open { self.render_notification_detail(cx) }
        else { self.render_chat_thread(cx) }
    }
    fn render_conversation(&self, c: &Conversation, cx: &mut Context<Self>) -> AnyElement {
        let id = c.id.clone();
        ListItem::new(SharedString::from(format!("conversation-{}", c.id)))
            .rounded(theme::radius(cx))
            .selected(self.chat.selected.as_ref() == Some(&c.id))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .child(self.chat_avatar(c))
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
                                            .text_ellipsis().whitespace_nowrap()
                                            .child(SharedString::from((&c.title).to_string())),
                                    )
                                    .child(div().text_xs().text_color(theme::muted(cx)).child(SharedString::from((plugins::stamp(c.updated_at)).to_string()))),
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
                                            .text_ellipsis().whitespace_nowrap()
                                            .child(SharedString::from((&c.preview).to_string())),
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
        let owner = cx.entity();
        let customer = self.chat.thread.customer.as_ref();
        let country = customer.and_then(|c| c.country_code.as_ref()).cloned();
        let header = crate::ui::tool_actions().px_3().py_1()
            .child(self.chat_avatar(c).small())
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(h_flex().gap_2().items_center().min_w_0()
                        .child(div().flex_1().min_w_0().text_sm().child(SharedString::from((&c.title).to_string())))
                        .when_some(country, |el,country| el.child(div().text_xs().text_color(theme::muted(cx))
                            .child(SharedString::from((country).to_string())))))
                    .children(c.items.iter().map(|item| self.chat.registry.renderer(&c.bundle).context(item,cx))),
            )
            .child(
                Button::new("chat-language")
                    .ghost()
                    .icon(FunctionIcon::Languages)
                    .disabled(self.read_only || customer.is_none())

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
                    .accessibility_label("刷新消息")
                    .loading(busy).disabled(!can_access||busy)
                    .on_click(cx.listener(move |this,_,_,cx|this.run_chat_sync(id.clone(),operation.clone(),true,cx)))
            })
            ;
        let total = self.chat.thread.messages.len() + self.chat.thread.pending.len();
        let transcript = div()
            .id("chat-transcript")
            .debug_selector(|| "chat-transcript".into())
            .w_full()
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
                    rv_gpui_chat::message_scroller::MessageScroller::new(
                        SharedString::from(format!("message-scroller-{}", c.id)),
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
                                        .message(message, c, this.chat_avatar(c), cx),
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
                    .with_jump_button_label("回到最新消息")
                    .size_full(),
                )
            });
        v_flex()
            .flex_1()
            .min_w_0()
            .size_full()
            .min_h_0()
            .child(header)
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

                            .disabled(!can_reply)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.send_chat(window, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }
}
