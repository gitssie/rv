//! Compact draft editor; persistence and CRUD live in drafts.rs.
use super::*;
use gpui_component::{
    IconName,
    input::{Input, Textarea},
    menu::{DropdownMenu, PopupMenuItem},
};
use widgets::{field, selection_menu, send_button};

impl WorkAssistant {
    pub(super) fn apply_template(
        &mut self,
        content: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.description
            .update(cx, |input, cx| input.set_value(content, window, cx));
        // Programmatic set_value intentionally does not emit InputEvent::Change.
        self.capture(cx);
        self.tab = Tab::Drafts;
        cx.notify();
    }

    pub(super) fn render_drafts(&self, cx: &mut Context<Self>) -> AnyElement {
        let locked = self.busy || self.save_busy || self.dirty || self.import_busy;
        let title = self
            .draft
            .as_ref()
            .map(|d| {
                if d.title.is_empty() {
                    "未命名商品".to_owned()
                } else {
                    d.title.clone()
                }
            })
            .unwrap_or("选择商品草稿".into());
        v_flex().w_full().gap_3()
            .child(h_flex().w_full().gap_2()
                .child(div().flex_1().min_w_0().child(Button::new("draft-picker").debug_selector(|| "draft-picker".into()).outline().w_full().min_w_0()
                    .child(div().flex_1().min_w_0().text_ellipsis().child(title))
                    .child(gpui_component::Icon::new(IconName::ChevronDown)).disabled(locked || self.drafts.is_empty())
                    .dropdown_menu({
                        let items = self.drafts.clone(); let entity = cx.entity();
                        move |menu, window, _| items.clone().into_iter().fold(selection_menu(menu, window), |menu, d| {
                            let label = if d.title.is_empty() { "未命名商品".into() } else { d.title.clone() };
                            let entity = entity.clone();
                            menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                                entity.update(cx, |this, cx| this.select_draft(d.clone(), window, cx));
                            }))
                        })
                    })))
                .child(Button::new("draft-new").debug_selector(|| "draft-new".into()).outline()
                    .icon(IconName::Plus).tooltip("新增商品草稿").disabled(locked)
                    .on_click(cx.listener(|this, _, window, cx| this.create_draft(false, window, cx))))
                .child(Button::new("draft-actions").debug_selector(|| "draft-actions".into()).ghost().icon(IconName::Ellipsis)
                    .tooltip("草稿操作").disabled(locked || self.draft.is_none()).dropdown_menu({
                        let entity = cx.entity();
                        move |menu, _, _| {
                            let copy = entity.clone(); let delete = entity.clone();
                            menu.item(PopupMenuItem::new("复制草稿").on_click(move |_, window, cx| {
                                copy.update(cx, |this, cx| this.create_draft(true, window, cx));
                            })).item(PopupMenuItem::new("删除草稿…").on_click(move |_, _, cx| {
                                delete.update(cx, |this, cx| { this.confirm_delete = true; cx.notify(); });
                            }))
                        }
                    })))
            .when(self.draft.is_none(), |el| el.child(v_flex().gap_2().py_4()
                .child(div().font_semibold().child("创建第一个商品草稿"))
                .child(div().text_xs().whitespace_normal().text_color(theme::muted(cx)).child("点击上方 ＋，整理商品图片与文案，再传到手机使用。"))))
            .when(self.draft.is_some(), |el| {
                el.child(self.render_images(cx))
                    .child(v_flex().gap_1()
                        .child(crate::typography::label("标题", cx))
                        .child(h_flex().gap_2().items_start()
                            .child(div().flex_1().min_w_0().child(Input::new(&self.title)))
                            .child(send_button("send-title", self.connected && self.writable && !self.title.read(cx).value().is_empty())
                                .on_click(cx.listener(|this, _, _, cx| this.send(this.title.read(cx).value().to_string(), cx))))))
                    .child(v_flex().gap_1()
                        .child(h_flex().justify_between()
                            .child(crate::typography::label("描述", cx))
                            .child(Button::new("use-template").debug_selector(|| "use-template".into()).ghost()
                                .label("使用文案").disabled(self.templates.is_empty()).dropdown_menu({
                                    let items = self.templates.clone(); let entity = cx.entity();
                                    move |menu, window, _| items.clone().into_iter().fold(selection_menu(menu, window), |menu, t| {
                                        let entity = entity.clone();
                                        menu.item(PopupMenuItem::new(t.name).on_click(move |_, window, cx| {
                                            entity.update(cx, |this, cx| this.apply_template(t.content.clone(), window, cx));
                                        }))
                                    })
                                })))
                        .child(h_flex().gap_2().items_start()
                            .child(div().flex_1().min_w_0().child(Textarea::new(&self.description).h(px(96.)).overflow_hidden()))
                            .child(send_button("send-description", self.connected && self.writable && !self.description.read(cx).value().is_empty())
                                .on_click(cx.listener(|this, _, _, cx| this.send(this.description.read(cx).value().to_string(), cx))))))
                    .child(h_flex().debug_selector(|| "draft-price-stock".into()).gap_3().items_start()
                        .child(div().flex_1().min_w_0().child(field("价格（EUR）",
                            h_flex().gap_1().child(div().flex_1().min_w_0().child(Input::new(&self.price)))
                                .child(Button::new("send-price").ghost().icon(IconName::Copy)
                                    .tooltip("发送价格到手机剪贴板").disabled(!self.writable || !self.connected || self.price_invalid || self.draft.as_ref().is_none_or(|d| d.price_minor.is_none()))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(d) = &this.draft { this.send(format_price(d.price_minor), cx); }
                                    }))), cx)))
                        .child(div().flex_1().min_w_0().child(field("库存备注 · 本地", Input::new(&self.stock), cx))))
                    .child(div().text_xs().whitespace_normal().text_color(theme::muted(cx)).child("发送后，在手机输入框内粘贴。"))
                    .when(self.confirm_delete, |el| el.child(v_flex().p_2().gap_2().border_1().border_color(theme::line(cx)).rounded(theme::radius(cx))
                        .child(div().text_xs().whitespace_normal().child("删除本地草稿？手机相册中的照片会保留。"))
                        .child(h_flex().gap_2()
                            .child(Button::new("draft-delete-cancel").label("取消").on_click(cx.listener(|this, _, _, cx| { this.confirm_delete = false; cx.notify(); })))
                            .child(Button::new("draft-delete-confirm").debug_selector(|| "draft-delete-confirm".into()).danger().label("确认删除").disabled(locked)
                                .on_click(cx.listener(|this, _, window, cx| this.delete_draft(window, cx)))))))
            }).into_any_element()
    }
}
