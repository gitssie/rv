use crate::theme;
use gpui::*;
use gpui_component::{
    Sizable,
    button::{Button, ButtonCustomVariant, ButtonVariants as _},
    h_flex,
};

/// Same compact blue action treatment as the connection cards.
pub(super) fn primary_button(id: &'static str, label: &'static str, cx: &App) -> Button {
    Button::new(id)
        .small()
        .h(px(28.))
        .custom(
            ButtonCustomVariant::new(cx)
                .color(theme::accent(cx))
                .foreground(theme::accent_fg(cx))
                .hover(rgb(0x4598ff).into())
                .active(rgb(0x2273d5).into()),
        )
        .bg(theme::accent(cx))
        .label(label)
}
pub(super) fn field(label: &'static str, input: impl IntoElement, cx: &App) -> impl IntoElement {
    h_flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .w(px(72.))
                .flex_shrink_0()
                .text_xs()
                .text_color(theme::muted(cx))
                .child(label),
        )
        .child(div().flex_1().min_w_0().child(input))
}

/// One preference row: label on the left, control on the right.
pub(super) fn setting_row(
    label: &'static str,
    control: impl IntoElement,
    cx: &App,
) -> impl IntoElement {
    h_flex()
        .justify_between()
        .items_center()
        .gap_3()
        .child(div().text_sm().text_color(theme::ink(cx)).child(label))
        .child(control)
}
