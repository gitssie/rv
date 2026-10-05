//! Compact global actions; connection actions belong to their card or row.
use super::{AddressBookApp, ViewMode};
use crate::{actions::*, theme};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    Disableable as _, Icon, IconName, Selectable as _, Sizable,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
};

impl AddressBookApp {
    pub(super) fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .id("management-toolbar")
            .debug_selector(|| "management-toolbar".into())
            .h(px(44.))
            .flex_shrink_0()
            .px_4()
            .gap_3()
            .items_center()
            .border_b_1()
            .border_color(theme::line(cx))
            .bg(theme::sidebar(cx))
            .child(
                Button::new("sidebar-toggle")
                    .ghost()
                    .size(px(28.))
                    .icon(if self.collapsed {
                        IconName::PanelLeftOpen
                    } else {
                        IconName::PanelLeftClose
                    })
                    .tooltip("显示或隐藏分组（⌘B）")
                    .accessibility_label("显示或隐藏分组")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.on_toggle_sidebar(&ToggleSidebar, window, cx);
                    })),
            )
            .child(div().w(px(1.)).h(px(18.)).bg(theme::line(cx)))
            .child(
                div().w(px(280.)).flex_shrink(1.).min_w(px(140.)).child(
                    Input::new(&self.search)
                        .small()
                        .prefix(Icon::new(IconName::Search).text_color(theme::muted(cx))),
                ),
            )
            .child(div().flex_1())
            .child(
                h_flex()
                    .gap_0p5()
                    .child(
                        Button::new("view-grid")
                            .debug_selector(|| "view-grid".into())
                            .ghost()
                            .size(px(28.))
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
                            .debug_selector(|| "view-list".into())
                            .ghost()
                            .size(px(28.))
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
                    .debug_selector(|| "management-settings".into())
                    .ghost()
                    .size(px(28.))
                    .icon(IconName::Settings)
                    .tooltip("偏好设置（⌘,）")
                    .accessibility_label("偏好设置")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.on_prefs(&OpenPreferences, window, cx)
                    })),
            )
            .child(
                Button::new("new")
                    .debug_selector(|| "management-new".into())
                    .outline()
                    .small()
                    .h(px(28.))
                    .icon(IconName::Plus)
                    .label("新增连接")
                    .disabled(self.credential_busy)
                    .tooltip("新增连接（⌘N）")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.on_new(&NewConnection, window, cx)),
                    ),
            )
    }

    /// Visible only when there is feedback to show; replaces the old permanent footer.
    pub(super) fn render_status(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .id("management-feedback")
            .debug_selector(|| "management-feedback".into())
            .w_full()
            .gap_2()
            .px_3()
            .py_1()
            .items_center()
            .rounded_md()
            .bg(theme::card(cx))
            .border_1()
            .border_color(theme::line(cx))
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                Icon::new(IconName::Info)
                    .size_4()
                    .text_color(theme::muted(cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .child(self.status.clone()),
            )
            .when(!self.credential_busy && !self.read_only, |el| {
                el.child(
                    Button::new("dismiss-feedback")
                        .ghost()
                        .size(px(24.))
                        .icon(IconName::Close)
                        .accessibility_label("关闭提示")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.status = "Ready".into();
                            cx.notify();
                        })),
                )
            })
    }
}
