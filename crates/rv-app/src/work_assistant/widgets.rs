use crate::theme;
use gpui::*;
use gpui_component::{
    Disableable, IconName, Sizable,
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
        .label(label)
}

pub(super) fn send_button(id: &'static str, enabled: bool) -> Button {
    Button::new(id)
        .debug_selector(move || id.into())
        .outline()
        .small()
        .h(px(28.))
        .flex_shrink_0()
        .icon(IconName::Copy)
        .label("发送")
        .tooltip("发送到手机剪贴板，在手机输入框内粘贴")
        .disabled(!enabled)
}

pub(super) fn selection_menu(
    menu: gpui_component::menu::PopupMenu,
    window: &Window,
) -> gpui_component::menu::PopupMenu {
    menu.scrollable(true).max_h(px(
        (f32::from(window.viewport_size().height) - 180.).clamp(100., 300.)
    ))
}
