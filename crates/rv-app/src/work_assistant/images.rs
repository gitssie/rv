use super::*;
use gpui::prelude::FluentBuilder;
use gpui_component::{Disableable, Icon, IconName};
#[derive(Clone)]
struct DraggedImage(String);
impl WorkAssistant {
    fn add_paths(&mut self, paths: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        if self.busy || self.import_busy {
            return;
        }
        let Some(id) = self.draft.as_ref().map(|d| d.id.clone()) else {
            return;
        };
        let service = self.service.clone();
        self.busy = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    service.call(move |s| {
                        let mut errors = Vec::new();
                        for path in paths {
                            if let Err(e) = s.add_image(&id, &path) {
                                errors.push(format!(
                                    "{}：{e}",
                                    path.file_name().unwrap_or_default().to_string_lossy()
                                ));
                            }
                        }
                        Ok((s.images(&id)?, errors))
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok((images, errors)) => {
                        this.images = images;
                        if !errors.is_empty() {
                            this.error = Some(errors.join("；"));
                        } else {
                            this.error = None;
                            this.message = "图片已加入本地草稿".into();
                        }
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn pick_images(&mut self, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("添加商品图片".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picker.await {
                let _ = this.update(cx, |this, cx| this.add_paths(paths, cx));
            }
        })
        .detach();
    }
    fn reorder(&mut self, source: &str, target: &str, cx: &mut Context<Self>) {
        if self.busy || self.import_busy {
            return;
        }
        let Some(id) = self.draft.as_ref().map(|d| d.id.clone()) else {
            return;
        };
        let mut ids: Vec<_> = self.images.iter().map(|i| i.id.clone()).collect();
        let Some(from) = ids.iter().position(|i| i == source) else {
            return;
        };
        let Some(to) = ids.iter().position(|i| i == target) else {
            return;
        };
        let item = ids.remove(from);
        ids.insert(to, item);
        let service = self.service.clone();
        self.busy = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    service.call(move |s| {
                        s.reorder_images(&id, &ids)?;
                        s.images(&id)
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(images) => this.images = images,
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn remove_image(&mut self, image: String, cx: &mut Context<Self>) {
        if self.busy || self.import_busy {
            return;
        }
        let Some(id) = self.draft.as_ref().map(|d| d.id.clone()) else {
            return;
        };
        let service = self.service.clone();
        self.busy = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    service.call(move |s| {
                        s.remove_image(&image)?;
                        s.images(&id)
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(images) => this.images = images,
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn render_images(&self, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.busy || self.import_busy;
        v_flex()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(theme::muted(cx))
                    .child("商品图片 · 拖动调整顺序"),
            )
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .on_drop(cx.listener(|this, files: &ExternalPaths, _, cx| {
                        this.add_paths(files.paths().to_vec(), cx)
                    }))
                    .children(self.images.iter().enumerate().map(|(index, image)| {
                        let id = image.id.clone();
                        let drag = id.clone();
                        let target = id.clone();
                        let remove = id.clone();
                        let path = self.service.root.join(&image.relative_path);
                        v_flex()
                            .id(SharedString::from(format!("draft-image-{id}")))
                            .w(px(72.))
                            .gap_1()
                            .child(
                                div()
                                    .id(SharedString::from(format!("image-drag-{id}")))
                                    .size(px(72.))
                                    .relative()
                                    .rounded_md()
                                    .overflow_hidden()
                                    .border_1()
                                    .border_color(theme::line(cx))
                                    .on_drag(DraggedImage(drag), |drag, _, _, cx| {
                                        cx.new(|_| ImageDragView(drag.0.clone()))
                                    })
                                    .on_drop(cx.listener(
                                        move |this, drag: &DraggedImage, _, cx| {
                                            this.reorder(&drag.0, &target, cx)
                                        },
                                    ))
                                    .child(
                                        if path
                                            .extension()
                                            .is_some_and(|e| e == "heic" || e == "heif")
                                        {
                                            div()
                                                .size_full()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .child(Icon::new(IconName::File))
                                                .into_any_element()
                                        } else {
                                            img(path)
                                                .size_full()
                                                .object_fit(ObjectFit::Cover)
                                                .into_any_element()
                                        },
                                    ),
                            )
                            .child(
                                h_flex()
                                    .justify_between()
                                    .child(div().text_xs().child(format!("{}", index + 1)))
                                    .child(
                                        Button::new(SharedString::from(format!("remove-{id}")))
                                            .ghost()
                                            .xsmall()
                                            .label("×")
                                            .disabled(busy)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.remove_image(remove.clone(), cx)
                                            })),
                                    ),
                            )
                    }))
                    .child(
                        Button::new("add-draft-images")
                            .outline()
                            .small()
                            .w(px(72.))
                            .h(px(72.))
                            .label("＋")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.pick_images(cx))),
                    ),
            )
            .child(
                Button::new("import-draft-images")
                    .debug_selector(|| "import-draft-images".into())
                    .outline()
                    .small()
                    .label(if self.import_busy {
                        "导入中…"
                    } else {
                        "导入手机相册"
                    })
                    .disabled(busy || !self.writable || !self.connected || self.images.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.writable && this.connected && !this.import_busy {
                            cx.emit(AssistantEvent::Import(this.images.clone()));
                        }
                    })),
            )
            .when(self.retry_import_available && !self.import_busy, |el| {
                el.child(
                    v_flex()
                        .gap_2()
                        .child("手机没有原任务记录，请先核实相册中没有这些图片。")
                        .child(
                            Button::new("photo-recovery")
                                .outline()
                                .small()
                                .label("核实后恢复导入…")
                                .disabled(busy || !self.writable || !self.connected)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.confirm_import_retry = true;
                                    cx.notify();
                                })),
                        )
                        .when(self.confirm_import_retry, |el| {
                            el.child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new("photo-recovery-cancel")
                                            .ghost()
                                            .small()
                                            .label("取消")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.confirm_import_retry = false;
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new("photo-recovery-confirm")
                                            .outline()
                                            .small()
                                            .label("确认未导入，恢复")
                                            .disabled(busy || !self.writable || !self.connected)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                if this.connected
                                                    && this.writable
                                                    && !this.import_busy
                                                {
                                                    this.confirm_import_retry = false;
                                                    cx.emit(AssistantEvent::RetryImport(
                                                        this.images.clone(),
                                                    ));
                                                }
                                            })),
                                    ),
                            )
                        }),
                )
            })
            .when(self.import_busy, |el| {
                el.child(
                    Button::new("cancel-draft-import")
                        .ghost()
                        .small()
                        .label("停止后续导入")
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(AssistantEvent::CancelImport))),
                )
            })
            .child(
                div()
                    .text_xs()
                    .text_color(theme::muted(cx))
                    .child("导入完成后，在手机 App 内选择照片。"),
            )
            .into_any_element()
    }
}
struct ImageDragView(String);
impl Render for ImageDragView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .p_2()
            .bg(rgb(0x233247))
            .text_color(rgb(0xffffff))
            .child(format!("移动图片 {}", &self.0[..8]))
    }
}
