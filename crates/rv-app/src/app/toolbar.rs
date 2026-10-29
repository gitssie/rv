//! Compact global actions; connection actions belong to their card or row.
use super::{AddressBookApp, ViewMode};
use crate::{actions::*, assets::FunctionIcon, theme};
use gpui::*;
use gpui_component::{
    IconName, Selectable as _, StyledExt,
    badge::Badge,
    button::{Button, ButtonVariants as _},
    h_flex,
};

impl AddressBookApp {
    pub(super) fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let unread_accounts = self
            .chat
            .accounts
            .iter()
            .filter_map(|account| {
                account
                    .unread
                    .filter(|count| *count > 0)
                    .map(|count| format!("{}：{count}", account.title))
            })
            .collect::<Vec<_>>();
        let has_unread = !unread_accounts.is_empty();
        let message_tooltip = if has_unread {
            format!("未读消息\n{}", unread_accounts.join("\n"))
        } else {
            "消息：会话与通知".to_string()
        };
        h_flex()
            .id("management-toolbar")
            .debug_selector(|| "management-toolbar".into())
            .h_full()
            .flex_1()
            .min_w_0()
            .px_2()
            .gap_3()
            .items_center()
            .child(div().font_semibold().mr_2().child("RV"))
            .child(
                Button::new("sidebar-toggle")
                    .occlude()
                    .debug_selector(|| "sidebar-toggle".into())
                    .ghost()
                    .icon(if self.collapsed {
                        IconName::PanelLeftOpen
                    } else {
                        IconName::PanelLeftClose
                    })
                    .tooltip("显示或隐藏设备列表（⌘B）")
                    .accessibility_label("显示或隐藏设备列表")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.on_toggle_sidebar(&ToggleSidebar, window, cx);
                    })),
            )
            .child(div().w(px(1.)).h(px(18.)).bg(theme::line(cx)))
            .child(
                Button::new("home-refresh")
                    .occlude()
                    .ghost()
                    .icon(FunctionIcon::Refresh)
                    .tooltip("刷新卡片")
                    .accessibility_label("刷新卡片")
                    .on_click(cx.listener(|this, _, _, cx| this.reload_home(cx))),
            )
            .child(
                Badge::new().dot().count(usize::from(has_unread)).child(
                    Button::new("chat-open")
                        .occlude()
                        .debug_selector(|| "chat-open".into())
                        .ghost()
                        .icon(FunctionIcon::Chat)
                        .accessibility_label("消息")
                        .tooltip(message_tooltip)
                        .selected(self.chat.window.is_some())
                        .on_click(cx.listener(|this, _, window, cx| this.open_chat(window,cx))),
                ),
            )
            .child(div().flex_1())
            .child(
                h_flex()
                    .gap_0p5()
                    .child(
                        Button::new("view-grid")
                            .occlude()
                            .debug_selector(|| "view-grid".into())
                            .ghost()
                            .icon(IconName::LayoutDashboard)
                            .selected(self.view_mode == ViewMode::Grid)
                            .tooltip("网格视图（⌘L）")
                            .accessibility_label("网格视图")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.view_mode = ViewMode::Grid;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("view-list")
                            .occlude()
                            .debug_selector(|| "view-list".into())
                            .ghost()
                            .icon(IconName::Menu)
                            .selected(self.view_mode == ViewMode::List)
                            .tooltip("列表视图（⌘L）")
                            .accessibility_label("列表视图")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.view_mode = ViewMode::List;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                Button::new("prefs")
                    .occlude()
                    .debug_selector(|| "management-settings".into())
                    .ghost()
                    .icon(IconName::Settings)
                    .tooltip("偏好设置（⌘,）")
                    .accessibility_label("偏好设置")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.on_prefs(&OpenPreferences, window, cx)
                    })),
            )
    }
}
