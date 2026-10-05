//! Small connection presentation primitives shared by card and row.
use super::super::AddressBookApp;
use crate::theme;
use gpui::*;
use gpui_component::{
    Disableable as _, IconName, Sizable,
    button::{Button, ButtonCustomVariant, ButtonVariants as _},
};
use rv_core::ConnectionId;

pub(super) fn label_chip(label: &str, cx: &App) -> impl IntoElement {
    div()
        .max_w(px(84.))
        .text_ellipsis()
        .px_1p5()
        .py_0p5()
        .rounded_sm()
        .text_xs()
        .bg(theme::selected(cx))
        .text_color(theme::accent(cx))
        .child(label.to_owned())
}

pub(super) fn connect_button(
    id: ConnectionId,
    busy: bool,
    cx: &mut Context<AddressBookApp>,
) -> Button {
    Button::new(SharedString::from(format!("connection-connect-{id}")))
        .debug_selector(move || format!("connection-connect-{id}"))
        .custom(
            ButtonCustomVariant::new(cx)
                .color(theme::accent(cx))
                .foreground(theme::accent_fg(cx))
                .hover(rgb(0x4598ff).into())
                .active(rgb(0x2273d5).into()),
        )
        .bg(theme::accent(cx))
        .small()
        .h(px(28.))
        .w(px(76.))
        .icon(IconName::Play)
        .label("连接")
        .disabled(busy)
        .tooltip("连接（双击卡片或按回车）")
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            this.select(id, window, cx);
            this.connect_id(id, cx);
        }))
}
