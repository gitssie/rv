//! Modal overlay and routing; form content lives in sibling modules.
mod connection;
mod preferences;
mod widgets;
use super::{AddressBookApp, Modal};
use crate::theme;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    Sizable, StyledExt,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use widgets::primary_button;
impl AddressBookApp {
    pub(super) fn render_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let (title, subtitle): (&str, Option<String>) = match &self.modal {
            Modal::Connection => {
                if self.editing.is_some() {
                    ("编辑连接", None)
                } else {
                    ("新增连接", None)
                }
            }
            Modal::DeleteConfirm { name, .. } => {
                ("删除连接", Some(format!("确定删除“{name}”吗？")))
            }
            Modal::Preferences => ("设置", None),
            Modal::None => return div().into_any_element(),
        };

        let body: AnyElement = match &self.modal {
            Modal::Connection => self.render_connection_dialog(cx),
            Modal::DeleteConfirm { id, .. } => {
                let id = *id;
                v_flex()
                    .gap_3()
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme::muted(cx))
                            .child("已保存的密码和画面预览也会一并删除。"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme::muted(cx))
                            .child("此操作无法撤销。"),
                    )
                    .child(
                        h_flex()
                            .justify_end()
                            .pt_3()
                            .border_t_1()
                            .border_color(theme::line(cx))
                            .gap_2()
                            .child(
                                Button::new("del-cancel")
                                    .small()
                                    .h(px(28.))
                                    .label("取消")
                                    .on_click(cx.listener(|this, _, _, cx| this.close_modal(cx))),
                            )
                            .child(
                                Button::new("del-ok")
                                    .small()
                                    .h(px(28.))
                                    .danger()
                                    .label("删除")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.confirm_delete(id, cx);
                                    })),
                            ),
                    )
                    .into_any_element()
            }
            Modal::Preferences => v_flex()
                .gap_3()
                .child(preferences::prefs_body(self, cx))
                .child(
                    h_flex().justify_end().child(
                        primary_button("prefs-done", "完成", cx)
                            .on_click(cx.listener(|this, _, _, cx| this.close_modal(cx))),
                    ),
                )
                .into_any_element(),
            Modal::None => div().into_any_element(),
        };

        div()
            .id("modal-overlay")
            .occlude()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::scrim())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, _, cx| this.close_modal(cx)))
            .child(
                v_flex()
                    .id("modal-card")
                    .debug_selector(|| "modal-card".into())
                    .w(px(if matches!(self.modal, Modal::DeleteConfirm { .. }) {
                        420.
                    } else {
                        480.
                    }))
                    .max_w(relative(0.9))
                    .max_h(relative(0.9))
                    .whitespace_normal()
                    .rounded_lg()
                    .bg(theme::card(cx))
                    .border_1()
                    .border_color(theme::line(cx))
                    .shadow_lg()
                    .overflow_hidden()
                    .p_5()
                    .gap_4()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_base()
                                    .font_semibold()
                                    .text_color(theme::ink(cx))
                                    .child(title),
                            )
                            .when_some(subtitle, |this, s| {
                                this.child(
                                    div().w_full().text_sm().text_color(theme::ink(cx)).child(s),
                                )
                            }),
                    )
                    .child(
                        v_flex()
                            .id("modal-body-scroll")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .when(
                                !matches!(self.modal, Modal::Connection)
                                    && self.status.as_ref() != "Ready",
                                |el| el.child(self.render_status(cx)),
                            )
                            .child(body),
                    ),
            )
            .into_any_element()
    }
}
