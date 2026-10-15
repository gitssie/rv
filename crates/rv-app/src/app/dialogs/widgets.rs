use gpui::*;
use gpui_component::h_flex;

pub(in crate::app) use crate::ui::field;

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
        .child(crate::typography::label(label, cx))
        .child(control)
}
