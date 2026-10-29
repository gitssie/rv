//! Explicit RV → phone backup handoff. This never restores or launches the phone app.
use super::*;
use crate::{assets::FunctionIcon, theme};
use gpui_component::{
    Disableable, Icon,
    button::{Button, ButtonVariants},
    menu::{DropdownMenu, PopupMenuItem},
    tooltip::Tooltip,
};

impl AddressBookApp {
    pub(crate) fn execution_mode(&self, id: &AppId) -> ExecutionMode {
        self.home
            .card_context
            .get(id)
            .map(|context| context.execution.mode)
            .unwrap_or_default()
    }
    pub(in crate::app) fn render_execution_indicator(&self, id: &AppId, cx: &App) -> AnyElement {
        let mode = self.execution_mode(id);
        let (icon, tooltip) = match mode {
            ExecutionMode::Rv => (FunctionIcon::Monitor, "RV 接管"),
            ExecutionMode::ToPhone => (
                FunctionIcon::Recent,
                "交接未完成；RV 任务已停止，需完成手机备份回写校验",
            ),
            ExecutionMode::Phone => (
                FunctionIcon::Smartphone,
                "手机接管；备份已回写校验，RV 任务停止。恢复由 Shadow 操作",
            ),
        };
        div()
            .id(SharedString::from(format!(
                "execution-owner-{}",
                id.as_str()
            )))
            .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
            .child(Icon::new(icon).text_color(if mode == ExecutionMode::Rv {
                theme::muted(cx)
            } else {
                theme::accent(cx)
            }))
            .into_any_element()
    }
    pub(super) fn render_phone_handoff_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(app) = &self.home.editing else {
            return div().into_any_element();
        };
        let id = app.id.clone();
        let mode = self.execution_mode(&id);
        let links = self.home.links.get(&id).cloned().unwrap_or_default();
        let active = self.home.handoff_running.contains_key(&id);
        let online: std::collections::HashSet<_> = links
            .iter()
            .filter(|link| {
                self.home
                    .controls
                    .get(&link.device_id)
                    .is_some_and(|control| control.view.read(cx).can_handoff_backup())
            })
            .map(|link| link.device_id)
            .collect();
        let button = Button::new("shop-handoff-phone")
            .debug_selector(|| "shop-handoff-phone".into())
            .ghost()
            .icon(FunctionIcon::Smartphone)
            .tooltip(match mode {
                ExecutionMode::Rv => "交给手机：停止 RV 任务并回写账号备份",
                ExecutionMode::ToPhone => "重试交接：回写并校验手机备份",
                ExecutionMode::Phone => "已由手机接管",
            })
            .accessibility_label("交给手机")
            .loading(active)
            .disabled(
                self.read_only
                    || active
                    || mode == ExecutionMode::Phone
                    || online.is_empty()
                    || self.home.backup_syncing.contains_key(&id),
            );
        if let [link] = links.as_slice() {
            let link = link.clone();
            return button
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.confirm_phone_handoff(id.clone(), link.clone(), window, cx)
                }))
                .into_any_element();
        }
        let owner = cx.entity();
        let titles = self.backup_source_labels(&links);
        button
            .dropdown_menu(move |mut menu, _, _| {
                for (link, title) in links.iter().zip(&titles) {
                    let owner = owner.clone();
                    let id = id.clone();
                    let link = link.clone();
                    menu = menu.item(
                        PopupMenuItem::new(title.clone())
                            .disabled(!online.contains(&link.device_id))
                            .on_click(move |_, window, cx| {
                                owner.update(cx, |this, cx| {
                                    this.confirm_phone_handoff(id.clone(), link.clone(), window, cx)
                                });
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }
    fn confirm_phone_handoff(
        &mut self,
        id: AppId,
        link: BackupLink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only || self.home.handoff_running.contains_key(&id) {
            return;
        }
        let owner = cx.entity();
        let device = self
            .book
            .get(link.device_id)
            .map(|device| device.name.clone())
            .unwrap_or_else(|| "手机".into());
        crate::ui::confirm_execution_handoff(
            crate::ui::ExecutionHandoffPrompt {
                title: "交给手机",
                icon: FunctionIcon::Smartphone,
                device,
                backup: link.title.clone(),
                acknowledgement: None,
            },
            window,
            cx,
            move |_, cx| {
                owner.update(cx, |this, cx| {
                    this.start_phone_handoff(id.clone(), link.clone(), cx)
                });
            },
        );
    }
    fn start_phone_handoff(&mut self, id: AppId, link: BackupLink, cx: &mut Context<Self>) {
        if self.read_only
            || self.home.handoff_running.contains_key(&id)
            || self.home.backup_syncing.contains_key(&id)
        {
            return;
        }
        let Some(view) = self
            .home
            .controls
            .get(&link.device_id)
            .map(|control| control.view.clone())
        else {
            self.set_status("请先连接目标手机", cx);
            return;
        };
        if !view.read(cx).can_handoff_backup() {
            self.set_status("请先连接可写的目标手机并完成当前操作", cx);
            return;
        }
        let prepared = WorkspaceStore::open(self.book.paths()).and_then(|mut store| {
            let app = store.load(&id)?;
            let execution = store.begin_phone_handoff(&id, link.device_id)?;
            Ok((app.bundle_id, execution))
        });
        match prepared {
            Ok((bundle, execution)) => {
                let command = rv_session::AppCommand::HandoffBackup {
                    app_id: id.clone(),
                    bundle,
                    backup: link.backup_id,
                    store_root: self.book.paths().root.clone(),
                    device_id: link.device_id,
                    operation_id: execution.operation_id.clone(),
                };
                self.home
                    .card_context
                    .entry(id.clone())
                    .or_default()
                    .execution = execution;
                let owner = view.entity_id();
                if view.update(cx, |view, cx| view.handoff_backup(command, cx)) {
                    self.home.handoff_running.insert(id.clone(), owner);
                    self.set_status("交接中：RV 任务已停止，等待本轮任务结束后回写手机备份", cx);
                } else {
                    self.set_status("RV 任务已停止，请重新连接目标手机后重试交接", cx);
                }
                self.reload_shop_tasks(cx);
                self.reload_home(cx);
            }
            Err(error) => {
                tracing::error!(app_id=id.as_str(),error=%error,"Phone handoff preparation failed");
                self.set_status(format!("交接未开始：{error}"), cx);
            }
        }
    }
    pub(crate) fn rv_takeover_available(
        &self,
        bundle: &str,
        backup: &str,
        device: Option<ConnectionId>,
    ) -> bool {
        if self.read_only || rv_business::strategy(bundle).is_none() {
            return false;
        }
        let Some(id) = backup
            .strip_prefix("snapshot:")
            .and_then(|value| AppId::from_idfv(value).ok())
        else {
            return false;
        };
        let available = device.is_some_and(|device| {
            WorkspaceStore::open(self.book.paths()).is_ok_and(|store| {
                store.execution(&id).is_ok_and(|execution| {
                    store.validate_rv_takeover(&id, device, &execution.operation_id).is_ok()
                })
            })
        });
        available && !self.home.handoff_running.contains_key(&id)
            && !self.home.backup_syncing.contains_key(&id)
    }
    pub(crate) fn takeover_phone_name(&self, device: ConnectionId) -> String {
        self.book
            .get(device)
            .map(|device| device.name.clone())
            .unwrap_or_else(|| "手机".into())
    }
    pub(crate) fn prepare_rv_takeover(
        &mut self,
        id: &AppId,
        bundle: &str,
        backup: &str,
        device: ConnectionId,
        owner: EntityId,
        cx: &mut Context<Self>,
    ) -> Result<rv_session::AppCommand, String> {
        if !self.rv_takeover_available(bundle, backup, Some(device)) {
            return Err("请选择提供账号凭据的手机备份，或等待当前同步完成".into());
        }
        let command = WorkspaceStore::open(self.book.paths())
            .and_then(|store| {
                let app = store.load(id)?;
                if app.bundle_id != bundle {
                    return Err(rv_core::StoreError::Invalid("备份与账号不匹配".into()));
                }
                let execution = store.execution(id)?;
                store.validate_rv_takeover(id, device, &execution.operation_id)?;
                Ok(rv_session::AppCommand::TakeoverBackup {
                    app_id: id.clone(),
                    bundle: bundle.into(),
                    backup: backup.into(),
                    store_root: self.book.paths().root.clone(),
                    device_id: device,
                    operation_id: execution.operation_id,
                })
            })
            .map_err(|error| {
                tracing::error!(app_id=id.as_str(),error=%error,"RV takeover preparation failed");
                error.to_string()
            })?;
        self.home.handoff_running.insert(id.clone(), owner);
        self.set_status("正在读取手机备份并交给 RV…", cx);
        Ok(command)
    }
    pub(crate) fn rv_takeover_finished(
        &mut self,
        id: &AppId,
        owner: EntityId,
        success: bool,
        cx: &mut Context<Self>,
    ) {
        if self
            .home
            .handoff_running
            .get(id)
            .is_some_and(|current| *current != owner)
        {
            return;
        }
        self.home.handoff_running.remove(id);
        self.set_status(
            if success {
                "已采用手机备份凭据，Token 由 RV 刷新；已停止的任务保持停止"
            } else {
                "交给 RV 未完成，原凭据和 Token 来源保持不变"
            },
            cx,
        );
        self.refresh_editing_baseline(id);
        self.reload_home(cx);
        if self.home.editing.as_ref().is_some_and(|app| app.id == *id) {
            self.reload_shop_tasks(cx);
        }
    }
    pub(crate) fn phone_handoff_finished(
        &mut self,
        id: &AppId,
        owner: EntityId,
        success: bool,
        cx: &mut Context<Self>,
    ) {
        if self
            .home
            .handoff_running
            .get(id)
            .is_some_and(|current| *current != owner)
        {
            return;
        }
        self.home.handoff_running.remove(id);
        self.set_status(
            if success {
                "备份已回写校验，手机接管；请使用 Shadow 恢复，RV 任务保持停止"
            } else {
                "交接未完成，RV 任务保持停止，请连接原目标手机重试"
            },
            cx,
        );
        self.reload_home(cx);
        if self.home.editing.as_ref().is_some_and(|app| app.id == *id) {
            self.reload_shop_tasks(cx);
        }
    }
}
