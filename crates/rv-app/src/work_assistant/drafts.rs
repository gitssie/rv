use super::*;
use gpui::prelude::FluentBuilder;
use gpui_component::{
    Disableable,
    input::{Input, Textarea},
    menu::{DropdownMenu, PopupMenuItem},
};
use widgets::{field, primary};
impl WorkAssistant {
    pub(super) fn save(&mut self, cx: &mut Context<Self>) {
        if self.save_busy || self.save_error.is_some() {
            return;
        }
        let Some(draft) = self.draft.clone() else {
            return;
        };
        self.dirty = false;
        self.save_busy = true;
        let service = self.service.clone();
        let pending = persistence::PendingSave::default();
        self.pending_save = Some(pending.clone());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let result = service.call(move |s| s.save_draft(&draft));
                    pending.finish(&result, &service);
                    result
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.save_busy = false;
                this.pending_save = None;
                match result {
                    Ok(saved) => {
                        if let Some(d) = &mut this.draft
                            && d.id == saved.id
                        {
                            d.revision = saved.revision;
                            d.updated_at = saved.updated_at;
                        }
                        if let Some(d) = this.drafts.iter_mut().find(|d| d.id == saved.id) {
                            *d = saved;
                        }
                        this.message = "已保存到本地".into();
                    }
                    Err(e) => {
                        this.dirty = true;
                        this.save_error = Some(e);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn select_draft(
        &mut self,
        draft: Draft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.draft = None;
        self.title
            .update(cx, |v, cx| v.set_value(draft.title.clone(), window, cx));
        self.description.update(cx, |v, cx| {
            v.set_value(draft.description.clone(), window, cx)
        });
        self.price.update(cx, |v, cx| {
            v.set_value(
                draft
                    .price_input
                    .clone()
                    .unwrap_or_else(|| format_price(draft.price_minor)),
                window,
                cx,
            )
        });
        self.stock.update(cx, |v, cx| {
            v.set_value(draft.stock_note.clone(), window, cx)
        });
        let id = draft.id.clone();
        self.price_invalid = draft
            .price_input
            .as_ref()
            .is_some_and(|v| parse_price(v).is_err());
        self.draft = Some(draft);
        self.images.clear();
        self.dirty = false;
        self.error = None;
        self.save_error = None;
        self.confirm_delete = false;
        self.retry_import_available = false;
        self.confirm_import_retry = false;
        self.message = "已保存到本地".into();
        let service = self.service.clone();
        let requested_id = id.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { service.call(move |s| s.images(&id)) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if !this.draft.as_ref().is_some_and(|d| d.id == requested_id) {
                    return;
                }
                match result {
                    Ok(images) => {
                        this.images = images;
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn create_draft(&mut self, duplicate: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.save_busy || self.dirty {
            return;
        }
        let current = self.draft.as_ref().map(|d| d.id.clone());
        let service = self.service.clone();
        self.busy = true;
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    service.call(move |s| {
                        let draft = if duplicate {
                            s.duplicate(&current.ok_or_else(|| {
                                rv_core::assistant::StoreError::Invalid("没有选中草稿".into())
                            })?)?
                        } else {
                            s.save_draft(&Draft::default())?
                        };
                        Ok((draft, s.list_drafts()?))
                    })
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok((draft, list)) => {
                        this.drafts = list;
                        this.select_draft(draft, window, cx);
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn delete_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.save_busy || self.dirty || self.import_busy {
            return;
        }
        let Some(d) = self.draft.clone() else {
            return;
        };
        let service = self.service.clone();
        self.busy = true;
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    service.call(move |s| {
                        s.delete_draft(&d.id, d.revision)?;
                        s.list_drafts()
                    })
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                this.confirm_delete = false;
                match result {
                    Ok(list) => {
                        this.drafts = list;
                        this.draft = None;
                        this.images.clear();
                        if let Some(d) = this.drafts.first().cloned() {
                            this.select_draft(d, window, cx);
                        }
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
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
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("draft-picker")
                            .outline()
                            .small()
                            .label(title)
                            .disabled(locked)
                            .dropdown_menu({
                                let items = self.drafts.clone();
                                let entity = cx.entity();
                                move |menu, _, _| {
                                    items.clone().into_iter().fold(menu, |menu, d| {
                                        let label = if d.title.is_empty() {
                                            "未命名商品".into()
                                        } else {
                                            d.title.clone()
                                        };
                                        let entity = entity.clone();
                                        menu.item(PopupMenuItem::new(label).on_click(
                                            move |_, window, cx| {
                                                entity.update(cx, |this, cx| {
                                                    this.select_draft(d.clone(), window, cx)
                                                })
                                            },
                                        ))
                                    })
                                }
                            }),
                    )
                    .child(
                        Button::new("draft-new")
                            .debug_selector(|| "draft-new".into())
                            .outline()
                            .small()
                            .label("＋新增")
                            .disabled(locked)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.create_draft(false, window, cx)
                            })),
                    )
                    .child(
                        Button::new("draft-copy")
                            .ghost()
                            .small()
                            .label("复制")
                            .disabled(locked || self.draft.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.create_draft(true, window, cx)
                            })),
                    ),
            )
            .when(self.draft.is_none(), |el| {
                el.child(
                    div()
                        .text_color(theme::muted(cx))
                        .child("新增草稿，在 Mac 整理图片与文案，再传到手机使用。"),
                )
            })
            .when(self.draft.is_some(), |el| {
                el.child(self.render_images(cx))
                    .child(
                        h_flex()
                            .justify_between()
                            .child(div().text_xs().text_color(theme::muted(cx)).child("标题"))
                            .child(
                                Button::new("send-title")
                                    .ghost()
                                    .small()
                                    .label("发送到剪贴板")
                                    .disabled(!self.writable || !self.connected)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.send(this.title.read(cx).value().to_string(), cx)
                                    })),
                            ),
                    )
                    .child(Input::new(&self.title).small())
                    .child(field(
                        "描述",
                        Textarea::new(&self.description).h(px(110.)),
                        cx,
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("use-template")
                                    .outline()
                                    .small()
                                    .label("使用文案")
                                    .dropdown_menu({
                                        let items = self.templates.clone();
                                        let input = self.description.clone();
                                        move |menu, _, _| {
                                            items.clone().into_iter().fold(menu, |menu, t| {
                                                let input = input.clone();
                                                menu.item(PopupMenuItem::new(t.name).on_click(
                                                    move |_, window, cx| {
                                                        input.update(cx, |v, cx| {
                                                            v.set_value(
                                                                t.content.clone(),
                                                                window,
                                                                cx,
                                                            )
                                                        })
                                                    },
                                                ))
                                            })
                                        }
                                    }),
                            )
                            .child(
                                primary("send-description", "发送到剪贴板", cx)
                                    .disabled(!self.writable || !self.connected)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.send(this.description.read(cx).value().to_string(), cx)
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::muted(cx))
                                    .child("价格（EUR）"),
                            )
                            .child(
                                Button::new("send-price")
                                    .ghost()
                                    .small()
                                    .label("发送价格")
                                    .disabled(
                                        !self.writable || !self.connected || self.price_invalid,
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(d) = &this.draft {
                                            this.send(format_price(d.price_minor), cx);
                                        }
                                    })),
                            ),
                    )
                    .child(Input::new(&self.price).small())
                    .child(field(
                        "库存备注 · 仅本地保存",
                        Input::new(&self.stock).small(),
                        cx,
                    ))
                    .child(
                        div()
                            .text_xs()
                            .whitespace_normal()
                            .text_color(theme::muted(cx))
                            .child("发送后请点击手机中的输入框并粘贴。发布仍在手机 App 内完成。"),
                    )
                    .child(
                        Button::new("draft-delete")
                            .danger()
                            .outline()
                            .small()
                            .label("删除草稿…")
                            .disabled(locked)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.confirm_delete = true;
                                cx.notify();
                            })),
                    )
                    .when(self.confirm_delete, |el| {
                        el.child(
                            v_flex()
                                .gap_2()
                                .child("删除本地草稿？不会删除手机相册中的照片。")
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            Button::new("draft-delete-cancel")
                                                .small()
                                                .label("取消")
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.confirm_delete = false;
                                                    cx.notify();
                                                })),
                                        )
                                        .child(
                                            Button::new("draft-delete-confirm")
                                                .danger()
                                                .small()
                                                .label("确认删除")
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.delete_draft(window, cx)
                                                })),
                                        ),
                                ),
                        )
                    })
            })
            .into_any_element()
    }
}
