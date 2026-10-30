//! Shared compositions; individual controls keep GPUI Component defaults.
use crate::{theme, typography};
use gpui::*;
use gpui_component::{
    Disableable, Sizable as _, StyledExt as _, ThemeStyled, WindowExt,
    button::{Button, ButtonVariants},
    dialog::Dialog,
    h_flex,
    notification::{Notification, NotificationDelivery},
    v_flex,
};

/// One transient feedback slot per window owner, including inactive windows.
/// Retain the returned task so replacing feedback cancels its previous deadline.
pub(crate) fn notify<T: 'static>(
    note: Notification,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Task<()> {
    window.defer(cx, move |window, cx| {
        window.push_notification(
            note.id::<T>()
                .placement(Anchor::BottomRight)
                .delivery(NotificationDelivery::InApp)
                .autohide(true),
            cx,
        );
    });
    cx.spawn_in(window, async move |this, cx| {
        cx.background_executor()
            .timer(std::time::Duration::from_secs(3))
            .await;
        let _ = this.update_in(cx, |_, window, cx| {
            window.remove_notification::<T>(cx);
        });
    })
}

/// A tool surface has the same padding in its menu and operation pages.
pub(crate) fn tool_panel(cx: &App) -> Div {
    tool_surface(cx).p(theme::tool_panel_padding()).gap_2()
}

pub(crate) fn tool_surface(cx: &App) -> Div {
    tool_section().popover_style(cx).text_sm()
}

/// Content groups share the panel edge; only the outer surface adds padding.
pub(crate) fn tool_section() -> Div {
    v_flex().w_full().min_w_0().gap_2()
}

/// Use the same centered baseline and spacing for every tool action row.
pub(crate) fn tool_actions() -> Div {
    h_flex().w_full().min_w_0().items_center().gap_2()
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

/// The standard Alert keeps its layout while only the loading icon rotates.
pub(crate) fn loading(
    id: &'static str,
    title: impl Into<SharedString>,
    message: SharedString,
    _cx: &App,
) -> Stateful<Div> {
    let message = if message.is_empty() {
        title.into()
    } else {
        message
    };
    v_flex().id(id).debug_selector(move || id.into()).child(
        gpui_component::alert::Alert::info("loading-status", message).with_animation(
            "loading-icon",
            Animation::new(std::time::Duration::from_millis(800))
                .repeat()
                .with_easing(ease_in_out),
            |alert, delta| {
                alert.icon(
                    gpui_component::Icon::new(gpui_component::IconName::Loader)
                        .transform(Transformation::rotate(percentage(delta))),
                )
            },
        ),
    )
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

pub(crate) fn empty_icon(cx: &App) -> Div {
    div().flex_1().flex().items_center().justify_center().child(
        gpui_component::Icon::new(gpui_component::IconName::Inbox)
            .large()
            .text_color(theme::muted(cx)),
    )
}

/// All explicit imports from a phone use the same direction and credential warning.
pub(crate) fn confirm_backup_sync(
    source: String,
    window: &mut Window,
    cx: &mut App,
    on_confirm: impl Fn(&mut Window, &mut App) + 'static,
) {
    let on_confirm = std::rc::Rc::new(on_confirm);
    window.open_alert_dialog(cx, move |alert, _, _| {
        let on_confirm = on_confirm.clone();
        alert
            .title("从手机同步账号备份？")
            .description(format!(
                "将从“{source}”读取账号备份，更新 RV 中保存的 state 和设备请求参数。\n手机备份中的令牌与 RV 不同时，保留 RV 当前的 Access Token 和 Refresh Token。此操作不会将 RV 状态写回手机。"
            ))
            .confirm()
            .button_props(
                gpui_component::dialog::DialogButtonProps::default()
                    .show_cancel(true)
                    .ok_text("确认同步")
                    .cancel_text("取消"),
            )
            .on_ok(move |_, window, cx| {
                on_confirm(window, cx);
                true
            })
    });
}

pub(crate) const RV_HANDOFF_DESCRIPTION: &str = "请先在 Shadow 保存该环境的最新备份，并切换到其他环境。目标仍为 active 时不能交给 RV。RV 只读取所选备份的 state 和令牌，不会自动备份或切换手机环境；已停止的任务保持停止。";
pub(crate) const PHONE_HANDOFF_DESCRIPTION: &str = "RV 将停止该店铺的调度，通过 ZXTouch 自动关闭手机 App 并确认进程退出，等待在途任务结束后，把最新状态写回手机备份。当前 active 备份也可交接；回写不会自动应用到当前环境，请先在 Shadow 应用最新状态，再打开 App。";

pub(crate) struct ExecutionHandoffPrompt {
    pub title: &'static str,
    pub description: &'static str,
    pub icon: crate::assets::FunctionIcon,
    pub device: String,
    pub backup: String,
    pub acknowledgement: Option<&'static str>,
}

/// Shared compact confirmation for either execution owner. The optional user
/// acknowledgement is explicit because a backup does not capture live changes.
pub(crate) fn confirm_execution_handoff(
    prompt: ExecutionHandoffPrompt,
    window: &mut Window,
    cx: &mut App,
    on_confirm: impl Fn(&mut Window, &mut App) + 'static,
) {
    use gpui_component::{
        Icon, IconName,
        checkbox::Checkbox,
        dialog::{Cancel, Confirm, DialogFooter},
    };
    let accepted = std::rc::Rc::new(std::cell::Cell::new(prompt.acknowledgement.is_none()));
    let on_confirm = std::rc::Rc::new(on_confirm);
    window.open_alert_dialog(cx, move |alert, _, cx| {
        let accept = accepted.clone();
        let confirm = on_confirm.clone();
        let checked = accepted.clone();
        let mut content = v_flex()
            .gap_3()
            .text_sm()
            .font_normal()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Icon::new(crate::assets::FunctionIcon::Smartphone)
                            .text_color(theme::accent(cx)),
                    )
                    .child(typography::label(prompt.device.clone(), cx)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(Icon::new(IconName::File).text_color(theme::muted(cx)))
                    .child(typography::caption(prompt.backup.clone(), cx)),
            )
            .child(div().text_xs().font_normal().text_color(theme::muted(cx))
                .whitespace_normal().line_height(relative(1.5)).child(prompt.description));
        if let Some(label) = prompt.acknowledgement {
            content = content.child(
                Checkbox::new("handoff-acknowledgement")
                    .small()
                    .font_normal()
                    .debug_selector(|| "handoff-acknowledgement".into())
                    .label(label)
                    .checked(accepted.get())
                    .on_click(move |value, window, _| {
                        checked.set(*value);
                        window.refresh();
                    }),
            );
        }
        alert
            .title(prompt.title)
            .icon(Icon::new(prompt.icon.clone()).text_color(theme::accent(cx)))
            .child(content)
            .on_ok(move |_, window, cx| {
                if !accept.get() {
                    return false;
                }
                confirm(window, cx);
                true
            })
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("handoff-cancel")
                            .label("取消")
                            .on_click(|_, window, cx| window.dispatch_action(Box::new(Cancel), cx)),
                    )
                    .child(
                        Button::new("handoff-confirm")
                            .primary()
                            .icon(prompt.icon.clone())
                            .label(prompt.title)
                            .debug_selector(|| "handoff-confirm".into())
                            .disabled(!accepted.get())
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(Confirm { secondary: false }), cx)
                            }),
                    ),
            )
    });
}
