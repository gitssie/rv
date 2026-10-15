//! Modal overlay and routing; form content lives in sibling modules.
mod connection;
pub(in crate::app) mod widgets;
use super::{AddressBookApp, Modal};
use crate::theme;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    StyledExt,
    button::{Button, ButtonVariants as _},
    dialog::Dialog,
    h_flex, v_flex,
};
impl AddressBookApp {
    fn render_shop_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(app) = &self.home.editing else {
            return div().into_any_element();
        };
        let (name, _symbol, _accent, _) =
            self.home.plugins.for_bundle(&app.bundle_id).identity(app);
        v_flex()
            .id("shop-panel")
            .text_sm()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(self.app_avatar(&app.bundle_id))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                div()
                                    .font_semibold()
                                    .text_ellipsis()
                                    .child(app.title.clone()),
                            )
                            .child(div().text_xs().text_color(theme::muted(cx)).child(name)),
                    )
                    .child(
                        self.home
                            .plugins
                            .for_bundle(&app.bundle_id)
                            .header_info(app, cx),
                    )
                    .child(self.render_shop_operations(cx))
                    .child(
                        Button::new("shop-close")
                            .ghost()
                            .icon(gpui_component::IconName::Close)
                            .accessibility_label("关闭店铺详情")
                            .on_click(cx.listener(|this, _, _, cx| this.close_modal(cx))),
                    ),
            )
            .child(
                div()
                    .id("shop-scroll")
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(self.render_business_data(cx)),
            )
            .into_any_element()
    }
    pub(super) fn render_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Modal::PhoneControl(id) = self.modal {
            let Some(control) = self.home.controls.get(&id) else {
                return div().into_any_element();
            };
            return div()
                .id("phone-control-overlay")
                .occlude()
                .absolute()
                .inset_0()
                .pt(px(40.))
                .bg(theme::scrim())
                .on_click(cx.listener(|this, _, _, cx| this.close_modal(cx)))
                .child(
                    div()
                        .id("phone-control-body")
                        .size_full()
                        .relative()
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(control.view.clone()),
                )
                .into_any_element();
        }
        if matches!(self.modal, Modal::Chat) {
            return div().into_any_element();
        }
        if matches!(self.modal, Modal::Shop) {
            return div().into_any_element();
        }
        div().into_any_element()
    }
    fn configure_standard_dialog(
        &self,
        dialog: Dialog,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Dialog {
        let owner = cx.entity();
        if matches!(self.modal, Modal::Shop) {
            return dialog
                .width(px(450.))
                .max_h(relative(0.9))
                .close_button(false)
                .child(self.render_shop_dialog(cx))
                .on_close(move |_, _, cx| owner.update(cx, |this, cx| this.close_modal(cx)));
        }
        if matches!(self.modal, Modal::Chat) {
            return dialog
                .width(px((f32::from(window.viewport_size().width) - 32.).min(980.)))
                .max_h(relative(0.9))
                .title("聊天")
                .child(
                    self.render_chat(
                        (f32::from(window.viewport_size().height) - 140.)
                            .min(520.)
                            .max(100.),
                        cx,
                    ),
                )
                .on_close(move |_, _, cx| owner.update(cx, |this, cx| this.close_modal(cx)));
        }
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
            Modal::AppDelete => ("删除卡片", None),
            Modal::App | Modal::Shop | Modal::Chat => (self.app_editor_title(), None),
            Modal::None | Modal::PhoneControl(_) => return dialog,
        };

        let body: AnyElement = match &self.modal {
            Modal::Connection => self.render_connection_dialog(cx),
            Modal::AppDelete => div().into_any_element(),
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
                                    .label("取消")
                                    .on_click(cx.listener(|this, _, _, cx| this.close_modal(cx))),
                            )
                            .child(Button::new("del-ok").danger().label("删除").on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.confirm_delete(id, cx);
                                }),
                            )),
                    )
                    .into_any_element()
            }
            Modal::App | Modal::Shop | Modal::Chat => self.render_app_editor(cx),
            Modal::None | Modal::PhoneControl(_) => div().into_any_element(),
        };

        let footer = match self.modal {
            Modal::Connection => self.render_connection_footer(cx),
            Modal::App => self.render_app_editor_footer(cx),
            _ => div().into_any_element(),
        };
        let owner = cx.entity();
        let confirm_owner = owner.clone();
        let cancel_owner = owner.clone();
        let content = v_flex()
            .id("modal-card")
            .text_sm()
            .debug_selector(|| "modal-card".into())
            .when_some(subtitle, |el, subtitle| el.child(subtitle))
            .child(body);
        dialog
            .max_h(relative(0.9))
            .title(title)
            .child(content)
            .footer(footer)
            .close_button(!self.credential_busy)
            .keyboard(!self.credential_busy)
            .overlay_closable(!self.credential_busy)
            .on_ok(move |_, window, cx| {
                confirm_owner.update(cx, |this, cx| match this.modal {
                    Modal::Connection => this.submit_connect(false, window, cx),
                    Modal::App => this.save_app_card(cx),
                    _ => {}
                });
                false
            })
            .on_cancel(move |_, _, cx| {
                cancel_owner.update(cx, |this, cx| {
                    if this.credential_busy {
                        return false;
                    }
                    this.close_modal(cx);
                    true
                })
            })
            .on_close(move |_, _, cx| owner.update(cx, |this, cx| this.close_modal(cx)))
    }
    pub(super) fn sync_component_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use gpui_component::WindowExt;
        let kind = match self.modal {
            Modal::Connection => Some(1),
            Modal::DeleteConfirm { .. } => Some(2),
            Modal::App => Some(4),
            Modal::AppDelete => Some(5),
            Modal::Shop => Some(6),
            Modal::Chat => Some(7),
            _ => None,
        };
        if self.home.component_dialog == kind {
            return;
        }
        if self.home.component_dialog.take().is_some() && window.has_active_dialog(cx) {
            window.close_dialog(cx);
            self.focus.focus(window, cx);
        }
        self.home.component_dialog = kind;
        let owner = cx.entity();
        match kind {
            Some(5) => window.open_alert_dialog(cx, move |alert, _, cx| {
                owner.update(cx, |this, cx| this.configure_app_delete_dialog(alert, cx))
            }),
            Some(2) => window.open_alert_dialog(cx, move |alert, _, cx| {
                owner.update(cx, |this, cx| {
                    let Modal::DeleteConfirm { id, ref name } = this.modal else {
                        return alert;
                    };
                    let confirm = cx.entity();
                    let cancel = confirm.clone();
                    alert
                        .title("删除连接")
                        .description(format!("确定删除“{name}”吗？"))
                        .confirm()
                        .button_props(
                            gpui_component::dialog::DialogButtonProps::default()
                                .show_cancel(true)
                                .ok_text("删除")
                                .cancel_text("取消")
                                .ok_variant(gpui_component::button::ButtonVariant::Danger),
                        )
                        .on_ok(move |_, _, cx| {
                            confirm.update(cx, |this, cx| this.confirm_delete(id, cx));
                            true
                        })
                        .on_cancel(move |_, _, cx| {
                            cancel.update(cx, |this, cx| this.close_modal(cx));
                            true
                        })
                        .on_close(cx.listener(|this, _, _, cx| this.close_modal(cx)))
                })
            }),
            Some(_) => window.open_dialog(cx, move |dialog, window, cx| {
                owner.update(cx, |this, cx| {
                    this.configure_standard_dialog(dialog, window, cx)
                })
            }),
            None => {}
        }
        if kind == Some(1) {
            let input = if self.editing.is_some() {
                &self.name_input
            } else {
                &self.server_input
            };
            input.update(cx, |state, cx| state.focus(window, cx));
        }
    }
}
