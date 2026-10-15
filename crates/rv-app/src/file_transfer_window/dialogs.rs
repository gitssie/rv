//! Component dialogs; file operations remain in the transfer controller.
use super::*;
use gpui_component::{
    WindowExt,
    button::ButtonVariant,
    dialog::{Dialog, DialogButtonProps, DialogFooter},
};

pub(super) struct TransferShell(pub Entity<FileTransferView>);
impl Render for TransferShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .relative()
            .child(self.0.clone())
            .children(gpui_component::Root::render_dialog_layer(window, cx))
    }
}
impl FileTransferView {
    pub(super) fn dismiss_component_dialog(&mut self, kind: u8, cx: &mut Context<Self>) {
        match kind {
            1 => self.details = None,
            2 => self.pending_delete = None,
            3 => self.new_folder_side = None,
            4 => self.pending_rename = None,
            5 => self.pending_conflicts = None,
            6 => self.pending_batch_delete = None,
            _ => {}
        }
        cx.notify();
    }
    pub(super) fn sync_component_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let kind = if self.pending_batch_delete.is_some() {
            Some(6)
        } else if self.pending_conflicts.is_some() {
            Some(5)
        } else if self.pending_rename.is_some() {
            Some(4)
        } else if self.new_folder_side.is_some() {
            Some(3)
        } else if self.pending_delete.is_some() {
            Some(2)
        } else if self.details.is_some() {
            Some(1)
        } else {
            None
        };
        if kind == self.component_dialog {
            return;
        }
        if self.component_dialog.take().is_some() && window.has_active_dialog(cx) {
            window.close_dialog(cx);
            self.focus.focus(window, cx);
        }
        self.component_dialog = kind;
        let Some(kind) = kind else {
            return;
        };
        let owner = cx.entity();
        if kind == 2 || kind == 6 {
            window.open_alert_dialog(cx, move |alert, _, cx| {
                owner.update(cx, |this, cx| {
                    let title = if kind == 2 {
                        format!("Delete {}?", this.pending_delete.as_ref().unwrap().name)
                    } else {
                        let count = if this.pending_batch_delete == Some(FileSide::Local) {
                            this.local_selected_paths.len()
                        } else {
                            this.remote_selected_names.len()
                        };
                        format!("Delete {count} items?")
                    };
                    let side = if kind == 2 {
                        this.pending_delete.as_ref().unwrap().side
                    } else {
                        this.pending_batch_delete.unwrap()
                    };
                    let location = if side == FileSide::Local {
                        "this Mac"
                    } else {
                        "TrollVNC"
                    };
                    let confirm = cx.entity();
                    let cancel = confirm.clone();
                    let close = confirm.clone();
                    alert
                        .title(title)
                        .description(format!(
                            "Permanently remove these items from {location}. Folders must be empty."
                        ))
                        .confirm()
                        .button_props(
                            DialogButtonProps::default()
                                .show_cancel(true)
                                .ok_text("Delete permanently")
                                .cancel_text("Cancel")
                                .ok_variant(ButtonVariant::Danger),
                        )
                        .on_ok(move |_, _, cx| {
                            confirm.update(cx, |this, cx| {
                                if kind == 2 {
                                    this.confirm_delete(cx)
                                } else {
                                    this.confirm_batch_delete(cx)
                                }
                            });
                            false
                        })
                        .on_cancel(move |_, _, cx| {
                            cancel.update(cx, |this, cx| this.dismiss_component_dialog(kind, cx));
                            true
                        })
                        .on_close(move |_, _, cx| {
                            close.update(cx, |this, cx| this.dismiss_component_dialog(kind, cx))
                        })
                })
            });
        } else {
            window.open_dialog(cx, move |dialog, _, cx| {
                owner.update(cx, |this, cx| {
                    this.configure_transfer_dialog(dialog, kind, cx)
                })
            });
            if kind == 3 || kind == 4 {
                self.new_folder_input
                    .update(cx, |input, cx| input.focus(window, cx));
            }
        }
    }
    fn configure_transfer_dialog(
        &self,
        dialog: Dialog,
        kind: u8,
        cx: &mut Context<Self>,
    ) -> Dialog {
        let owner = cx.entity();
        let confirm = owner.clone();
        let close = owner.clone();
        let dialog = dialog
            .max_h(relative(0.9))
            .on_ok(move |_, _, cx| {
                confirm.update(cx, |this, cx| match kind {
                    3 => this.create_folder(cx),
                    4 => this.rename_item(cx),
                    1 => this.dismiss_component_dialog(kind, cx),
                    _ => {}
                });
                false
            })
            .on_cancel(move |_, _, cx| {
                owner.update(cx, |this, cx| this.dismiss_component_dialog(kind, cx));
                true
            })
            .on_close(move |_, _, cx| {
                close.update(cx, |this, cx| this.dismiss_component_dialog(kind, cx))
            });
        let cancel = Button::new("transfer-dialog-cancel")
            .label(if kind == 1 { "Close" } else { "Cancel" })
            .on_click(cx.listener(move |this, _, _, cx| this.dismiss_component_dialog(kind, cx)));
        match kind {
            1 => {
                let Some(details) = &self.details else {
                    return dialog;
                };
                dialog
                    .title(format!("{} · Info", details.name))
                    .child(
                        v_flex()
                            .gap_3()
                            .child(info_row(
                                cx,
                                "Location",
                                if details.side == FileSide::Local {
                                    "This Mac"
                                } else {
                                    "TrollVNC"
                                },
                            ))
                            .child(info_row(
                                cx,
                                "Type",
                                if details.is_dir { "Folder" } else { "File" },
                            ))
                            .child(info_row(cx, "Path", details.path.clone()))
                            .child(info_row(
                                cx,
                                "Size",
                                details.size.map_or_else(
                                    || "—".into(),
                                    |size| format!("{} ({} bytes)", format_size(size), size),
                                ),
                            ))
                            .child(info_row(
                                cx,
                                "Modified",
                                details.modified.map_or_else(
                                    || "—".into(),
                                    |time| {
                                        DateTime::<Local>::from(time)
                                            .format("%Y-%m-%d %H:%M:%S")
                                            .to_string()
                                    },
                                ),
                            )),
                    )
                    .footer(DialogFooter::new().child(cancel))
            }
            3 | 4 => {
                let title = if kind == 3 {
                    "New folder".to_owned()
                } else {
                    format!("Rename {}", self.pending_rename.as_ref().unwrap().name)
                };
                let ok = Button::new("transfer-dialog-submit")
                    .primary()
                    .label(if kind == 3 { "Create" } else { "Rename" })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if kind == 3 {
                            this.create_folder(cx)
                        } else {
                            this.rename_item(cx)
                        }
                    }));
                dialog
                    .title(title)
                    .child(Input::new(&self.new_folder_input).aria_label("Name"))
                    .footer(DialogFooter::new().child(cancel).child(ok))
            }
            5 => {
                let Some(jobs) = &self.pending_conflicts else {
                    return dialog;
                };
                let can_replace = self.can_replace_conflicts(jobs);
                dialog.title("Files with the same name")
                    .child(format!("Some of the {} selected files already exist at the destination. Choose how to handle those files.", jobs.len()))
                    .footer(DialogFooter::new().child(cancel).children([
                        ("conflict-skip", "Skip", CollisionPolicy::Skip),
                        ("conflict-rename", "Rename", CollisionPolicy::Rename),
                        ("conflict-replace", "Replace", CollisionPolicy::Replace),
                    ].map(|(id, label, policy)| Button::new(id).label(label)
                        .disabled(matches!(policy, CollisionPolicy::Replace) && !can_replace)
                        .on_click(cx.listener(move |this, _, _, cx| this.resolve_conflicts(policy, cx))))))
            }
            _ => dialog,
        }
    }
}
