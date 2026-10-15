//! Connection editor and connection-specific options.
use super::super::AddressBookApp;
use super::widgets::{field, setting_row};
use crate::theme;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::button::ButtonVariants as _;
use gpui_component::{Disableable as _, button::Button, v_flex};
use gpui_component::{input::Input, switch::Switch};
use rv_core::{EncryptionMode, LocalCursorMode, QualityPreset};
impl AddressBookApp {
    pub(super) fn render_connection_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_3()
            .when_some(self.connection_error.as_ref(), |form, error| {
                form.child(
                    div()
                        .id("management-feedback")
                        .debug_selector(|| "management-feedback".into())
                        .w_full()
                        .mb_2()
                        .text_xs()
                        .text_color(theme::danger(cx))
                        .child(error.clone()),
                )
            })
            .child(field(
                "VNC 地址",
                div()
                    .w_full()
                    .id("vnc-address-input")
                    .debug_selector(|| "vnc-address-input".into())
                    .child(Input::new(&self.server_input)),
                cx,
            ))
            .child(field(
                "ZXTouch 端口",
                div()
                    .w_full()
                    .id("zxtouch-port-input")
                    .debug_selector(|| "zxtouch-port-input".into())
                    .child(Input::new(&self.zxtouch_port_input)),
                cx,
            ))
            .child(field("名称", Input::new(&self.name_input), cx))
            .child(field("用户名", Input::new(&self.username_input), cx))
            .child(field("密码", Input::new(&self.password_input), cx))
            .when(self.editing.is_some() && self.remember_password, |this| {
                this.child(div().child(crate::typography::caption("留空则保留已保存的密码。", cx)))
            })
            .child(field("标签", Input::new(&self.labels_input), cx))
            .child(connection_options(self, cx))
            .into_any_element()
    }
    pub(super) fn render_connection_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        gpui_component::dialog::DialogFooter::new()
            .child(
                Button::new("modal-cancel")
                    .debug_selector(|| "modal-cancel".into())
                    .disabled(self.credential_busy)
                    .label("取消")
                    .on_click(cx.listener(|this, _, _, cx| this.close_modal(cx))),
            )
            .child(
                Button::new("modal-save")
                    .debug_selector(|| "modal-save".into())
                    .disabled(self.credential_busy)
                    .outline()
                    .label("保存")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.submit_connect(false, window, cx)),
                    ),
            )
            .child(
                Button::new("modal-connect")
                    .primary()
                    .label("保存并连接")
                    .debug_selector(|| "modal-connect".into())
                    .disabled(self.credential_busy)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.submit_connect(true, window, cx)),
                    ),
            )
            .into_any_element()
    }
}
fn connection_options(app: &AddressBookApp, cx: &mut Context<AddressBookApp>) -> impl IntoElement {
    v_flex()
        .gap_2()
        .pt_2()
        .border_t_1()
        .border_color(theme::line(cx))
        .child(setting_row(
            "加密方式",
            Button::new("enc")
                .outline()
                .label(match app.encryption {
                    EncryptionMode::LetServerChoose => "由服务器决定",
                    EncryptionMode::PreferOn => "优先加密",
                    EncryptionMode::Always => "始终加密",
                    EncryptionMode::Off => "关闭",
                })
                .tooltip("点击切换")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.encryption = this.encryption.cycle();
                    cx.notify();
                })),
            cx,
        ))
        .child(setting_row(
            "画面质量",
            Button::new("qual")
                .outline()
                .label(match app.quality {
                    QualityPreset::Auto => "自动",
                    QualityPreset::Best => "最佳画质",
                    QualityPreset::Fast => "优先速度",
                })
                .tooltip("点击切换")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.quality = this.quality.cycle();
                    cx.notify();
                })),
            cx,
        ))
        .child(setting_row(
            "剪贴板编码",
            Button::new("clipboard")
                .debug_selector(|| "clipboard-mode".into())
                .outline()
                .label(app.clipboard_mode.label())
                .tooltip("切换 UTF-8 扩展剪贴板与 Latin-1")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.clipboard_mode = this.clipboard_mode.cycle();
                    cx.notify();
                })),
            cx,
        ))
        .child(setting_row(
            "本地光标",
            Button::new("cursor")
                .debug_selector(|| "cursor-mode".into())
                .outline()
                .label(match app.local_cursor_mode {
                    LocalCursorMode::Automatic => "自动",
                    LocalCursorMode::Show => "始终显示",
                    LocalCursorMode::Hide => "远程画面内隐藏",
                })
                .tooltip("点击切换")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.local_cursor_mode = this.local_cursor_mode.cycle();
                    cx.notify();
                })),
            cx,
        ))
        .child(
            v_flex()
                .gap_2()
                .child(setting_row(
                    "记住密码",
                    Switch::new("remember")
                        .checked(app.remember_password)
                        .on_click(cx.listener(|this, v: &bool, _, cx| {
                            this.remember_password = *v;
                            cx.notify();
                        })),
                    cx,
                ))
                .child(setting_row(
                    "仅查看",
                    Switch::new("viewonly")
                        .checked(app.view_only)
                        .on_click(cx.listener(|this, v: &bool, _, cx| {
                            this.view_only = *v;
                            cx.notify();
                        })),
                    cx,
                ))
                .child(setting_row(
                    "共享会话",
                    Switch::new("shared")
                        .checked(app.shared)
                        .on_click(cx.listener(|this, v: &bool, _, cx| {
                            this.shared = *v;
                            cx.notify();
                        })),
                    cx,
                )),
        )
}
