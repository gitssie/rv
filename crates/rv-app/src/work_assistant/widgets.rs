pub(super) use crate::ui::field;
use gpui::*;
use gpui_component::{Disableable, IconName, button::Button};

pub(super) fn send_button(id: &'static str, enabled: bool) -> Button {
    Button::new(id)
        .debug_selector(move || id.into())
        .outline()
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
