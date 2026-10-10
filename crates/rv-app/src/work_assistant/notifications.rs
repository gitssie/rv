use super::*;
use gpui_component::Disableable;
impl WorkAssistant {
    pub(super) fn render_notifications(&self, cx: &mut Context<Self>) -> AnyElement {
        let enabled = self.notifications_supported && self.connected;
        let notifications: Vec<_> = self
            .snapshot
            .iter()
            .flat_map(|s| s.notifications.iter())
            .filter(|n| !self.vinted_only || n.source.to_lowercase().contains("vinted"))
            .collect();
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("assistant-notifications-filter")
                            .outline()
                            .small()
                            .label(if self.vinted_only {
                                "仅 Vinted"
                            } else {
                                "全部通知"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.vinted_only = !this.vinted_only;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("assistant-notifications-refresh")
                            .outline()
                            .small()
                            .label("刷新")
                            .disabled(!enabled)
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(AssistantEvent::RefreshNotifications)
                            })),
                    )
                    .child(
                        Button::new("assistant-notifications-subscribe")
                            .ghost()
                            .small()
                            .label(if self.notifications_enabled {
                                "暂停推送"
                            } else {
                                "恢复推送"
                            })
                            .disabled(!enabled)
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.emit(AssistantEvent::Subscribe(!this.notifications_enabled))
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .whitespace_normal()
                    .text_color(theme::muted(cx))
                    .child(if !self.notifications_supported {
                        "此手机尚未提供通知能力"
                    } else {
                        "最近收到的手机通知缓存，不是完整聊天记录。"
                    }),
            )
            .when(notifications.is_empty(), |el| {
                el.child(
                    v_flex()
                        .py_4()
                        .gap_2()
                        .child(div().font_semibold().child(if self.vinted_only {
                            "还没有 Vinted 通知"
                        } else {
                            "还没有收到手机通知"
                        }))
                        .child(
                            div()
                                .text_xs()
                                .whitespace_normal()
                                .text_color(theme::muted(cx))
                                .child(if !self.connected {
                                    "连接手机后可接收通知。"
                                } else if !self.notifications_supported {
                                    "手机服务端需要支持系统通知推送。"
                                } else {
                                    "保持推送开启，或点击刷新获取手机端缓存。"
                                }),
                        ),
                )
            })
            .children(notifications.into_iter().map(|n| {
                v_flex()
                    .gap_1()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(theme::line(cx))
                    .child(div().text_xs().text_color(theme::muted(cx)).child(format!(
                            "{} · {}",
                            n.source,
                            chrono::DateTime::from_timestamp(n.timestamp, 0)
                                .map(|t| t
                                    .with_timezone(&chrono::Local)
                                    .format("%m-%d %H:%M")
                                    .to_string())
                                .unwrap_or_default()
                        )))
                    .when(!n.subtitle.is_empty(), |el| {
                        el.child(
                            div()
                                .text_xs()
                                .whitespace_normal()
                                .text_color(theme::muted(cx))
                                .child(n.subtitle.clone()),
                        )
                    })
                    .child(
                        div()
                            .font_semibold()
                            .whitespace_normal()
                            .child(n.title.clone()),
                    )
                    .child(div().whitespace_normal().child(n.message.clone()))
            }))
            .into_any_element()
    }
}
