//! Shared compositions; individual controls keep GPUI Component defaults.
use crate::{theme, typography};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ThemeStyled,
    button::{Button, ButtonVariants},
    dialog::Dialog,
    h_flex,
    progress::Progress,
    spinner::Spinner,
    v_flex,
};

/// A tool surface has the same padding in its menu and operation pages.
pub(crate) fn tool_panel(cx: &App) -> Div {
    tool_surface(cx).p(theme::tool_panel_padding()).gap_2()
}

pub(crate) fn tool_surface(cx: &App) -> Div {
    v_flex().popover_style(cx).text_sm()
}

pub(crate) fn icon_button(
    id: &'static str,
    icon: impl Into<gpui_component::Icon>,
    label: impl Into<SharedString>,
) -> Button {
    Button::new(id)
        .debug_selector(move || id.into())
        .ghost()
        .icon(icon.into())
        .accessibility_label(label)
}

/// Standard field typography/spacing comes from the library, not per-page label widths.
pub(crate) fn field(label: &'static str, control: impl IntoElement, _cx: &App) -> Stateful<Div> {
    // Standalone Field uses an internal default ID. Give each composition a scope.
    div().id(label).w_full().child(
        gpui_component::form::Field::new()
            .label(label)
            .child(control),
    )
}

pub(crate) fn loading(
    id: &'static str,
    title: impl Into<SharedString>,
    message: SharedString,
    cx: &App,
) -> Stateful<Div> {
    h_flex()
        .id(id)
        .debug_selector(move || id.into())
        .items_start()
        .gap_2()
        .child(Spinner::new())
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(typography::label(title, cx))
                .when(!message.is_empty(), |body| {
                    body.child(
                        div()
                            .debug_selector(move || format!("{id}-message"))
                            .child(typography::caption(message, cx)),
                    )
                }),
        )
}

/// Only native measured totals produce determinate percentages.
pub(crate) fn operation_progress(
    id: &'static str,
    progress: Option<&serde_json::Value>,
) -> Progress {
    let percent = progress.and_then(|progress| {
        let total = progress["total"]
            .as_f64()
            .filter(|n| n.is_finite() && *n > 0.)?;
        let done = progress["completed"]
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.)?;
        Some((done / total * 100.).clamp(0., 100.) as f32)
    });
    Progress::new(id)
        .loading(percent.is_none())
        .value(percent.unwrap_or(0.))
}

/// Compact confirmations that continue as a task panel share one placement policy.
pub(crate) fn compact_dialog(dialog: Dialog, window: &Window, content_height: f32) -> Dialog {
    dialog
        .width(px(320.).min(window.viewport_size().width - px(32.)))
        .margin_top(((window.viewport_size().height - px(content_height)) / 2.).max(px(40.)))
}

pub(crate) fn empty_state(message: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .child(typography::caption(message, cx))
}
