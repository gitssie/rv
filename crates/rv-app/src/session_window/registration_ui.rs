//! Compact functionality tab; registration keeps a single freeform input.
use super::*;
use gpui_component::checkbox::Checkbox;

pub(super) struct RegistrationUi {
    open: bool,
    text: Entity<TextareaState>,
    marketing: bool,
    terms: bool,
    pub busy: bool,
    pub message: SharedString,
}
impl RegistrationUi {
    pub fn new(window: &mut Window, cx: &mut Context<SessionView>) -> Self {
        Self {
            open: false,
            text: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .rows(4)
                    .placeholder("邮箱 用户名 密码\n取码链接（可选）")
            }),
            marketing: false,
            terms: true,
            busy: false,
            message: "等待开始".into(),
        }
    }
}
impl FunctionUi {
    pub(in crate::session_window) fn registration_busy(&self) -> bool {
        self.registration.busy
    }
}
impl SessionView {
    fn execute_registration(&mut self, cx: &mut Context<Self>) {
        if self.functions.registration.busy
            || !self.functions.available
            || self.req.view_only
            || self.phase != Phase::Connected
        {
            return;
        }
        if self.functions.busy
            || self.functions.panels.values().any(FunctionPanel::resumable)
            || self.touch_gesture.busy()
            || self.clipboard_transfer.busy()
            || self.buttons != 0
            || self.app_environment_busy()
            || self.unlock.blocks_input()
            || self.apps.mutations_pending()
        {
            self.functions.registration.message = "请先完成当前手机操作".into();
            cx.notify();
            return;
        }
        self.cancel_post_paste_touch();
        let released = self.keys.release_all();
        self.send_keys(released);
        let ui = &mut self.functions.registration;
        let text = ui.text.read(cx).value().to_string();
        if let Err(message) =
            rv_session::functions::registration::Request::parse(&text, ui.marketing, ui.terms)
        {
            ui.message = message.into();
            cx.notify();
            return;
        }
        ui.busy = true;
        ui.message = "正在识别注册页面…".into();
        self.handle.function(FunctionCommand::Register {
            text,
            marketing: ui.marketing,
            accept_terms: ui.terms,
        });
        cx.notify();
    }

    pub(super) fn render_registration_panel(
        &self,
        width: f32,
        height: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ui = &self.functions.registration;
        let busy = ui.busy;
        let live =
            self.functions.available && self.phase == Phase::Connected && !self.req.view_only;
        let panel = crate::ui::tool_panel(cx)
            .id("registration-panel")
            .debug_selector(|| "registration-panel".into())
            .w(px(360f32.min(width - 16.)))
            .max_h(px((height - 16.).max(100.)))
            .overflow_y_scroll()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(self.render_function_navigation(cx))
            .when(!ui.open, |panel| {
                panel.child(
                    Button::new("registration-open")
                        .ghost()
                        .w_full()
                        .debug_selector(|| "registration-open".into())
                        .child(
                            crate::ui::tool_actions()
                                .child(Icon::new(FunctionIcon::User))
                                .child(div().flex_1().text_left().child("注册"))
                                .child(Icon::new(IconName::ChevronRight)),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.functions.registration.open = true;
                            cx.notify();
                        })),
                )
            })
            .when(ui.open, |panel| {
                panel.child(
                    crate::ui::tool_section()
                        .child(
                            crate::ui::tool_actions()
                                .child(
                                    crate::ui::icon_button(
                                        "registration-back",
                                        IconName::ArrowLeft,
                                        "返回",
                                    )
                                    .on_click(cx.listener(
                                        |this, _, _, cx| {
                                            this.functions.registration.open = false;
                                            cx.notify();
                                        },
                                    )),
                                )
                                .child(div().flex_1().font_semibold().child("注册"))
                                .child(crate::typography::caption("Vinted", cx)),
                        )
                        .child(
                            div()
                                .id("registration-input")
                                .w_full()
                                .min_w_0()
                                .flex_shrink_0()
                                .debug_selector(|| "registration-input".into())
                                .child(Textarea::new(&ui.text).h(rems(8.)).disabled(busy)),
                        )
                        .child(crate::typography::caption(
                            "邮箱 用户名 密码；下一行填写取码链接",
                            cx,
                        ))
                        .child(
                            crate::ui::tool_actions()
                                .child(
                                    Checkbox::new("registration-marketing")
                                        .label("订阅信息")
                                        .checked(ui.marketing)
                                        .disabled(busy)
                                        .on_click(cx.listener(|this, value: &bool, _, cx| {
                                            this.functions.registration.marketing = *value;
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Checkbox::new("registration-terms")
                                        .label("接受协议")
                                        .checked(ui.terms)
                                        .disabled(busy)
                                        .on_click(cx.listener(|this, value: &bool, _, cx| {
                                            this.functions.registration.terms = *value;
                                            cx.notify();
                                        })),
                                ),
                        )
                        .child(crate::typography::caption(
                            "支持邮箱取码验证；手机号验证需在手机继续",
                            cx,
                        ))
                        .child(
                            crate::ui::tool_actions()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_xs()
                                        .text_color(theme::muted(cx))
                                        .child(ui.message.clone()),
                                )
                                .when(busy, |row| {
                                    row.child(
                                        Button::new("registration-stop").label("停止").on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.handle
                                                    .function(FunctionCommand::StopRegistration);
                                                this.functions.registration.message =
                                                    "正在停止…".into();
                                                cx.notify();
                                            }),
                                        ),
                                    )
                                })
                                .when(!busy, |row| {
                                    row.child(
                                        Button::new("registration-execute")
                                            .primary()
                                            .label("执行")
                                            .debug_selector(|| "registration-execute".into())
                                            .disabled(!live || !ui.terms)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.execute_registration(cx)
                                            })),
                                    )
                                }),
                        ),
                )
            });
        h_flex()
            .absolute()
            .top(px(8.))
            .right(px(8.))
            .items_start()
            .child(panel)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{FunctionCommand, FunctionEvent};
    use crate::session_window::ui_tests::setup;
    use gpui::{Modifiers, TestAppContext};

    #[gpui::test]
    fn registration_uses_one_draft_and_keeps_it_across_tabs(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        view.update(cx, |view, cx| {
            view.functions.open = true;
            view.apply_function_event(FunctionEvent::Available(true), cx);
        });
        for selector in ["function-tab-功能", "registration-open"] {
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let bounds = cx.debug_bounds(selector).unwrap();
            cx.simulate_click(bounds.center(), Modifiers::default());
        }
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.functions.registration.text.update(cx, |input, cx| {
                    input.set_value("a@example.test user secret", window, cx)
                });
            })
        });
        for selector in ["function-tab-Apple", "function-tab-功能"] {
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let bounds = cx.debug_bounds(selector).unwrap();
            cx.simulate_click(bounds.center(), Modifiers::default());
        }
        view.read_with(cx, |view, cx| {
            assert!(view.functions.registration.open);
            assert_eq!(
                view.functions.registration.text.read(cx).value(),
                "a@example.test user secret"
            );
            assert!(!view.functions.registration.marketing);
            assert!(view.functions.registration.terms);
        });
        while peer.commands.try_recv().is_ok() {}
        view.update(cx, |view, cx| {
            view.execute_registration(cx);
            view.execute_registration(cx);
            assert!(view.view_only());
        });
        let commands: Vec<_> = std::iter::from_fn(|| peer.commands.try_recv().ok()).collect();
        assert_eq!(
            commands
                .iter()
                .filter(|command| matches!(
                    command,
                    rv_session::SessionCommand::Function(FunctionCommand::Register { .. })
                ))
                .count(),
            1
        );
        view.update(cx, |view, cx| {
            view.apply_function_event(
                FunctionEvent::Registration(rv_session::functions::registration::Status {
                    running: false,
                    message: "连接不可用",
                }),
                cx,
            );
            assert!(!view.view_only());
            assert_eq!(
                view.functions.registration.text.read(cx).value(),
                "a@example.test user secret"
            );
        });
    }
}
