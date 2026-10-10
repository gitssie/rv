use super::*;
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
    pub(super) fn create_draft(
        &mut self,
        duplicate: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
    pub(super) fn delete_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
}
