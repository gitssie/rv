use super::apps_ui::app_icon;
use super::*;
use crate::assets::AppActionIcon;
use rv_session::{AppCommand, NotificationSnapshot};

pub(super) struct NotificationsUI {
    pub enabled: bool,
    pub open: bool,
    pub unread: usize,
    pub snapshot: Option<NotificationSnapshot>,
    pub error: Option<String>,
}
impl Default for NotificationsUI {
    fn default() -> Self {
        Self {
            enabled: true,
            open: false,
            unread: 0,
            snapshot: None,
            error: None,
        }
    }
}
impl NotificationsUI {
    pub fn receive(&mut self, snapshot: NotificationSnapshot) {
        if let Some(old) = &self.snapshot {
            let new_count = snapshot
                .notifications
                .iter()
                .filter(|n| !old.notifications.iter().any(|old| old.id == n.id))
                .count();
            if !self.open {
                self.unread = (self.unread + new_count).min(100);
            }
        }
        self.snapshot = Some(snapshot);
        self.error = None;
    }
}

fn screenshot_png(frame: rv_session::Framebuffer) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        frame.width > 0 && frame.height > 0,
        "No remote frame is available"
    );
    let mut rgba = frame.pixels;
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let image = ImageBuffer::<Rgba<u8>, _>::from_raw(frame.width.into(), frame.height.into(), rgba)
        .ok_or_else(|| anyhow::anyhow!("Invalid remote framebuffer"))?;
    let mut output = std::io::Cursor::new(Vec::new());
    image.write_to(&mut output, image::ImageFormat::Png)?;
    Ok(output.into_inner())
}

impl SessionView {
    fn screenshot(&mut self, save: bool, cx: &mut Context<Self>) {
        if self.phase != Phase::Connected || self.screenshot_busy {
            return;
        }
        let Ok(frame) = self.handle.framebuffer.lock().map(|fb| fb.clone()) else {
            return;
        };
        self.screenshot_busy = true;
        cx.spawn(async move |this, cx| {
            let png = cx
                .background_executor()
                .spawn(async move { screenshot_png(frame) })
                .await;
            let outcome: anyhow::Result<&str> = async {
                let png = png?;
                if save {
                    let folder = directories::UserDirs::new()
                        .map(|dirs| dirs.picture_dir().unwrap_or(dirs.home_dir()).to_path_buf())
                        .unwrap_or_else(|| PathBuf::from("."));
                    let name = format!("RV-{}.png", chrono::Local::now().format("%Y%m%d-%H%M%S"));
                    let prompt = cx.update(|cx| cx.prompt_for_new_path(&folder, Some(&name)));
                    if let Some(path) = prompt.await?? {
                        cx.background_executor()
                            .spawn(async move { std::fs::write(path, png) })
                            .await?;
                        Ok("截图已保存")
                    } else {
                        Ok("已取消保存截图")
                    }
                } else {
                    cx.update(|cx| {
                        cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(
                            ImageFormat::Png,
                            png,
                        )))
                    });
                    Ok("截图已复制")
                }
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                this.screenshot_busy = false;
                this.status = match outcome {
                    Ok(message) => message.into(),
                    Err(error) => format!("截图失败：{error}").into(),
                };
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_home(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let supported = self
            .apps
            .caps
            .as_ref()
            .is_some_and(|caps| caps.home && caps.control);
        crate::ui::icon_button("home-screen", AppActionIcon::Home, "回到主屏幕")
            .on_click(cx.listener(|this, _, _, _| {
                if this.phase == Phase::Connected
                    && !this.view_only()
                    && !this.unlock.blocks_input()
                {
                    this.handle.app(AppCommand::Home);
                }
            }))
            .disabled(
                !supported
                    || self.phase != Phase::Connected
                    || self.view_only()
                    || self.unlock.blocks_input(),
            )
    }

    pub(super) fn render_screenshot(&self, cx: &mut Context<Self>) -> impl IntoElement {
        crate::ui::icon_button("screenshot", AppActionIcon::Camera, "复制截图")
            .on_click(cx.listener(|this, _, _, cx| this.screenshot(false, cx)))
            .disabled(
                self.phase != Phase::Connected
                    || self.screenshot_busy
                    || self.render_image.is_none(),
            )
    }

    pub(super) fn render_notification_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let supported = self
            .apps
            .caps
            .as_ref()
            .is_some_and(|caps| caps.notifications);
        div()
            .relative()
            .flex_shrink_0()
            .child(
                crate::ui::icon_button("phone-notifications", IconName::Bell, "手机通知").on_click(
                    cx.listener(|this, _, _, cx| {
                        this.notifications.open = !this.notifications.open;
                        if this.notifications.open {
                            this.mailbox.open = false;
                            let released = this.keys.release_all();
                            this.send_keys(released);
                            this.command_paste_pressed = false;
                        }
                        this.notifications.unread = 0;
                        this.info_open = false;
                        if this.notifications.open
                            && this.phase == Phase::Connected
                            && this
                                .apps
                                .caps
                                .as_ref()
                                .is_some_and(|caps| caps.notifications)
                        {
                            this.handle.app(AppCommand::Notifications);
                        }
                        cx.notify();
                    }),
                ),
            )
            .when(self.notifications.unread > 0, |el| {
                el.child(
                    div()
                        .absolute()
                        .top_0()
                        .right_0()
                        .size(px(5.))
                        .rounded_full()
                        .bg(theme::accent(cx)),
                )
            })
            .when(!supported, |el| el.opacity(0.6))
    }

    pub(super) fn render_notifications(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let supported = self.phase == Phase::Connected
            && self
                .apps
                .caps
                .as_ref()
                .is_some_and(|caps| caps.notifications);
        let count = self
            .notifications
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.notifications.len());
        crate::ui::tool_surface(cx)
            .id("phone-notification-panel")
            .occlude()
            .absolute()
            .top_2()
            .right_2()
            .w(px(320.))
            .max_w_full()
            .max_h(px(420.))
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_1()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_semibold()
                            .child(if count == 0 {
                                "通知".into()
                            } else {
                                format!("通知 · {count}")
                            }),
                    )
                    .child(
                        crate::ui::icon_button(
                            "auto-notifications",
                            IconName::Bell,
                            "自动接收通知",
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.notifications.enabled = !this.notifications.enabled;
                            this.handle.app(AppCommand::SubscribeNotifications(
                                this.notifications.enabled,
                            ));
                            cx.notify();
                        }))
                        .selected(self.notifications.enabled)
                        .disabled(!supported),
                    )
                    .child(
                        crate::ui::icon_button(
                            "refresh-notifications",
                            AppActionIcon::Restart,
                            "刷新已接收的通知",
                        )
                        .on_click(
                            cx.listener(|this, _, _, _| this.handle.app(AppCommand::Notifications)),
                        )
                        .disabled(!supported),
                    )
                    .child(
                        crate::ui::icon_button(
                            "close-notifications",
                            IconName::Close,
                            "关闭通知列表",
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.notifications.open = false;
                            cx.notify();
                        })),
                    ),
            )
            .when(!supported, |el| {
                el.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_sm()
                        .text_color(theme::muted(cx))
                        .child(if self.phase == Phase::Connected {
                            "通知不可用"
                        } else {
                            "已断开"
                        }),
                )
            })
            .when_some(self.notifications.error.clone(), |el, error| {
                el.child(
                    div()
                        .px_3()
                        .text_xs()
                        .text_color(theme::warning(cx))
                        .child(error),
                )
            })
            .child(
                v_flex()
                    .id("notification-list")
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_3()
                    .gap_2()
                    .when(count == 0 && supported, |el| {
                        el.child(
                            div()
                                .py_4()
                                .text_sm()
                                .text_color(theme::muted(cx))
                                .child("暂无通知"),
                        )
                    })
                    .children(
                        self.notifications
                            .snapshot
                            .iter()
                            .flat_map(|snapshot| &snapshot.notifications)
                            .map(|notification| {
                                let time =
                                    chrono::DateTime::from_timestamp(notification.timestamp, 0)
                                        .map(|time| {
                                            time.with_timezone(&chrono::Local)
                                                .format("%H:%M")
                                                .to_string()
                                        })
                                        .unwrap_or_default();
                                let name = self
                                    .apps
                                    .installed
                                    .iter()
                                    .find(|app| app.bundle_id == notification.source)
                                    .map(|app| app.name.clone())
                                    .unwrap_or_else(|| {
                                        if rv_session::valid_bundle_id(&notification.source) {
                                            "通知".into()
                                        } else {
                                            notification.source.clone()
                                        }
                                    });
                                h_flex()
                                    .items_start()
                                    .py_2()
                                    .gap_2()
                                    .border_b_1()
                                    .border_color(theme::line(cx))
                                    .child(app_icon(self.apps.icon_for(&notification.source), 28.))
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(
                                                h_flex()
                                                    .gap_2()
                                                    .items_center()
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_w_0()
                                                            .text_xs()
                                                            .text_ellipsis()
                                                            .text_color(theme::muted(cx))
                                                            .child(name),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex_shrink_0()
                                                            .text_xs()
                                                            .text_color(theme::muted(cx))
                                                            .child(time),
                                                    ),
                                            )
                                            .when(!notification.title.is_empty(), |el| {
                                                el.child(
                                                    div()
                                                        .text_sm()
                                                        .font_semibold()
                                                        .child(notification.title.clone()),
                                                )
                                            })
                                            .when(!notification.subtitle.is_empty(), |el| {
                                                el.child(
                                                    div()
                                                        .text_xs()
                                                        .child(notification.subtitle.clone()),
                                                )
                                            })
                                            .when(!notification.message.is_empty(), |el| {
                                                el.child(
                                                    div()
                                                        .text_sm()
                                                        .child(notification.message.clone()),
                                                )
                                            }),
                                    )
                            }),
                    ),
            )
    }

    pub(super) fn render_recovery(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let connecting = matches!(self.phase, Phase::Connecting | Phase::Disconnecting);
        let automatic = self.recovery.deadline.is_some() || connecting;
        let auth_failed = self.error.as_deref().is_some_and(password_error);
        div()
            .id("session-recovery")
            .debug_selector(|| "session-recovery".into())
            .occlude()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::scrim())
            .child(
                v_flex()
                    .debug_selector(|| "recovery-dialog".into())
                    .w(px(280.))
                    .max_w_full()
                    .p_3()
                    .gap_2()
                    .rounded(theme::radius(cx))
                    .bg(theme::card(cx))
                    .border_1()
                    .border_color(theme::line(cx))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(Icon::new(if automatic {
                                IconName::Loader
                            } else {
                                IconName::TriangleAlert
                            }))
                            .child(div().text_sm().font_semibold().child(if connecting {
                                "正在重连"
                            } else {
                                "连接中断"
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::muted(cx))
                            .child(if connecting {
                                format!("第 {} 次尝试", self.recovery.attempt.max(1))
                            } else if self.recovery.deadline.is_some() {
                                format!(
                                    "{} 秒后重试 · 第 {} 次",
                                    self.recovery.seconds(std::time::Instant::now()),
                                    self.recovery.attempt
                                )
                            } else if auth_failed {
                                "密码错误，请重新输入".into()
                            } else if self.recovery.stopped {
                                "自动重连已停止".into()
                            } else {
                                "可手动重试".into()
                            }),
                    )
                    .when(!automatic && !auth_failed, |panel| {
                        panel.when_some(self.error.clone(), |panel, error| {
                            panel.child(div().text_xs().text_color(theme::muted(cx)).child(error))
                        })
                    })
                    .when(auth_failed, |panel| {
                        panel
                            .child(
                                Input::new(&self.password_input)
                                    .w_full()
                                    .aria_label("Password for reconnect"),
                            )
                            .when_some(self.password_validation.clone(), |panel, error| {
                                panel.child(
                                    div().text_xs().text_color(theme::danger(cx)).child(error),
                                )
                            })
                            .child(
                                Checkbox::new("retry-remember-password")
                                    .checked(self.remember_password)
                                    .label("记住密码")
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.remember_password = *checked;
                                        cx.notify();
                                    })),
                            )
                    })
                    .child(
                        h_flex()
                            .gap_2()
                            .justify_end()
                            .child(
                                Button::new("recovery-now")
                                    .ghost()
                                    .text_color(theme::accent(cx))
                                    .label("立即重连")
                                    .disabled(connecting)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if auth_failed {
                                            this.retry_with_password(window, cx);
                                        } else if this.recovery.deadline.is_some() {
                                            this.start_reconnect(cx);
                                        } else {
                                            this.reconnect(window, cx);
                                        }
                                    })),
                            )
                            .child(
                                Button::new("recovery-stop")
                                    .ghost()
                                    .label(if automatic { "停止" } else { "关闭" })
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if automatic {
                                            this.recovery.stop();
                                            if this.phase == Phase::Connecting {
                                                this.disconnect(cx);
                                            }
                                            cx.notify();
                                        } else {
                                            this.close_window(window, cx);
                                        }
                                    })),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::screenshot_png;
    #[test]
    fn screenshot_preserves_colors_and_rejects_missing_frames() {
        let frame = rv_session::Framebuffer {
            width: 1,
            height: 1,
            pixels: vec![3, 2, 1, 255],
            generation: 1,
        };
        let png = screenshot_png(frame).unwrap();
        assert_eq!(
            image::load_from_memory(&png).unwrap().into_rgba8().as_raw(),
            &[1, 2, 3, 255]
        );
        assert!(screenshot_png(rv_session::Framebuffer::default()).is_err());
    }
}
