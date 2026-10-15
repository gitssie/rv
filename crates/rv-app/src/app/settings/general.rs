//! Viewer preferences use the component library's standard Form and controls.
use super::*;
use gpui_component::{
    Disableable, StyledExt,
    form::{Field, Form},
    group_box::{GroupBox, GroupBoxVariants},
    select::Select,
    switch::Switch,
};
impl AddressBookApp {
    pub(super) fn render_general_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let prefs = self.book.prefs();
        v_flex()
            .id("settings-general")
            .debug_selector(|| "settings-general".into())
            .gap_6()
            .child(
                GroupBox::new()
                    .outline()
                    .child(div().font_semibold().child("界面与会话"))
                    .child(
                        Form::horizontal()
                            .child(
                                Field::new().label("外观").items_center().child(
                                    Select::new(&self.settings.theme)
                                        .accessibility_label("外观")
                                        .disabled(self.read_only),
                                ),
                            )
                            .child(
                                Field::new().label("默认缩放").items_center().child(
                                    Select::new(&self.settings.scale)
                                        .accessibility_label("默认缩放")
                                        .disabled(self.read_only),
                                ),
                            )
                            .child(
                                Field::new().label("会话菜单快捷键").items_center().child(
                                    Select::new(&self.settings.menu_key)
                                        .accessibility_label("会话菜单快捷键")
                                        .disabled(self.read_only),
                                ),
                            )
                            .child(
                                Field::new().label("固定会话工具栏").items_center().child(
                                    Switch::new("pin")
                                        .checked(prefs.pin_toolbar)
                                        .disabled(self.read_only)
                                        .on_click(cx.listener(|this, value: &bool, _, cx| {
                                            this.book.prefs_mut().pin_toolbar = *value;
                                            this.persist();
                                            cx.notify();
                                        })),
                                ),
                            )
                            .child(
                                Field::new().label("隐藏画面预览").items_center().child(
                                    Switch::new("hide")
                                        .checked(prefs.hide_screenshots)
                                        .disabled(self.read_only)
                                        .on_click(cx.listener(|this, value: &bool, _, cx| {
                                            this.book.prefs_mut().hide_screenshots = *value;
                                            this.persist();
                                            cx.notify();
                                        })),
                                ),
                            ),
                    ),
            )
            .child(
                GroupBox::new()
                    .outline()
                    .child(div().font_semibold().child("本地数据"))
                    .child(
                        Button::new("forget")
                            .danger()
                            .outline()
                            .icon(IconName::Delete)
                            .label("清除全部密码和预览")
                            .disabled(self.read_only || self.credential_busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.read_only || this.credential_busy {
                                    return;
                                }
                                match this.book.forget_sensitive() {
                                    Ok(()) => this.set_status("密码和预览已清除", cx),
                                    Err(error) => this.set_status(format!("无法清除：{error}"), cx),
                                }
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }
}
