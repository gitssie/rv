use super::*;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::{
    Disableable,
    input::{Input, Textarea},
    v_flex,
};
impl AddressBookApp {
    pub(in crate::app) fn app_editor_title(&self) -> &'static str {
        "编辑 App 信息"
    }
    pub(in crate::app) fn render_app_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.home.editing.is_none() {
            return div().into_any_element();
        }
        v_flex()
            .id("app-edit-form")
            .debug_selector(|| "app-edit-form".into())
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(crate::typography::label("名称", cx))
                    .child(Input::new(&self.home.title)),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(crate::typography::label("标签", cx))
                    .child(Input::new(&self.home.tags)),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(crate::typography::label("备注", cx))
                    .child(Textarea::new(&self.home.note)),
            )
            .child(self.render_proxy_binding(cx))
            .into_any_element()
    }
    pub(in crate::app) fn render_app_editor_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        gpui_component::dialog::DialogFooter::new()
            .child(
                Button::new("app-edit-cancel")
                    .label("取消")
                    .on_click(cx.listener(|this, _, _, cx| this.close_modal(cx))),
            )
            .child(
                Button::new("app-edit-save")
                    .primary()
                    .label("保存")
                    .disabled(self.read_only)
                    .on_click(cx.listener(|this, _, _, cx| this.save_app_card(cx))),
            )
            .into_any_element()
    }
}
