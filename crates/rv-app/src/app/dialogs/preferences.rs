//! Global viewer preferences.
use super::super::AddressBookApp;
use super::widgets::setting_row;
use crate::theme;
use gpui::*;
use gpui_component::switch::Switch;
use gpui_component::{
    IconName, Sizable,
    button::{Button, ButtonVariants as _},
    v_flex,
};
use rv_core::{ScaleMode, ThemePref};
pub(super) fn prefs_body(
    app: &AddressBookApp,
    cx: &mut Context<AddressBookApp>,
) -> impl IntoElement {
    let prefs = app.book.prefs();
    let theme_pref = prefs.theme;
    let scale = prefs.default_scale;
    let pin = prefs.pin_toolbar;
    let hide = prefs.hide_screenshots;
    let menu_key = prefs.menu_key.to_ascii_uppercase();
    v_flex()
        .gap_3()
        .child(setting_row(
            "外观",
            Button::new("theme")
                .outline()
                .small()
                .icon(if theme::is_dark(cx) {
                    IconName::Moon
                } else {
                    IconName::Sun
                })
                .label(match theme_pref {
                    ThemePref::System => "跟随系统",
                    ThemePref::Light => "浅色",
                    ThemePref::Dark => "深色",
                })
                .on_click(cx.listener(|this, _, window, cx| {
                    let next = this.book.prefs().theme.cycle();
                    this.book.prefs_mut().theme = next;
                    this.persist();
                    theme::apply(next, window, cx);
                    cx.notify();
                })),
            cx,
        ))
        .child(setting_row(
            "默认缩放",
            Button::new("scale")
                .outline()
                .small()
                .label(match scale {
                    ScaleMode::Fit => "适应窗口",
                    ScaleMode::Actual => "100%",
                    ScaleMode::Stretch => "拉伸",
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    let next = this.book.prefs().default_scale.cycle();
                    this.book.prefs_mut().default_scale = next;
                    this.persist();
                    cx.notify();
                })),
            cx,
        ))
        .child(setting_row(
            "会话菜单快捷键",
            Button::new("menukey")
                .outline()
                .small()
                .label(menu_key)
                .on_click(cx.listener(|this, _, _, cx| {
                    let next = match this.book.prefs().menu_key.as_str() {
                        "f8" => "f7",
                        "f7" => "f12",
                        _ => "f8",
                    };
                    this.book.prefs_mut().menu_key = next.into();
                    this.persist();
                    cx.notify();
                })),
            cx,
        ))
        .child(setting_row(
            "固定会话工具栏",
            Switch::new("pin")
                .small()
                .checked(pin)
                .on_click(cx.listener(|this, v: &bool, _, cx| {
                    this.book.prefs_mut().pin_toolbar = *v;
                    this.persist();
                    cx.notify();
                })),
            cx,
        ))
        .child(setting_row(
            "隐藏画面预览",
            Switch::new("hide")
                .small()
                .checked(hide)
                .on_click(cx.listener(|this, v: &bool, _, cx| {
                    this.book.prefs_mut().hide_screenshots = *v;
                    this.persist();
                    cx.notify();
                })),
            cx,
        ))
        .child(
            div()
                .pt_2()
                .border_t_1()
                .border_color(theme::line(cx))
                .child(
                    Button::new("forget")
                        .danger()
                        .outline()
                        .small()
                        .label("清除全部密码和预览")
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.credential_busy {
                                return;
                            }
                            match this.book.forget_sensitive() {
                                Ok(()) => this.status = "密码和预览已清除".into(),
                                Err(e) => this.status = format!("无法清除：{e}").into(),
                            }
                            cx.notify();
                        })),
                ),
        )
}
