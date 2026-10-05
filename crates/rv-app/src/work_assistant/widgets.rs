use crate::theme;
use gpui::*;
use gpui_component::{
    Sizable,
    button::{Button, ButtonCustomVariant, ButtonVariants},
    v_flex,
};
pub(super) fn field(label: &'static str, control: impl IntoElement, cx: &App) -> Div {
    v_flex()
        .gap_1()
        .child(div().text_xs().text_color(theme::muted(cx)).child(label))
        .child(control)
}
pub(super) fn primary(id: &'static str, label: &'static str, cx: &App) -> Button {
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
