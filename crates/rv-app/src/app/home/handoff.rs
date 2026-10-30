//! Explicit bidirectional backup handoff. This never restores or launches the phone app.
use super::*;
use crate::{assets::FunctionIcon, theme};
use gpui::prelude::FluentBuilder;
use gpui_component::{
    Disableable, Icon, WindowExt,
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
        let keepalive_enabled = self.home.card_context.get(id)
            .is_some_and(|context| context.keepalive_enabled);
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
            .child(Icon::new(icon).text_color(if mode != ExecutionMode::Rv || keepalive_enabled {
                theme::accent(cx)
            } else {
                theme::muted(cx)
            }))
            .into_any_element()
    }
    pub(super) fn render_phone_handoff_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(app) = &self.home.editing else {
            return div().into_any_element();
        };
        let id = app.id.clone();
        let mode = self.execution_mode(&id);
        let phone_sourced = WorkspaceStore::open(self.book.paths())
            .and_then(|store| store.token_source(&id))
            .is_ok_and(|source| matches!(source,rv_core::workspace::TokenSource::Phone(_)));
        let to_rv = mode != ExecutionMode::ToPhone && (phone_sourced || mode == ExecutionMode::Phone);
        let bundle = app.bundle_id.clone();
        let links = self.home.links.get(&id).cloned().unwrap_or_default();
        let active = self.home.handoff_running.contains_key(&id);
        let online: std::collections::HashSet<_> = links
            .iter()
            .filter(|link| {
                self.home
                    .controls
                    .get(&link.device_id)
                    .is_some_and(|control| control.view.read(cx).can_handoff_backup())
                    && (!to_rv || self.rv_takeover_available(&bundle,&link.backup_id,Some(link.device_id)))
            })
            .map(|link| link.device_id)
            .collect();
        let selector=if to_rv {"shop-handoff-rv"}else{"shop-handoff-phone"};
        let button = Button::new(selector)
            .debug_selector(move || selector.into())
            .ghost()
            .icon(if to_rv {FunctionIcon::Monitor}else{FunctionIcon::Smartphone})
            .tooltip(if to_rv {"交给 RV：读取关联手机最新备份并接管刷新"}
                else if mode==ExecutionMode::ToPhone {"重试交接：回写并校验手机备份"}
                else {"状态修复或交给手机"})
            .accessibility_label(if to_rv {"交给 RV"}else{"状态修复或交给手机"})
            .loading(active)
            .disabled(self.read_only || active || online.is_empty()
                || self.home.backup_syncing.contains_key(&id));
        if let [link] = links.as_slice() {
            let link = link.clone();
            return button
                .on_click(cx.listener(move |this, _, window, cx| {
                    if to_rv {this.confirm_shop_rv_takeover(id.clone(),link.clone(),window,cx);}
                    else {this.choose_phone_handoff(id.clone(), link.clone(), window, cx);}
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
                                    if to_rv {this.confirm_shop_rv_takeover(id.clone(),link.clone(),window,cx);}
                                    else {this.choose_phone_handoff(id.clone(), link.clone(), window, cx);}
                                });
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }
    fn choose_phone_handoff(&mut self,id:AppId,link:BackupLink,window:&mut Window,cx:&mut Context<Self>) {
        if self.read_only || self.home.handoff_running.contains_key(&id)
            || self.home.backup_syncing.contains_key(&id) {return;}
        if self.execution_mode(&id)==ExecutionMode::ToPhone {
            self.confirm_phone_handoff(id,link,window,cx);return;
        }
        let paths=self.book.paths().clone();
        let owner=cx.entity();
        let device=self.takeover_phone_name(link.device_id);
        window.open_dialog(cx,move|dialog,_,cx| {
            let can_repair=WorkspaceStore::open(&paths).is_ok_and(|store| {
                store.execution(&id).is_ok_and(|execution|
                    store.validate_rv_takeover(&id,link.device_id,&execution.operation_id).is_ok())
            });
            let repair_owner=owner.clone();let repair_id=id.clone();let repair_link=link.clone();
            let phone_owner=owner.clone();let phone_id=id.clone();let phone_link=link.clone();
            dialog.title("状态与交接").child(
                gpui_component::v_flex().gap_3()
                    .child(crate::typography::caption(format!("{} · {}",device,link.title),cx))
                    .when(can_repair,|content|content
                    .child(Button::new("shop-repair-state").outline().w_full()
                        .icon(FunctionIcon::Refresh).label("修复状态")
                        .debug_selector(||"shop-repair-state".into())
                        .on_click(move|_,window,cx| {
                            window.close_dialog(cx);
                            repair_owner.update(cx,|this,cx|this.confirm_shop_state_repair(repair_id.clone(),repair_link.clone(),window,cx));
                        }))
                    .child(crate::typography::caption("从手机最新备份修复 RV 状态，继续由 RV 接管。",cx)))
                    .child(Button::new("shop-transfer-to-phone").outline().w_full()
                        .icon(FunctionIcon::Smartphone).label("交给手机")
                        .debug_selector(||"shop-transfer-to-phone".into())
                        .on_click(move|_,window,cx| {
                            window.close_dialog(cx);
                            phone_owner.update(cx,|this,cx|this.confirm_phone_handoff(phone_id.clone(),phone_link.clone(),window,cx));
                        }))
                    .child(crate::typography::caption("将 RV 当前状态写回手机备份，停止 RV 调度。",cx))
            )
        });
    }
    fn confirm_shop_rv_takeover(&mut self,id:AppId,link:BackupLink,window:&mut Window,cx:&mut Context<Self>) {
        let owner=cx.entity();
        crate::ui::confirm_execution_handoff(
            crate::ui::ExecutionHandoffPrompt {
                title:"交给 RV",description:crate::ui::RV_HANDOFF_DESCRIPTION,icon:FunctionIcon::Monitor,
                device:self.takeover_phone_name(link.device_id),backup:link.title.clone(),
                acknowledgement:Some("已在 Shadow 保存最新备份，并切换到其他环境"),
            },window,cx,move|_,cx|owner.update(cx,|this,cx|this.start_shop_rv_takeover(id.clone(),link.clone(),cx)),
        );
    }
    pub(super) fn confirm_shop_state_repair(&mut self,id:AppId,link:BackupLink,window:&mut Window,cx:&mut Context<Self>) {
        if self.read_only || self.home.handoff_running.contains_key(&id)
            || self.home.backup_syncing.contains_key(&id) {return;}
        if !WorkspaceStore::open(self.book.paths()).and_then(|store|store.state_repair_needed(&id)).unwrap_or(false) {
            self.set_error_status("访问令牌仍可用，无需修复状态",cx);return;
        }
        let owner=cx.entity();
        crate::ui::confirm_execution_handoff(
            crate::ui::ExecutionHandoffPrompt {
                title:"修复状态",
                description:"请先在 Shadow 保存该环境的最新备份，并切换到其他环境。目标仍为 active 时不能交接。此操作以所选备份的 state 和完整令牌对修复 RV，继续由 RV 接管；不会自动备份或切换手机，已停止的任务保持停止。",
                icon:FunctionIcon::Refresh,
                device:self.takeover_phone_name(link.device_id),backup:link.title.clone(),
                acknowledgement:Some("已在 Shadow 保存最新备份，并切换到其他环境"),
            },window,cx,move|_,cx|owner.update(cx,|this,cx|this.start_shop_rv_takeover(id.clone(),link.clone(),cx)),
        );
    }
    fn start_shop_rv_takeover(&mut self,id:AppId,link:BackupLink,cx:&mut Context<Self>) {
        let Some(view)=self.home.controls.get(&link.device_id).map(|control|control.view.clone()) else {
            self.set_error_status("请先连接关联手机",cx);return;
        };
        if !view.read(cx).can_handoff_backup() {
            self.set_error_status("请先连接可写手机并完成当前操作",cx);return;
        }
        let bundle=match WorkspaceStore::open(self.book.paths()).and_then(|store|store.load(&id)) {
            Ok(app)=>app.bundle_id,
            Err(error)=>{self.set_error_status(error.to_string(),cx);return;},
        };
        let owner=view.entity_id();
        match self.prepare_rv_takeover(&id,&bundle,&link.backup_id,link.device_id,owner,cx) {
            Ok(command)=>{
                if !view.update(cx,|view,cx|view.handoff_backup(command,cx)) {
                    self.rv_takeover_finished(&id,owner,false,cx);
                }
            }
            Err(error)=>self.set_error_status(error,cx),
        }
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
                description: crate::ui::PHONE_HANDOFF_DESCRIPTION,
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
