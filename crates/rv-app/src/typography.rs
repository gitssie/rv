//! Shared text roles. Component controls keep their own default typography.
use crate::theme;
use gpui::{App, SharedString, Styled};
use gpui_component::label::Label;

/// Primary labels and body copy match the component library's default medium inputs.
pub(crate) fn label(text: impl Into<SharedString>, cx: &App) -> Label {
    Label::new(text).text_sm().text_color(theme::ink(cx))
}
/// Secondary descriptions, timestamps and technical metadata.
pub(crate) fn caption(text: impl Into<SharedString>, cx: &App) -> Label {
    Label::new(text).text_xs().text_color(theme::muted(cx))
}
