//! Local card deletion. Shadow owns phone backup management.
use super::*;
use gpui::prelude::FluentBuilder;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::{
    Disableable,
    dialog::{AlertDialog, DialogFooter},
};

pub(in crate::app) struct AppDeletion {
    pub id: AppId,
    pub title: String,
    pub phone: bool,
    pub busy: bool,
    pub error: Option<String>,
    started: bool,
    remaining: Vec<BackupLink>,
}
impl AddressBookApp {
    pub(in crate::app) fn open_app_delete(&mut self, id: AppId, cx: &mut Context<Self>) {
        if self.read_only
            || self.home.jobs.contains(&id)
            || self.home.backup_syncing.contains_key(&id)
        {
            return;
        }
        let Ok(app) = WorkspaceStore::open(self.book.paths()).and_then(|store| store.load(&id))
        else {
            return;
        };
        self.home.deletion = Some(AppDeletion {
            id,
            title: self
                .home
                .plugins
                .for_bundle(&app.bundle_id)
                .display_title(&app),
            phone: false,
            busy: false,
            error: None,
            started: false,
            remaining: vec![],
        });
        self.modal = Modal::AppDelete;
        cx.notify();
    }
    pub(in crate::app) fn configure_app_delete_dialog(
        &self,
        alert: AlertDialog,
        cx: &mut Context<Self>,
    ) -> AlertDialog {
        let Some(state) = &self.home.deletion else {
            return alert;
        };
        let owner = cx.entity();
        let confirm = owner.clone();
        let cancel = owner.clone();
        alert
            .title("删除卡片")
            .description(format!(
                "确定删除“{}”的本地账号、state 和任务？手机备份保留。",
                state.title
            ))
            .keyboard(!state.busy)
            .close_button(!state.busy)
            .on_ok(move |_, _, cx| {
                confirm.update(cx, |this, cx| this.confirm_app_delete(cx));
                false
            })
            .on_cancel(move |_, _, cx| {
                cancel.update(cx, |this, cx| {
                    if this.home.deletion.as_ref().is_some_and(|state| state.busy) {
                        return false;
                    }
                    this.close_modal(cx);
                    true
                })
            })
            .on_close(move |_, _, cx| owner.update(cx, |this, cx| this.close_modal(cx)))
            .when_some(state.error.as_ref(), |alert, message| {
                alert.child(
                    crate::typography::caption(message.clone(), cx)
                        .text_color(crate::theme::danger(cx)),
                )
            })
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("cancel-app-delete")
                            .label("取消")
                            .disabled(state.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.close_modal(cx))),
                    )
                    .child(
                        Button::new("confirm-app-delete")
                            .danger()
                            .label("删除")
                            .debug_selector(|| "confirm-app-delete".into())
                            .loading(state.busy)
                            .disabled(state.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.confirm_app_delete(cx))),
                    ),
            )
    }
    pub(in crate::app) fn confirm_app_delete(&mut self, cx: &mut Context<Self>) {
        let Some(state) = &self.home.deletion else {
            return;
        };
        if state.busy
            || self.read_only
            || self.home.jobs.contains(&state.id)
            || self.home.backup_syncing.contains_key(&state.id)
        {
            return;
        }
        let id = state.id.clone();
        let phone = false;
        let links = if state.started {
            // Completed deletions have already been popped. An explicit retry
            // continues from the failed archive instead of repeating the batch.
            state.remaining.clone()
        } else {
            match WorkspaceStore::open(self.book.paths()).and_then(|store| store.backups(&id)) {
                Ok(links) => links,
                Err(error) => {
                    self.delete_failed(error.to_string(), cx);
                    return;
                }
            }
        };
        if phone {
            // Check every source before sending any mutation.
            if links
                .iter()
                .any(|link| !self.phone_online(link.device_id, cx))
            {
                self.delete_failed("请先连接备份来源手机，或取消同时删除手机备份".into(), cx);
                return;
            }
        }
        let state = self.home.deletion.as_mut().unwrap();
        state.busy = true;
        state.error = None;
        state.started = true;
        state.remaining = if phone { links } else { vec![] };
        self.advance_app_delete(cx);
    }
    fn delete_failed(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(state) = &mut self.home.deletion {
            tracing::error!(app_id = state.id.as_str(), delete_phone_backups = state.phone,
                remaining_backups = state.remaining.len(), error = %message,
                "Home App card deletion failed");
            state.busy = false;
            state.error = Some(message);
        }
        cx.notify();
    }
    fn advance_app_delete(&mut self, cx: &mut Context<Self>) {
        let Some(state) = &self.home.deletion else {
            return;
        };
        let id = state.id.clone();
        if let Some(link) = state.remaining.last().cloned() {
            let result = WorkspaceStore::open(self.book.paths()).and_then(|store| store.load(&id));
            let app = match result {
                Ok(app) => app,
                Err(error) => {
                    self.delete_failed(error.to_string(), cx);
                    return;
                }
            };
            let Some(view) = self
                .home
                .controls
                .get(&link.device_id)
                .map(|control| control.view.clone())
            else {
                self.delete_failed("手机控制未连接".into(), cx);
                return;
            };
            if let Err(error) = view.update(cx, |view, cx| {
                view.delete_home_backup(id, &app.bundle_id, &link.backup_id, cx)
            }) {
                self.delete_failed(error, cx);
            }
            return;
        }
        match WorkspaceStore::open(self.book.paths()).and_then(|mut store| store.delete_app(&id)) {
            Ok(()) => {
                self.home.deletion = None;
                self.home.editing = None;
                self.modal = Modal::None;
                self.reload_home(cx);
                self.reload_chat(cx);
                self.set_status("已删除", cx);
            }
            Err(error) => self.delete_failed(error.to_string(), cx),
        }
    }
    pub(crate) fn home_backup_deleted(
        &mut self,
        id: AppId,
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        if !self
            .home
            .deletion
            .as_ref()
            .is_some_and(|state| state.id == id && state.busy)
        {
            return;
        }
        match result {
            Ok(()) => {
                self.home.deletion.as_mut().unwrap().remaining.pop();
                // Avoid borrowing the controller again while it delivers this callback.
                let owner = cx.entity().downgrade();
                cx.defer(move |cx| {
                    let _ = owner.update(cx, |this, cx| this.advance_app_delete(cx));
                });
            }
            Err(message) => self.delete_failed(message, cx),
        }
    }
}
