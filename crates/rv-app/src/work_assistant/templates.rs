use super::*;
use gpui::prelude::FluentBuilder;
use gpui_component::{
    Disableable, IconName,
    input::{Input, Textarea},
    menu::{DropdownMenu, PopupMenuItem},
};
use widgets::{field, selection_menu};
impl WorkAssistant {
    pub(super) fn template_dirty(&self, cx: &App) -> bool {
        self.template.name != self.template_name.read(cx).value().as_ref()
            || self.template.content != self.template_content.read(cx).value().as_ref()
            || self.template.category != self.template_category.read(cx).value().as_ref()
    }
    pub(super) fn select_template(
        &mut self,
        t: Template,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.template_name
            .update(cx, |v, cx| v.set_value(t.name.clone(), window, cx));
        self.template_content
            .update(cx, |v, cx| v.set_value(t.content.clone(), window, cx));
        self.template_category
            .update(cx, |v, cx| v.set_value(t.category.clone(), window, cx));
        self.template = t;
        self.confirm_template_delete = false;
        cx.notify();
    }
    fn save_template(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let t = Template {
            id: self.template.id.clone(),
            revision: self.template.revision,
            name: self.template_name.read(cx).value().to_string(),
            content: self.template_content.read(cx).value().to_string(),
            category: self.template_category.read(cx).value().to_string(),
        };
        let service = self.service.clone();
        self.busy = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    service.call(move |s| {
                        let t = s.save_template(&t)?;
                        Ok((t, s.templates()?))
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok((t, list)) => {
                        this.template = t;
                        this.templates = list;
                        this.message = "文案已保存到本地".into();
                        this.error = None;
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn delete_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let t = self.template.clone();
        let service = self.service.clone();
        self.busy = true;
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    service.call(move |s| {
                        s.delete_template(&t.id, t.revision)?;
                        s.templates()
                    })
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(list) => {
                        this.templates = list;
                        this.select_template(Template::default(), window, cx);
                        this.message = "文案已删除".into();
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn render_templates(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .w_full()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div().flex_1().min_w_0().child(
                            Button::new("template-picker")
                                .debug_selector(|| "template-picker".into())
                                .outline()
                                .w_full()
                                .min_w_0()
                                .child(div().flex_1().min_w_0().text_ellipsis().child(
                                    if self.template.name.is_empty() {
                                        "选择常用文案".to_owned()
                                    } else {
                                        self.template.name.clone()
                                    },
                                ))
                                .child(gpui_component::Icon::new(IconName::ChevronDown))
                                .disabled(
                                    self.busy
                                        || self.template_dirty(cx)
                                        || self.templates.is_empty(),
                                )
                                .dropdown_menu({
                                    let items = self.templates.clone();
                                    let entity = cx.entity();
                                    move |menu, window, _| {
                                        items.clone().into_iter().fold(
                                            selection_menu(menu, window),
                                            |menu, t| {
                                                let entity = entity.clone();
                                                menu.item(
                                                    PopupMenuItem::new(t.name.clone()).on_click(
                                                        move |_, window, cx| {
                                                            entity.update(cx, |this, cx| {
                                                                this.select_template(
                                                                    t.clone(),
                                                                    window,
                                                                    cx,
                                                                )
                                                            });
                                                        },
                                                    ),
                                                )
                                            },
                                        )
                                    }
                                }),
                        ),
                    )
                    .child(
                        Button::new("template-new")
                            .debug_selector(|| "template-new".into())
                            .outline()
                            .icon(IconName::Plus)
                            .tooltip("新增文案")
                            .disabled(self.busy || self.template_dirty(cx))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.select_template(Template::default(), window, cx)
                            })),
                    ),
            )
            .when(self.template_dirty(cx), |el| {
                el.child(
                    h_flex().gap_2().child("文案有未保存修改").child(
                        Button::new("template-discard")
                            .debug_selector(|| "template-discard".into())
                            .ghost()
                            .label("放弃修改")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.select_template(this.template.clone(), window, cx)
                            })),
                    ),
                )
            })
            .child(field("名称", Input::new(&self.template_name), cx))
            .child(field("分类", Input::new(&self.template_category), cx))
            .child(field(
                "文案内容",
                Textarea::new(&self.template_content)
                    .h(px(140.))
                    .overflow_hidden(),
                cx,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("template-save")
                            .primary()
                            .label("保存文案")
                            .debug_selector(|| "template-save".into())
                            .disabled(
                                self.busy
                                    || !self.template_dirty(cx)
                                    || self.template_name.read(cx).value().trim().is_empty()
                                    || self.template_content.read(cx).value().trim().is_empty(),
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.save_template(cx))),
                    )
                    .child(
                        Button::new("template-send")
                            .outline()
                            .label("发送到剪贴板")
                            .disabled(!self.writable || !self.connected)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.send(this.template_content.read(cx).value().to_string(), cx)
                            })),
                    ),
            )
            .child(
                h_flex().child(
                    Button::new("template-apply")
                        .debug_selector(|| "template-apply".into())
                        .outline()
                        .label("用作当前商品描述")
                        .disabled(
                            self.draft.is_none()
                                || self.busy
                                || self.template_content.read(cx).value().is_empty(),
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.apply_template(
                                this.template_content.read(cx).value().to_string(),
                                window,
                                cx,
                            )
                        })),
                ),
            )
            .when(self.template.revision > 0, |el| {
                el.child(
                    h_flex().child(
                        Button::new("template-delete")
                            .danger()
                            .outline()
                            .label("删除文案…")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.confirm_template_delete = true;
                                cx.notify();
                            })),
                    ),
                )
            })
            .when(self.confirm_template_delete, |el| {
                el.child(
                    h_flex()
                        .gap_2()
                        .child("删除当前文案？")
                        .child(
                            Button::new("template-delete-cancel")
                                .label("取消")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.confirm_template_delete = false;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("template-delete-confirm")
                                .danger()
                                .label("删除")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.delete_template(window, cx)
                                })),
                        ),
                )
            })
            .into_any_element()
    }
}
