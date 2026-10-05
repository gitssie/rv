use super::*;
use crate::assets::AppActionIcon;
use gpui_component::popover::Popover;
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
        tool_btn(
            "home-screen",
            AppActionIcon::Home,
            "回到主屏幕",
            cx.listener(|this, _, _, _| {
                if this.phase == Phase::Connected
                    && !this.view_only()
                    && !this.unlock.blocks_input()
                {
                    this.handle.app(AppCommand::Home);
                }
            }),
        )
        .disabled(
            !supported
                || self.phase != Phase::Connected
                || self.view_only()
                || self.unlock.blocks_input(),
        )
    }

    pub(super) fn render_screenshot(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        Popover::new("screenshot-menu")
            .anchor(Anchor::TopLeft)
            .trigger(
                Button::new("screenshot")
                    .ghost()
                    .size(px(28.))
                    .icon(AppActionIcon::Camera)
                    .text_color(theme::toolbar_fg())
                    .tooltip("截图")
                    .accessibility_label("截图")
                    .disabled(
                        self.phase != Phase::Connected
                            || self.screenshot_busy
                            || self.render_image.is_none(),
                    ),
            )
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let save_popover = popover.clone();
                entity.update(cx, |this, cx| {
                    v_flex()
                        .w(px(170.))
                        .gap_0p5()
                        .child(
                            menu_item(
                                "copy-screenshot",
                                IconName::Copy,
                                "复制截图",
                                cx.listener(move |this, _, window, cx| {
                                    let _ =
                                        popover.update(cx, |state, cx| state.dismiss(window, cx));
                                    this.screenshot(false, cx);
                                }),
                            )
                            .disabled(this.screenshot_busy),
                        )
                        .child(
                            menu_item(
                                "save-screenshot",
                                IconName::ArrowDown,
                                "保存截图…",
                                cx.listener(move |this, _, window, cx| {
                                    let _ = save_popover
                                        .update(cx, |state, cx| state.dismiss(window, cx));
                                    this.screenshot(true, cx);
                                }),
                            )
                            .disabled(this.screenshot_busy),
                        )
                        .child(
                            div()
                                .text_xs()
                                .px_2()
                                .py_1()
                                .text_color(theme::toolbar_muted())
                                .child("仅截取远程画面"),
                        )
                        .into_any_element()
                })
            })
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
            .child(tool_btn(
                "phone-notifications",
                IconName::Bell,
                "手机通知",
                cx.listener(|this, _, _, cx| {
                    this.notifications.open = !this.notifications.open;
                    if this.notifications.open {
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
            ))
            .when(self.notifications.unread > 0, |el| {
                el.child(
                    div()
                        .absolute()
                        .top_0()
                        .right_0()
                        .size(px(5.))
                        .rounded_full()
                        .bg(rgb(0x79b8ff)),
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
        v_flex()
            .id("phone-notification-panel")
            .occlude()
            .absolute()
            .top_2()
            .right_2()
            .w(px(320.))
            .max_w_full()
            .max_h(px(420.))
            .rounded_md()
            .bg(theme::toolbar())
            .border_1()
            .border_color(theme::toolbar_line())
            .shadow_lg()
            .text_color(theme::toolbar_fg())
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
                            .child(format!("手机通知 · {count}")),
                    )
                    .child(
                        tool_btn(
                            "refresh-notifications",
                            AppActionIcon::Restart,
                            "刷新已接收的通知",
                            cx.listener(|this, _, _, _| this.handle.app(AppCommand::Notifications)),
                        )
                        .disabled(!supported),
                    )
                    .child(tool_btn(
                        "close-notifications",
                        IconName::Close,
                        "关闭通知列表",
                        cx.listener(|this, _, _, cx| {
                            this.notifications.open = false;
                            cx.notify();
                        }),
                    )),
            )
            .child(
                h_flex().px_3().pb_2().gap_2().child(
                    Checkbox::new("auto-notifications")
                        .checked(self.notifications.enabled)
                        .label("自动接收")
                        .disabled(!supported)
                        .on_click(cx.listener(|this, enabled: &bool, _, cx| {
                            this.notifications.enabled = *enabled;
                            this.handle
                                .app(AppCommand::SubscribeNotifications(*enabled));
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
                        .text_color(theme::toolbar_muted())
                        .child(if self.phase == Phase::Connected {
                            "此设备未提供通知监听组件"
                        } else {
                            "连接已断开，显示上次接收的通知"
                        }),
                )
            })
            .when_some(self.notifications.error.clone(), |el, error| {
                el.child(
                    div()
                        .px_3()
                        .text_xs()
                        .text_color(rgb(0xe9a11b))
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
                    .when(count == 0, |el| {
                        el.child(
                            div()
                                .py_4()
                                .text_sm()
                                .text_color(theme::toolbar_muted())
                                .child("暂无已接收的通知"),
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
                                                .format("%m-%d %H:%M")
                                                .to_string()
                                        })
                                        .unwrap_or_default();
                                v_flex()
                                    .py_2()
                                    .gap_1()
                                    .border_b_1()
                                    .border_color(theme::toolbar_line())
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .text_xs()
                                                    .text_ellipsis()
                                                    .text_color(theme::toolbar_muted())
                                                    .child(notification.source.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(theme::toolbar_muted())
                                                    .child(time),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_medium()
                                            .child(notification.title.clone()),
                                    )
                                    .when(!notification.subtitle.is_empty(), |el| {
                                        el.child(
                                            div().text_xs().child(notification.subtitle.clone()),
                                        )
                                    })
                                    .child(div().text_sm().child(notification.message.clone()))
                            }),
                    ),
            )
            .child(
                div()
                    .px_3()
                    .py_2()
                    .text_xs()
                    .text_color(theme::toolbar_muted())
                    .child("最近 100 条已接收通知 · 非完整通知中心历史"),
            )
    }

    pub(super) fn render_recovery(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("session-recovery")
            .occlude()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::scrim())
            .child(
                v_flex()
                    .w(px(280.))
                    .max_w_full()
                    .p_3()
                    .gap_2()
                    .rounded_md()
                    .bg(theme::toolbar())
                    .border_1()
                    .border_color(theme::toolbar_line())
                    .text_color(theme::toolbar_fg())
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(Icon::new(IconName::Loader).small())
                            .child(div().text_sm().font_semibold().child(
                                if self.phase == Phase::Connecting {
                                    "正在重连"
                                } else {
                                    "连接中断"
                                },
                            )),
                    )
                    .child(div().text_xs().text_color(theme::toolbar_muted()).child(
                        if self.phase == Phase::Connecting {
                            format!("第 {} 次尝试", self.recovery.attempt)
                        } else {
                            format!(
                                "{} 秒后重试 · 第 {} 次",
                                self.recovery.seconds(std::time::Instant::now()),
                                self.recovery.attempt
                            )
                        },
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .justify_end()
                            .child(
                                Button::new("recovery-now")
                                    .small()
                                    .ghost()
                                    .text_color(rgb(0x79b8ff))
                                    .label("立即重连")
                                    .disabled(self.phase == Phase::Connecting)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.start_reconnect(cx);
                                    })),
                            )
                            .child(
                                Button::new("recovery-stop")
                                    .small()
                                    .ghost()
                                    .text_color(theme::toolbar_fg())
                                    .label("停止")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.recovery.stop();
                                        if this.phase == Phase::Connecting {
                                            this.disconnect(cx);
                                        }
                                        cx.notify();
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
