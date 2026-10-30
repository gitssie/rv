use super::super::apps_ui::app_icon;
use super::*;
use gpui_component::menu::{DropdownMenu, PopupMenuItem};
use rv_session::AppCommand;

// Same system exclusions as native AppState; native entitlement checks still apply.
fn backup_candidate(id: &str) -> bool {
    rv_session::valid_bundle_id(id)
        && !id.starts_with("com.apple.")
        && id != "com.82flex.TrollVNCApp"
}

#[derive(Clone)]
pub(super) struct Backup {
    pub id: String,
    app: String,
    name: String,
    note: String,
    date: String,
    snapshot: bool,
}
#[derive(Clone, Copy, PartialEq)]
enum EnvironmentRunStatus {
    Running,
    Succeeded,
    Failed,
    Unknown,
}
#[derive(Clone)]
pub(super) struct EnvironmentRun {
    action: &'static str,
    title: &'static str,
    status: EnvironmentRunStatus,
    operation: Option<String>,
    progress: serde_json::Value,
    result: Option<serde_json::Value>,
    message: String,
}
#[derive(Clone)]
pub(super) struct EnvironmentUi {
    pub open: bool,
    run: Option<EnvironmentRun>,
    apps: Vec<rv_core::AppShortcut>,
    catalog_open: bool,
    records: Vec<Backup>,
    baselines: Vec<Backup>,
    current: Option<String>,
    loaded: bool,
    running: Option<bool>,
    status_failed: bool,
    restore_pending: bool,
    pub backup: Option<Backup>,
}
impl EnvironmentUi {
    pub(super) fn mark_disconnected(&mut self) {
        if let Some(run) = &mut self.run {
            if run.status == EnvironmentRunStatus::Running {
                run.status = EnvironmentRunStatus::Unknown;
                run.message = "连接已中断，结果未确认".into();
            }
        }
    }
    pub(super) fn mark_status_failed(&mut self) {
        self.running = None;
        self.status_failed = true;
    }
    pub fn new(apps: Vec<rv_core::AppShortcut>) -> Self {
        Self {
            open: false,
            run: None,
            apps: apps
                .into_iter()
                .filter(|app| backup_candidate(&app.bundle_id))
                .collect(),
            catalog_open: false,
            records: vec![],
            baselines: vec![],
            current: None,
            loaded: false,
            running: None,
            status_failed: false,
            restore_pending: false,
            backup: None,
        }
    }
    pub fn clear_records(&mut self) {
        self.records.clear();
        self.baselines.clear();
        self.current = None;
        self.loaded = false;
        self.running = None;
        self.status_failed = false;
        self.restore_pending = false;
        self.backup = None;
    }
}
impl SessionView {
    pub(crate) fn can_handoff_backup(&self) -> bool {
        self.phase == Phase::Connected && !self.view_only() && !self.functions.app_state_unsettled()
    }
    pub(crate) fn handoff_backup(&mut self, command: AppCommand, cx: &mut Context<Self>) -> bool {
        if !self.can_handoff_backup() {
            return false;
        }
        let to_rv=matches!(&command,AppCommand::TakeoverBackup{..});
        self.handle.app(command);
        self.functions.message = if to_rv {"正在读取手机备份并交给 RV…"}
            else {"正在回写账号备份，RV 任务已停止…"}.into();
        cx.notify();
        true
    }
    pub(crate) fn read_home_profile(
        &mut self,
        id: rv_core::workspace::AppId,
        bundle: String,
        backup: String,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.phase != Phase::Connected
            || self.view_only()
            || self.functions.app_state_unsettled()
        {
            self.functions.message = "请先连接手机并完成当前备份操作".into();
            cx.notify();
            return false;
        }
        self.handle.app(AppCommand::ReadProfile {
            app_id: id,
            bundle,
            backup,
        });
        self.functions.message = "正在从手机读取账号备份…".into();
        cx.notify();
        true
    }
    pub(in crate::session_window) fn phone_handoff_result(
        &mut self,
        saved: bool,
        cx: &mut Context<Self>,
    ) {
        self.functions.message = if saved {
            "手机备份已回写校验，RV 任务保持停止"
        } else {
            "交接未完成，RV 任务保持停止，请查看日志并重试"
        }
        .into();
        cx.notify();
    }
    pub(in crate::session_window) fn backup_profile_result(
        &mut self,
        saved: bool,
        cx: &mut Context<Self>,
    ) {
        self.functions.message = if saved {
            "账号备份已保存到 RV"
        } else {
            "账号备份保存失败，请查看日志"
        }
        .into();
        cx.notify();
    }
    pub(crate) fn home_connected(&self) -> bool {
        self.phase == Phase::Connected
    }
    pub(crate) fn notify_home(&self, cx: &mut Context<Self>) {
        if let Some(id) = self.req.connection_id
            && let Some(book) = self.address_book.as_ref().and_then(WeakEntity::upgrade)
        {
            let foreground = if self.phase == Phase::Connected {
                self.apps.home_foreground()
            } else {
                None
            };
            let cx_entity = cx.entity().entity_id();
            book.update(cx, |book, cx| {
                if self.phase != Phase::Connected {
                    book.backup_sync_disconnected(cx_entity, cx);
                }
                if self.control_only {
                    book.phone_changed(id, foreground, cx);
                }
            });
        }
    }
    pub(in crate::session_window) fn close_function_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.functions.open = false;
        if self.functions.environment.open
            && !self.functions.busy
            && self.functions.operation.is_none()
        {
            self.clear_function_text(window, cx);
        }
        if self.control_only {
            if let Some(id) = self.req.connection_id
                && let Some(book) = self.address_book.as_ref().and_then(WeakEntity::upgrade)
            {
                book.update(cx, |book, cx| book.dismiss_phone_control(id, cx));
            }
        } else {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }
    pub(crate) fn delete_home_backup(
        &mut self,
        app: rv_core::workspace::AppId,
        bundle: &str,
        backup: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if !self.home_connected() || self.view_only() {
            return Err("手机控制未连接或仅查看".into());
        }
        if self.pending_home_delete.is_some()
            || self.functions.app_state_unsettled()
            || self.functions.busy
        {
            return Err("请先完成手机上的当前操作".into());
        }
        if !backup.starts_with("snapshot:")
            || bundle.contains(['\n', '\r'])
            || backup.contains(['\n', '\r'])
        {
            return Err("备份标识无效".into());
        }
        self.pending_home_delete = Some(app);
        self.handle.function(FunctionCommand::Invoke {
            id: "app-delete".into(),
            text: format!("App={bundle}\n备份={backup}"),
        });
        cx.notify();
        Ok(())
    }
    pub(in crate::session_window) fn receive_home_delete(
        &mut self,
        event: &FunctionEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.pending_home_delete.is_none() {
            return false;
        }
        let event = match event {
            FunctionEvent::ForOperation { id, event } if id == "app-delete" => event.as_ref(),
            FunctionEvent::Error(_) | FunctionEvent::Uncertain(_) => event,
            _ => return false,
        };
        let result = match event {
            FunctionEvent::State { task: 32, record } if record["status"] == "completed" => {
                Some(if rv_session::functions::uncertain_result(32, record) {
                    Err("手机删除结果未知，请先核对手机备份；本地卡片已保留".into())
                } else if record.get("error").is_some() {
                    Err(super::functions_ui::function_error_message(record).to_string())
                } else {
                    Ok(())
                })
            }
            FunctionEvent::Error(message) | FunctionEvent::Uncertain(message) => {
                Some(Err(message.clone()))
            }
            _ => None,
        };
        if let Some(result) = result {
            let app = self.pending_home_delete.take().unwrap();
            if let Some(book) = self.address_book.as_ref().and_then(WeakEntity::upgrade) {
                book.update(cx, |book, cx| book.home_backup_deleted(app, result, cx));
            }
        }
        true
    }
    pub(crate) fn show_home_environment(
        &mut self,
        bundle: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.functions.app_state_unsettled() {
            self.functions.message = "备份操作正在进行".into();
            cx.notify();
            return;
        }
        self.open_app_environment(None, window, cx);
        self.requested_environment = bundle;
        self.tick_home_environment(cx);
    }
    pub(in crate::session_window) fn tick_home_environment(&mut self, cx: &mut Context<Self>) {
        if !self.functions.environment.open {
            self.requested_environment = None;
            return;
        }
        if self.functions.app_state_unsettled() {
            return;
        }
        if self.phase != Phase::Connected
            || !self.functions.available
            || self.apps.loading
            || self.apps.installed.is_empty()
        {
            return;
        }
        let Some(bundle) = self.requested_environment.take() else {
            return;
        };
        if let Some(app) = self
            .apps
            .installed
            .iter()
            .find(|a| a.bundle_id == bundle)
            .cloned()
        {
            self.functions.target_app = Some((app.bundle_id, app.name));
            self.functions.environment.clear_records();
            self.refresh_environment(cx);
        } else {
            self.functions.message = "此 App 未安装在选中设备上".into();
            cx.notify();
        }
    }
    pub(in crate::session_window) fn environment_icon_ids(&self) -> Vec<String> {
        let mut ids: Vec<_> = self
            .functions
            .environment
            .apps
            .iter()
            .filter(|app| backup_candidate(&app.bundle_id))
            .map(|app| app.bundle_id.clone())
            .collect();
        if self.functions.environment.catalog_open {
            ids.extend(
                self.apps
                    .installed
                    .iter()
                    .filter(|app| backup_candidate(&app.bundle_id))
                    .map(|app| app.bundle_id.clone()),
            );
        }
        ids
    }
    fn toggle_environment_app(&mut self, id: &str, name: &str, cx: &mut Context<Self>) {
        if self.environment_locked()
            || !backup_candidate(id)
            || !self.apps.installed.iter().any(|app| app.bundle_id == id)
        {
            return;
        }
        let apps = &mut self.functions.environment.apps;
        if apps.iter().any(|app| app.bundle_id == id) {
            apps.retain(|app| app.bundle_id != id);
        } else {
            if apps.len() >= 32 {
                self.functions.message = "最多添加 32 个环境 App".into();
                cx.notify();
                return;
            }
            apps.push(rv_core::AppShortcut {
                bundle_id: id.into(),
                name: name.into(),
            });
        }
        if let Some(id) = self.req.connection_id
            && let Some(book) = self.address_book.as_ref().and_then(WeakEntity::upgrade)
            && let Err(error) = book.update(cx, |book, cx| {
                book.remember_environment_apps(id, self.functions.environment.apps.clone(), cx)
            })
        {
            self.functions.message = error.into();
        }
        self.request_app_icons(cx);
        cx.notify();
    }
    pub(in crate::session_window) fn return_function_menu(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.functions.stash_panel();
        let fresh = FunctionUi::new(Vec::new(), window, cx).snapshot();
        self.functions.restore(fresh);
        self.functions.open = true;
        self.functions.environment.open = false;
        self.functions.environment.catalog_open = false;
        self.functions.selected = None;
        self.functions.target_app = None;
        self.functions.environment.clear_records();
        self.functions.category = "Apple";
        self.assistant_open = false;
        self.apps.picker_open = false;
        self.notifications.open = false;
        self.functions.message = "".into();
        self.functions.result = None;
        self.clear_function_text(window, cx);
        self.focus.focus(window, cx);
        cx.notify();
    }
    pub(in crate::session_window) fn function_editor_visible(&self) -> bool {
        !self.functions.editor_hidden
            && self.functions.selected.is_some_and(|spec| {
                !(self.functions.environment.open && matches!(spec.id, "app-list" | "app-status"))
            })
    }
    pub(in crate::session_window) fn app_environment_busy(&self) -> bool {
        self.functions.selected.is_some_and(|spec| spec.task == 32) && self.environment_locked()
    }
    pub(super) fn environment_locked(&self) -> bool {
        self.functions.selected.is_some_and(|spec| spec.task == 32) && self.functions.busy
    }
    pub(in crate::session_window) fn open_app_environment(
        &mut self,
        id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.functions.stash_panel();
        let fresh = FunctionUi::new(Vec::new(), window, cx).snapshot();
        self.functions.restore(fresh);
        self.functions.open = true;
        self.functions.environment.open = true;
        self.functions.category = "App 环境";
        self.functions.environment.catalog_open = false;
        self.functions.selected = None;
        self.functions.target_app = None;
        self.functions.environment.clear_records();
        self.functions.message = "".into();
        self.functions.result = None;
        self.assistant_open = false;
        self.apps.picker_open = false;
        self.notifications.open = false;
        self.clear_function_text(window, cx);
        self.focus.focus(window, cx);
        if let Some(id) = id {
            if !backup_candidate(id) {
                self.functions.message = "此 App 不支持环境备份".into();
                cx.notify();
                return;
            }
            if let Some(app) = self
                .apps
                .installed
                .iter()
                .find(|app| app.bundle_id == id)
                .cloned()
            {
                self.choose_environment_app(&app.bundle_id, &app.name, window, cx);
            } else {
                self.functions.message = "请等待手机 App 列表加载，或刷新后选择".into();
                self.refresh_apps(cx);
            }
        }
        cx.notify();
    }
    fn choose_environment_app(
        &mut self,
        id: &str,
        name: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.environment_locked()
            || !backup_candidate(id)
            || !self.apps.installed.iter().any(|app| app.bundle_id == id)
        {
            return;
        }
        self.requested_environment = None;
        self.functions.target_app = Some((id.into(), name.into()));
        self.functions.environment.run = None;
        self.functions.environment.clear_records();
        self.refresh_environment(cx);
    }
    fn refresh_environment(&mut self, cx: &mut Context<Self>) {
        if self.environment_locked()
            || !self.functions.available
            || self.phase != Phase::Connected
            || self.view_only()
        {
            return;
        }
        let Some((id, _)) = &self.functions.target_app else {
            return;
        };
        if !backup_candidate(id) {
            return;
        }
        self.send_function_command(FunctionCommand::Invoke {
            id: "app-list".into(),
            text: format!("App={id}"),
        });
        self.functions.selected = OPERATIONS.iter().find(|spec| spec.id == "app-list");
        self.functions.busy = true;
        self.functions.message = "正在读取备份…".into();
        cx.notify();
    }
    pub(super) fn finish_environment_operation(
        &mut self,
        record: &serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        let Some(spec) = self.functions.selected else {
            return;
        };
        if spec.id == "app-status" {
            let result = &record["result"];
            let matches_app = self
                .functions
                .target_app
                .as_ref()
                .is_some_and(|(id, _)| result["bundle_id"].as_str() == Some(id));
            let running = result["running"].as_bool().filter(|_| {
                matches_app && !self.functions.uncertain && record.get("error").is_none()
            });
            self.functions.environment.running = running;
            self.functions.environment.status_failed = running.is_none();
            self.functions.selected = None;
            if running.is_some() {
                self.functions.message = "".into();
            } else if record.get("error").is_none() && !self.functions.uncertain {
                self.functions.message = "运行状态返回无效".into();
            }
            return;
        }
        if self.functions.uncertain || record.get("error").is_some() {
            if matches!(
                spec.id,
                "app-save" | "app-switch" | "app-edit" | "app-delete" | "app-baseline"
            ) {
                self.functions.selected = None;
                self.functions.environment.backup = None;
                self.refresh_environment(cx);
            }
            return;
        }
        if spec.id == "app-list" {
            let result = &record["result"];
            let Some((id, _)) = &self.functions.target_app else {
                return;
            };
            if result["bundle_id"].as_str() != Some(id) || !result["backups"].is_array() {
                self.functions.message = "备份列表回复与所选 App 不匹配，请刷新".into();
                self.functions.selected = None;
                return;
            }
            let app_id = id.clone();
            let archives: Vec<Backup> = result["backups"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|row| row["kind"] == "snapshot" || row["kind"] == "baseline")
                .filter_map(|row| {
                    let id = row["backup_id"].as_str()?.to_string();
                    Some(Backup {
                        app: app_id.clone(),
                        snapshot: row["kind"] == "snapshot",
                        name: row["display_name"]
                            .as_str()
                            .filter(|name| !name.is_empty())
                            .unwrap_or(&id)
                            .to_string(),
                        note: row["note"].as_str().unwrap_or("").into(),
                        date: row["date"].as_str().unwrap_or("").into(),
                        id,
                    })
                })
                .collect();
            let (snapshots, baselines) = archives.into_iter().partition(|backup| backup.snapshot);
            self.functions.environment.records = snapshots;
            self.functions.environment.baselines = baselines;
            self.functions.environment.current =
                result["current_backup_id"].as_str().map(str::to_owned);
            self.functions.environment.restore_pending =
                result["restore_pending"].as_bool().unwrap_or(false);
            if let Some(device) = self.req.connection_id
                && let Some(book) = self.address_book.as_ref().and_then(WeakEntity::upgrade)
            {
                let profiles = book.update(cx, |book, cx| {
                    book.sync_phone_backups(device, &app_id, result, cx);
                    book.missing_backup_profiles(&app_id, result)
                });
                for command in profiles {
                    self.handle.app(command);
                }
            }
            self.functions.environment.loaded = true;
            self.functions.selected = None;
            self.functions.message = "".into();
        } else if matches!(
            spec.id,
            "app-save" | "app-switch" | "app-edit" | "app-delete" | "app-baseline"
        ) {
            self.functions.selected = None;
            self.functions.environment.backup = None;
            self.refresh_environment(cx);
        }
    }
    pub(super) fn update_environment_run(&mut self, record: &serde_json::Value) {
        let Some(run) = &mut self.functions.environment.run else {
            return;
        };
        if run.operation.is_none()
            && !self
                .functions
                .selected
                .is_some_and(|spec| spec.id == run.action)
        {
            return;
        }
        let Some(id) = record["operation_id"].as_str() else {
            return;
        };
        if run
            .operation
            .as_deref()
            .is_some_and(|current| current != id)
        {
            return;
        }
        if run.status == EnvironmentRunStatus::Succeeded {
            return;
        }
        run.operation = Some(id.into());
        run.progress = record["progress"].clone();
        if record["status"] == "completed" {
            run.result = Some(record.clone());
            run.status = if rv_session::functions::uncertain_result(32, record) {
                EnvironmentRunStatus::Unknown
            } else if record.get("error").is_some() {
                EnvironmentRunStatus::Failed
            } else {
                EnvironmentRunStatus::Succeeded
            };
            run.message = match run.status {
                EnvironmentRunStatus::Succeeded => if run.action == "app-delete" {
                    "删除完成"
                } else {
                    "切换完成"
                }
                .into(),
                EnvironmentRunStatus::Unknown => "执行结果未确认".into(),
                _ => super::function_error_message(record).into(),
            };
        }
    }
    pub(super) fn environment_run_error(&mut self, message: &str, uncertain: bool) {
        if self
            .functions
            .selected
            .is_some_and(|spec| matches!(spec.id, "app-switch" | "app-delete"))
        {
            if let Some(run) = &mut self.functions.environment.run {
                if run.status == EnvironmentRunStatus::Succeeded {
                    return;
                }
                // A read/transport error cannot decide the original mutation's outcome.
                run.status = if uncertain || run.operation.is_some() {
                    EnvironmentRunStatus::Unknown
                } else {
                    EnvironmentRunStatus::Failed
                };
                run.message = message.into();
            }
        }
    }
    fn configure_environment_run_dialog(
        dialog: gpui_component::dialog::Dialog,
        run: EnvironmentRun,
        owner: WeakEntity<Self>,
        window: &mut Window,
        cx: &App,
    ) -> gpui_component::dialog::Dialog {
        let running = run.status == EnvironmentRunStatus::Running;
        let success = run.status == EnvironmentRunStatus::Succeeded;
        let stage = environment_progress_label(&run.progress);
        let files = run.progress["files"].as_u64();
        let bytes = run.progress["bytes"].as_u64();
        let can_query = owner.upgrade().is_some_and(|owner| {
            let view = owner.read(cx);
            view.phase == Phase::Connected && view.functions.available && !view.functions.busy
        });
        let mut footer = gpui_component::dialog::DialogFooter::new();
        if !running && !success {
            if let Some(record) = run.result.clone() {
                footer = footer.child(
                    crate::ui::icon_button(
                        "environment-error-copy",
                        IconName::Copy,
                        "复制错误详情",
                    )
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(record.to_string()))
                    }),
                );
            }
        }
        if run.status == EnvironmentRunStatus::Unknown && run.operation.is_some() {
            let operation = run.operation.clone().unwrap();
            footer = footer.child(
                Button::new("environment-operation-query")
                    .label("查询结果")
                    .disabled(!can_query)
                    .on_click(move |_, window, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            if let Some(run) = &mut this.functions.environment.run {
                                run.status = EnvironmentRunStatus::Running;
                                run.message = "正在查询".into();
                            }
                            if let Some(run) = &this.functions.environment.run {
                                this.functions.selected =
                                    OPERATIONS.iter().find(|spec| spec.id == run.action);
                            }
                            this.functions.busy = true;
                            this.send_function_command(FunctionCommand::Resume {
                                task: 32,
                                operation_id: operation.clone(),
                            });
                            cx.notify();
                        });
                        window.refresh();
                    }),
            );
        }
        footer = footer.child(
            Button::new("environment-operation-close")
                .label("关闭")
                .disabled(running)
                .on_click(|_, window, cx| window.close_dialog(cx)),
        );
        crate::ui::compact_dialog(dialog, window, 190.)
            .close_button(false)
            .keyboard(!running)
            .overlay_closable(false)
            .on_ok(move |_, _, _| !running)
            .on_cancel(move |_, _, _| !running)
            .title(run.title)
            .child(
                crate::ui::tool_section()
                    .child(if running {
                        crate::ui::loading(
                            "environment-operation-loading",
                            run.title,
                            if !stage.is_empty() {
                                stage.into()
                            } else {
                                run.message.clone().into()
                            },
                            cx,
                        )
                        .into_any_element()
                    } else {
                        use gpui_component::alert::Alert;
                        let alert = if success {
                            Alert::success("environment-operation-status", run.message.clone())
                        } else if run.status == EnvironmentRunStatus::Unknown {
                            Alert::warning("environment-operation-status", run.message.clone())
                        } else {
                            Alert::error("environment-operation-status", run.message.clone())
                        };
                        alert.into_any_element()
                    })
                    .when(running && (files.is_some() || bytes.is_some()), |body| {
                        body.child(div().text_xs().text_color(theme::muted(cx)).child(format!(
                            "{} 个文件 · {:.1} MB",
                            files.unwrap_or(0),
                            bytes.unwrap_or(0) as f64 / 1_048_576.
                        )))
                    })
                    .when(!running && !success && !run.progress.is_null(), |body| {
                        body.child(
                            div()
                                .text_xs()
                                .text_color(theme::muted(cx))
                                .child(environment_progress_label(&run.progress)),
                        )
                    }),
            )
            .footer(footer)
    }
    fn confirm_environment_operation(
        &mut self,
        action: &'static str,
        backup: Backup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.phase != Phase::Connected
            || self.view_only()
            || !self.functions.available
            || self.functions.app_state_unsettled()
            || window.has_active_dialog(cx)
        {
            return;
        }
        let switching = action == "app-switch";
        let can_save = !self.functions.environment.restore_pending;
        let owner = cx.entity().downgrade();
        let save_current = Rc::new(Cell::new(switching && can_save));
        let submitted = Rc::new(Cell::new(false));
        let error = Rc::new(RefCell::new(None::<String>));
        let current = self.functions.environment.current.clone();
        window.open_dialog(cx, move |dialog, window, cx| {
            let checked = save_current.clone();
            let confirm_save = save_current.clone();
            let submitted = submitted.clone();
            let owner = owner.clone();
            let backup = backup.clone();
            let current = current.clone();
            let failure = error.clone();
            let checkbox_error = error.clone();
            if submitted.get() {
                if let Some(run) = owner
                    .upgrade()
                    .and_then(|owner| owner.read(cx).functions.environment.run.clone())
                {
                    return Self::configure_environment_run_dialog(dialog, run, owner, window, cx);
                }
            }
            let title = match (switching, backup.snapshot) {
                (true, true) => "切换环境",
                (true, false) => "启用基线",
                (false, true) => "删除备份",
                (false, false) => "删除基线",
            };
            let name = backup.name.clone();
            let on_confirm: Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) -> bool> =
                Rc::new(move |_, window, cx| {
                    if submitted.get() {
                        return false;
                    }
                    let result = owner.update(cx, |this, cx| {
                        if this.phase != Phase::Connected
                            || this.view_only()
                            || !this.functions.available
                            || this.functions.app_state_unsettled()
                            || this
                                .functions
                                .target_app
                                .as_ref()
                                .is_none_or(|(id, _)| id != &backup.app)
                            || (switching && this.functions.environment.current != current)
                            || !this
                                .apps
                                .installed
                                .iter()
                                .any(|app| app.bundle_id == backup.app)
                            || !this
                                .functions
                                .environment
                                .records
                                .iter()
                                .chain(this.functions.environment.baselines.iter())
                                .any(|row| row.id == backup.id && row.app == backup.app)
                        {
                            return Err("环境状态已变化，请重新选择".to_owned());
                        }
                        let save = switching && confirm_save.get();
                        if save && this.functions.environment.restore_pending {
                            return Err("当前环境恢复未完成，请先完成恢复或取消备份".to_owned());
                        }
                        let spec = OPERATIONS.iter().find(|spec| spec.id == action).unwrap();
                        let mut text = format!("App={}\n备份={}", backup.app, backup.id);
                        if switching {
                            text.push_str(if save {
                                "\n放弃当前环境=否"
                            } else {
                                "\n放弃当前环境=是"
                            });
                        }
                        if save && current.is_none() {
                            text.push_str(&format!(
                                "\n名称=自动备份 {}",
                                chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
                            ));
                        }
                        parse_operation(spec, &text)?;
                        this.functions.selected = Some(spec);
                        this.functions.uncertain = false;
                        this.functions.panels.remove(action);
                        this.functions.environment.run = Some(EnvironmentRun {
                            action,
                            title,
                            status: EnvironmentRunStatus::Running,
                            operation: None,
                            progress: serde_json::Value::Null,
                            result: None,
                            message: "正在准备".into(),
                        });
                        this.functions.environment.backup = Some(backup.clone());
                        this.functions.operation = None;
                        this.functions.result = None;
                        this.functions.result_open = false;
                        this.functions.editor_hidden = true;
                        this.functions.message = "".into();
                        this.functions.busy = true;
                        this.send_function_command(FunctionCommand::Invoke {
                            id: spec.id.into(),
                            text,
                        });
                        cx.notify();
                        Ok(())
                    });
                    match result {
                        Ok(Ok(())) => {
                            submitted.set(true);
                            window.refresh();
                            false
                        }
                        Ok(Err(message)) => {
                            *failure.borrow_mut() = Some(message);
                            window.refresh();
                            false
                        }
                        Err(_) => true,
                    }
                });
            let confirm_click = on_confirm.clone();
            // Keep the component's modal/keyboard behavior while configuring compact placement.
            let content_height = if error.borrow().is_some() { 196. } else { 156. };
            crate::ui::compact_dialog(dialog, window, content_height)
                .close_button(false)
                .overlay_closable(false)
                .child(
                    gpui_component::dialog::DialogHeader::new()
                        .child(gpui_component::dialog::DialogTitle::new().child(title))
                        .child(gpui_component::dialog::DialogDescription::new().child(name)),
                )
                .footer(
                    gpui_component::dialog::DialogFooter::new()
                        .child(
                            Button::new("switch-cancel")
                                .label("取消")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("switch-confirm")
                                .when(switching, |button| button.primary())
                                .when(!switching, |button| button.danger())
                                .label(if switching { "切换" } else { "删除" })
                                .on_click(move |event, window, cx| {
                                    if confirm_click(event, window, cx) {
                                        window.close_dialog(cx);
                                    }
                                }),
                        ),
                )
                .when(switching, |dialog| {
                    dialog.child(
                        Checkbox::new("switch-backup-current")
                            .debug_selector(|| "switch-backup-current".into())
                            .label("备份当前环境")
                            .checked(save_current.get())
                            .disabled(!can_save)
                            .on_click(move |value: &bool, window, _| {
                                checked.set(*value);
                                *checkbox_error.borrow_mut() = None;
                                window.refresh();
                            }),
                    )
                })
                .children(error.borrow().as_ref().map(|message| {
                    div()
                        .text_sm()
                        .text_color(theme::danger(cx))
                        .child(message.clone())
                }))
                .on_ok(move |event, window, cx| on_confirm(event, window, cx))
        });
    }
    fn environment_action(
        &mut self,
        action: &str,
        backup: Option<Backup>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.environment_locked()
            || self.functions.target_app.is_none()
            || (action == "app-save" && self.functions.environment.restore_pending)
        {
            return;
        }
        if let Some(backup) = &backup
            && (self
                .functions
                .target_app
                .as_ref()
                .is_none_or(|(id, _)| id != &backup.app)
                || !self
                    .functions
                    .environment
                    .records
                    .iter()
                    .chain(self.functions.environment.baselines.iter())
                    .any(|row| row.id == backup.id && row.app == backup.app))
        {
            return;
        }
        let Some(spec) = OPERATIONS.iter().find(|spec| spec.id == action) else {
            return;
        };
        if matches!(action, "app-switch" | "app-delete") {
            if let Some(backup) = backup {
                self.confirm_environment_operation(spec.id, backup, window, cx);
            }
            return;
        }
        self.functions.environment.backup = backup.clone();
        if action == "app-status" {
            self.functions.environment.running = None;
            self.functions.environment.status_failed = false;
            // A status refresh is a new read, including after a failed request.
            self.functions.panels.remove("app-status");
            self.functions.selected = None;
            self.functions.operation = None;
            self.functions.challenge = None;
            self.functions.result = None;
            self.functions.uncertain = false;
        }
        self.select_function(spec, window, cx);
        if action == "app-edit"
            && let Some(backup) = backup
        {
            self.functions.text.update(cx, |input, cx| {
                input.set_value(
                    format!("名称={}\n备注={}", backup.name, backup.note),
                    window,
                    cx,
                )
            });
        }
        cx.notify();
    }
    pub(super) fn function_has_text(&self, spec: &OperationSpec) -> bool {
        if matches!(spec.id, "apple-logout" | "cloud-logout") {
            return false;
        }
        let contextual_backup =
            self.functions.environment.open && self.functions.environment.backup.is_some();
        spec.fields.len()
            > usize::from(needs_app(spec))
                + usize::from(contextual_backup && spec.fields.contains(&"backup_id"))
    }
    pub(super) fn render_environment_context(&self, cx: &Context<Self>) -> impl IntoElement {
        let target = self.functions.target_app.as_ref();
        crate::ui::tool_actions()
            .when_some(target, |row, (id, name)| {
                row.child(app_icon(self.apps.icon_for(id), 24.))
                    .child(div().text_sm().child(name.clone()))
            })
            .when_some(self.functions.environment.backup.as_ref(), |row, backup| {
                row.child(
                    div()
                        .text_xs()
                        .text_color(theme::muted(cx))
                        .text_ellipsis()
                        .child(backup.name.clone()),
                )
            })
    }
    pub(super) fn render_app_environment(
        &self,
        width: f32,
        height: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editor = self.function_editor_visible();
        let side = width >= 900. && height >= 480.;
        h_flex()
            .id("function-panel")
            .debug_selector(|| "function-panel".into())
            .absolute()
            .top(px(8.))
            .right(px(8.))
            .gap(px(10.))
            .items_start()
            .when(editor, |panel| {
                panel.child(self.render_function_editor(width, height, side, cx))
            })
            .when(!editor || side, |panel| {
                panel.child(self.render_environment_browser(width, height, cx))
            })
            .into_any_element()
    }
    fn render_environment_browser(
        &self,
        width: f32,
        height: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let locked = self.environment_locked();
        let disabled = locked
            || !self.functions.available
            || self.view_only()
            || self.phase != Phase::Connected;
        let catalog = self.functions.environment.catalog_open;
        let environment_apps: Vec<_> = if catalog {
            self.apps
                .installed
                .iter()
                .filter(|app| backup_candidate(&app.bundle_id))
                .map(|app| (app.bundle_id.clone(), app.name.clone()))
                .collect()
        } else {
            self.functions
                .environment
                .apps
                .iter()
                .filter(|app| {
                    backup_candidate(&app.bundle_id)
                        && self
                            .apps
                            .installed
                            .iter()
                            .any(|installed| installed.bundle_id == app.bundle_id)
                })
                .map(|app| (app.bundle_id.clone(), app.name.clone()))
                .collect()
        };
        let target = self.functions.target_app.clone();
        crate::ui::tool_panel(cx)
            .id("app-environment")
            .debug_selector(|| "app-environment".into())
            .w(px(360f32.min(width - 16.)))
            .max_h(px((height - 16.).max(100.)))
            .overflow_y_scroll()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .when(target.is_some(), |panel| {
                panel.child(
                    crate::ui::tool_actions()
                        .child(
                            Button::new("environment-back")
                                .ghost()
                                .icon(IconName::ChevronLeft)
                                .disabled(locked)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if this.functions.target_app.is_some() {
                                        this.open_app_environment(None, window, cx);
                                    } else {
                                        this.functions.environment.open = false;
                                        this.functions.selected = None;
                                        cx.notify();
                                    }
                                })),
                        )
                        .when_some(target.as_ref(), |row, (id, _)| {
                            row.child(app_icon(self.apps.icon_for(id), 24.))
                        })
                        .child(
                            div().flex_1().font_semibold().child(
                                target
                                    .as_ref()
                                    .map(|(_, name)| name.clone())
                                    .unwrap_or("App 环境与备份".into()),
                            ),
                        )
                        .child(
                            Button::new("environment-close")
                                .ghost()
                                .icon(IconName::Close)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.close_function_panel(window, cx);
                                })),
                        ),
                )
            })
            .when(target.is_none(), |panel| {
                panel
                    .child(self.render_function_navigation(cx))
                    .when(catalog, |panel| {
                        panel.child(
                            h_flex()
                                .justify_between()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme::muted(cx))
                                        .child("添加环境 App"),
                                )
                                .child(
                                    Button::new("environment-catalog-done")
                                        .ghost()
                                        .label("完成")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.functions.environment.catalog_open = false;
                                            cx.notify();
                                        })),
                                ),
                        )
                    })
                    .child(
                        h_flex()
                            .flex_wrap()
                            .items_start()
                            .children(environment_apps.iter().map(|(id, name)| {
                                let id = id.clone();
                                let name = name.clone();
                                let selected = self
                                    .functions
                                    .environment
                                    .apps
                                    .iter()
                                    .any(|app| app.bundle_id == id);
                                v_flex()
                                    .id(SharedString::from(format!("environment-app-{id}")))
                                    .debug_selector({
                                        let id = id.clone();
                                        move || format!("environment-app-{id}")
                                    })
                                    .relative()
                                    .w(px(80.))
                                    .h(px(80.))
                                    .gap_2()
                                    .items_center()
                                    .justify_center()
                                    .rounded(theme::radius(cx))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(theme::hover(cx)))
                                    .child(app_icon(self.apps.icon_for(&id), 38.))
                                    .child(
                                        div()
                                            .w(px(70.))
                                            .text_xs()
                                            .text_center()
                                            .text_ellipsis()
                                            .child(name.clone()),
                                    )
                                    .when(catalog && selected, |tile| {
                                        tile.child(
                                            div()
                                                .absolute()
                                                .top_1()
                                                .right_1()
                                                .text_color(theme::accent(cx))
                                                .child(Icon::new(IconName::Check)),
                                        )
                                    })
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if catalog {
                                            this.toggle_environment_app(&id, &name, cx);
                                        } else {
                                            this.choose_environment_app(&id, &name, window, cx);
                                        }
                                    }))
                            }))
                            .when(!catalog, |grid| {
                                grid.child(
                                    div().size(px(80.)).child(
                                        Button::new("environment-add-app")
                                            .debug_selector(|| "environment-add-app".into())
                                            .ghost()
                                            .size_full()
                                            .accessibility_label("添加 App")
                                            .child(
                                                v_flex().gap_2().items_center().child(
                                                    div()
                                                        .size(px(38.))
                                                        .border_1()
                                                        .rounded(theme::radius(cx))
                                                        .border_color(theme::line(cx))
                                                        .flex()
                                                        .items_center()
                                                        .justify_center()
                                                        .child(Icon::new(IconName::Plus)),
                                                ),
                                            )
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                if this.environment_locked() {
                                                    return;
                                                }
                                                this.functions.environment.catalog_open = true;
                                                this.apps.picker_open = false;
                                                this.refresh_apps(cx);
                                                cx.notify();
                                            })),
                                    ),
                                )
                            }),
                    )
                    .when(environment_apps.is_empty(), |panel| {
                        panel.child(div().text_color(theme::muted(cx)).child(if catalog {
                            "暂无可添加的 App"
                        } else {
                            "点击 + 添加 App"
                        }))
                    })
            })
            .when_some(target.as_ref(), |panel, (_id, _)| {
                panel
                    .child(
                        crate::ui::tool_actions()
                            .debug_selector(|| "environment-actions".into())
                            .child(div().flex_1())
                            .child(
                                Button::new("environment-refresh")
                                    .ghost()
                                    .debug_selector(|| "environment-refresh".into())
                                    .icon(FunctionIcon::Refresh)
                                    .tooltip("刷新备份")
                                    .accessibility_label("刷新备份")
                                    .disabled(disabled)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.refresh_environment(cx)),
                                    ),
                            ),
                    )
                    .child(
                        v_flex()
                            .id("environment-backups")
                            .gap_1()
                            .max_h(px(270.))
                            .overflow_y_scroll()
                            .children(
                                self.functions
                                    .environment
                                    .records
                                    .iter()
                                    .map(|backup| self.render_backup_row(backup, disabled, cx)),
                            ),
                    )
                    .when(
                        self.functions.environment.loaded
                            && self.functions.environment.records.is_empty()
                            && self.functions.environment.baselines.is_empty(),
                        |panel| {
                            panel.child(div().py_2().text_color(theme::muted(cx)).child("暂无备份"))
                        },
                    )
            })
            .when(
                self.functions.busy
                    && self
                        .functions
                        .selected
                        .is_some_and(|spec| spec.id == "app-list"),
                |panel| {
                    panel.child(super::function_loading(
                        "function-backup-loading",
                        "App 备份",
                        "正在读取备份…".into(),
                        cx,
                    ))
                },
            )
            .when(
                self.functions.environment.run.is_none()
                    && !self.functions.message.is_empty()
                    && !(self.functions.busy
                        && self
                            .functions
                            .selected
                            .is_some_and(|spec| spec.id == "app-list")),
                |panel| {
                    panel.child(
                        div()
                            .text_xs()
                            .text_color(theme::muted(cx))
                            .child(self.functions.message.clone()),
                    )
                },
            )
    }
    fn confirm_rv_takeover(&mut self, backup: Backup, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_handoff_backup() {
            return;
        }
        let Some(device) = self.req.connection_id else {
            return;
        };
        let Some(book) = self.address_book.as_ref().and_then(WeakEntity::upgrade) else {
            return;
        };
        let name = book.read(cx).takeover_phone_name(device);
        let owner = cx.entity();
        crate::ui::confirm_execution_handoff(
            crate::ui::ExecutionHandoffPrompt {
                title: "交给 RV",
                description: crate::ui::RV_HANDOFF_DESCRIPTION,
                icon: FunctionIcon::Monitor,
                device: name,
                backup: backup.name.clone(),
                acknowledgement: Some("已在 Shadow 保存最新备份，并切换到其他环境"),
            },
            window,
            cx,
            move |_, cx| {
                owner.update(cx, |this, cx| this.start_rv_takeover(backup.clone(), cx));
            },
        );
    }
    fn start_rv_takeover(&mut self, backup: Backup, cx: &mut Context<Self>) {
        let result = (|| {
            if !self.can_handoff_backup() {
                return Err("请先连接可写手机并完成当前操作".into());
            }
            let device = self.req.connection_id.ok_or("手机连接未保存")?;
            let id = rv_core::workspace::AppId::from_idfv(
                backup
                    .id
                    .strip_prefix("snapshot:")
                    .ok_or("请选择账号备份")?,
            )
            .map_err(|_| "备份身份无效")?;
            let book = self
                .address_book
                .as_ref()
                .and_then(WeakEntity::upgrade)
                .ok_or("本地账号列表不可用")?;
            let owner = cx.entity().entity_id();
            let command = book.update(cx, |book, cx| {
                book.prepare_rv_takeover(&id, &backup.app, &backup.id, device, owner, cx)
            })?;
            self.handle.app(command);
            Ok::<_, String>(())
        })();
        match result {
            Ok(()) => self.functions.message = "正在读取手机备份并交给 RV…".into(),
            Err(error) => {
                tracing::error!(bundle_id=%backup.app,error=%error,"RV takeover start failed");
                self.functions.message = error.into();
            }
        }
        cx.notify();
    }
    pub(in crate::session_window) fn rv_takeover_result(
        &mut self,
        saved: bool,
        cx: &mut Context<Self>,
    ) {
        self.functions.message = if saved {
            "已采用手机备份凭据，Token 由 RV 刷新"
        } else {
            "交给 RV 未完成，原凭据和 Token 来源保持不变"
        }
        .into();
        cx.notify();
    }
    fn confirm_sync_backup_to_home(
        &mut self,
        backup: Backup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let owner = cx.entity();
        crate::ui::confirm_backup_sync(backup.name.clone(), window, cx, move |_, cx| {
            owner.update(cx, |this, cx| this.sync_backup_to_home(backup.clone(), cx));
        });
    }
    fn sync_backup_to_home(&mut self, backup: Backup, cx: &mut Context<Self>) {
        let result = (|| {
            if self.phase != Phase::Connected
                || self.view_only()
                || self.functions.app_state_unsettled()
            {
                return Err("请先连接手机并完成当前操作".to_string());
            }
            let device = self.req.connection_id.ok_or("备份来源设备未保存")?;
            let id = rv_core::workspace::AppId::from_idfv(
                backup
                    .id
                    .strip_prefix("snapshot:")
                    .ok_or("请选择账号备份")?,
            )
            .map_err(|_| "备份身份无效，请刷新列表".to_string())?;
            let book = self
                .address_book
                .as_ref()
                .and_then(WeakEntity::upgrade)
                .ok_or("本地账号列表不可用")?;
            book.update(cx, |book, cx| {
                book.reimport_phone_backup(device, &backup.app, &id, &backup.id, cx)
            })?;
            if backup.app == "lt.manodrabuziai.fr" {
                let owner = cx.entity().entity_id();
                if !book.update(cx, |book, cx| {
                    book.begin_backup_sync(&id, device, owner, cx)
                }) {
                    return Err("此账号正在同步，请稍后重试".into());
                }
                if !self.read_home_profile(id.clone(), backup.app.clone(), backup.id.clone(), cx) {
                    book.update(cx, |book, cx| book.backup_sync_failed(&id, owner, cx));
                    return Err("请先连接手机并完成当前备份操作".into());
                }
            }
            Ok::<_, String>(())
        })();
        match result {
            Ok(()) => {
                self.functions.message = if backup.app == "lt.manodrabuziai.fr" {
                    "正在从手机读取账号备份…"
                } else {
                    "备份信息已同步到本地"
                }
                .into()
            }
            Err(error) => {
                tracing::error!(bundle_id=%backup.app,error=%error,"Backup local sync failed");
                self.functions.message = error.into();
            }
        }
        cx.notify();
    }
    fn render_backup_row(
        &self,
        backup: &Backup,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let takeover_target = backup.clone();
        let menu_backup = backup.clone();
        let entity = cx.entity();
        let focus = self.focus.clone();
        let mode = backup
            .id
            .strip_prefix("snapshot:")
            .and_then(|id| rv_core::workspace::AppId::from_idfv(id).ok())
            .and_then(|id| {
                self.address_book
                    .as_ref()
                    .and_then(WeakEntity::upgrade)
                    .map(|book| book.read(cx).execution_mode(&id))
            })
            .unwrap_or_default();
        let show_sync = backup.snapshot && mode == rv_core::workspace::ExecutionMode::Rv;
        let can_takeover = self
            .address_book
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .is_some_and(|book| {
                book.read(cx)
                    .rv_takeover_available(&backup.app, &backup.id, self.req.connection_id)
            });
        let current = self.functions.environment.current.as_ref() == Some(&backup.id);
        h_flex()
            .gap_2()
            .py_2()
            .border_b_1()
            .border_color(theme::line(cx))
            .items_center()
            .child(
                div().self_start().pt_1().child(
                    Icon::new(if current {
                        IconName::CircleCheck
                    } else {
                        IconName::Folder
                    })
                    .text_color(if current {
                        theme::success(cx)
                    } else {
                        theme::muted(cx)
                    }),
                ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .font_medium()
                            .text_ellipsis()
                            .child(backup.name.clone()),
                    )
                    .when(!backup.note.is_empty(), |row| {
                        row.child(
                            div()
                                .text_xs()
                                .text_color(theme::muted(cx))
                                .text_ellipsis()
                                .child(backup.note.clone()),
                        )
                    })
                    .child(div().text_xs().text_color(theme::muted(cx)).child(
                        if current && self.functions.environment.restore_pending {
                            format!("{} · 恢复未完成", backup.date)
                        } else {
                            backup.date.clone()
                        },
                    )),
            )
            .child(
                Button::new(format!("takeover-{}", backup.id))
                    .ghost()
                    .debug_selector({
                        let id = backup.id.clone();
                        move || format!("takeover-{id}")
                    })
                    .icon(if mode == rv_core::workspace::ExecutionMode::ToPhone {
                        FunctionIcon::Recent
                    } else {
                        FunctionIcon::Monitor
                    })
                    .tooltip(if can_takeover { "交给 RV" } else { match mode {
                        rv_core::workspace::ExecutionMode::Phone => "交给 RV",
                        rv_core::workspace::ExecutionMode::ToPhone => "交接未完成",
                        rv_core::workspace::ExecutionMode::Rv => "RV 接管",
                    } })
                    .accessibility_label("交给 RV")
                    .disabled(disabled || !can_takeover)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.confirm_rv_takeover(takeover_target.clone(), window, cx)
                    })),
            )
            .child(
                Button::new(format!("backup-menu-{}", backup.id))
                    .ghost()
                    .icon(IconName::Ellipsis)
                    .tooltip("更多")
                    .accessibility_label("更多")
                    .disabled(disabled)
                    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                        let edit = entity.clone();
                        let sync = entity.clone();
                        let sync_backup = menu_backup.clone();
                        let edit_backup = menu_backup.clone();
                        let copy = menu_backup.id.clone();
                        menu.action_context(focus.clone())
                            .when(show_sync, |menu| {
                                menu.item(
                                    PopupMenuItem::new("从手机同步账号备份…")
                                        .icon(IconName::Redo)
                                        .on_click(move |_, window, cx| {
                                            sync.update(cx, |this, cx| {
                                                this.confirm_sync_backup_to_home(
                                                    sync_backup.clone(),
                                                    window,
                                                    cx,
                                                )
                                            })
                                        }),
                                )
                            })
                            .item(
                                PopupMenuItem::new("编辑名称与备注")
                                    .icon(FunctionIcon::Edit)
                                    .disabled(!menu_backup.snapshot)
                                    .on_click(move |_, window, cx| {
                                        edit.update(cx, |this, cx| {
                                            this.environment_action(
                                                "app-edit",
                                                Some(edit_backup.clone()),
                                                window,
                                                cx,
                                            )
                                        });
                                    }),
                            )
                            .item(
                                PopupMenuItem::new("复制备份标识")
                                    .icon(IconName::Copy)
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copy.clone(),
                                        ))
                                    }),
                            )
                    }),
            )
            .into_any_element()
    }
}
pub(super) fn environment_progress_label(progress: &serde_json::Value) -> String {
    let phase = match progress["phase"].as_str() {
        Some("backup") => "备份当前环境",
        Some("restore") => "恢复目标环境",
        Some("baseline") => "创建基线",
        _ => "处理环境",
    };
    let stage = match progress["stage"].as_str() {
        Some("prepare") => "准备",
        Some("copy") => "复制文件",
        Some("verify_files") => "校验文件",
        Some("keychain") | Some("verify_keychain") => "处理凭据",
        Some("identity") | Some("verify_identity") => "处理身份",
        Some("publish") => "保存备份",
        Some("preferences") | Some("verify_preferences") => "刷新设置",
        Some("complete") => "完成",
        _ => "",
    };
    if progress.is_null() {
        String::new()
    } else if stage.is_empty() {
        phase.into()
    } else {
        format!("{phase} · {stage}")
    }
}
pub(super) fn example(spec: &OperationSpec, has_backup: bool) -> String {
    function_example(spec)
        .lines()
        .filter(|line| !has_backup || !line.starts_with("备份="))
        .collect::<Vec<_>>()
        .join("\n")
}
pub(super) fn operation_text(
    spec: &OperationSpec,
    app: &str,
    backup: Option<&str>,
    text: &str,
) -> String {
    if let Some(backup) = backup.filter(|_| spec.fields.contains(&"backup_id")) {
        let named = text.trim().is_empty()
            || text
                .lines()
                .find(|line| !line.trim().is_empty())
                .is_some_and(|line| {
                    line.split_once('=').is_some_and(|(key, _)| {
                        matches!(
                            key.trim(),
                            "App"
                                | "应用"
                                | "bundle_id"
                                | "备份"
                                | "backup_id"
                                | "名称"
                                | "display_name"
                                | "备注"
                                | "note"
                                | "放弃当前环境"
                                | "discard_current"
                        )
                    })
                });
        if named {
            format!("App={app}\n备份={backup}\n{text}")
        } else {
            format!("{app}----{backup}----{text}")
        }
    } else {
        function_app_text(app, text)
    }
}

#[cfg(test)]
impl SessionView {
    pub(in crate::session_window) fn preview_environment(
        &mut self,
        state: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut shortcuts = rv_core::default_app_shortcuts();
        shortcuts.extend([
            rv_core::AppShortcut {
                bundle_id: "fr.vinted".into(),
                name: "Vinted".into(),
            },
            rv_core::AppShortcut {
                bundle_id: "com.hydra.projectx".into(),
                name: "ProjectX".into(),
            },
        ]);
        self.functions.environment.apps = shortcuts
            .iter()
            .filter(|app| backup_candidate(&app.bundle_id))
            .cloned()
            .collect();
        let apps = shortcuts
            .into_iter()
            .map(|app| rv_session::RemoteApp {
                bundle_id: app.bundle_id,
                name: app.name,
                can_launch: true,
                can_terminate: true,
            })
            .collect();
        self.apply_app_event(
            rv_session::AppEvent::Capabilities(rv_session::AppCapabilities {
                list: true,
                launch: true,
                terminate: true,
                control: true,
                ..Default::default()
            }),
            cx,
        );
        self.apply_app_event(rv_session::AppEvent::List(apps), cx);
        self.apply_function_event(FunctionEvent::Available(true), cx);
        self.open_app_environment(None, window, cx);
        if state == "common" || state == "catalog" {
            self.functions.environment.catalog_open = state == "catalog";
            return;
        }
        self.choose_environment_app("fr.vinted", "Vinted", window, cx);
        self.apply_function_event(FunctionEvent::State {task:32,record:serde_json::json!({"operation_id":"preview-list","status":"completed","result":{"bundle_id":"fr.vinted","current_backup_id":"snapshot:current","backups":[
            {"backup_id":"snapshot:current","kind":"snapshot","display_name":"工作账号 A","note":"日常使用","date":"2026-10-10 10:20"},
            {"backup_id":"snapshot:spare","kind":"snapshot","display_name":"备用账号 B","note":"备用环境","date":"2026-10-09 18:45"},
            {"backup_id":"baseline:1","kind":"baseline","display_name":"空白基线","note":"首次启动前","date":"2026-10-08 09:12"}
        ]}})}, cx);
        if state == "save" {
            self.environment_action("app-edit", None, window, cx);
            self.functions.text.update(cx, |input, cx| {
                input.set_value("名称=工作账号 C\n备注=备用环境", window, cx)
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui_component::WindowExt;
    fn unscoped(cmd: rv_session::SessionCommand) -> rv_session::SessionCommand {
        match cmd {
            rv_session::SessionCommand::Function(FunctionCommand::Scoped { command, .. }) => {
                rv_session::SessionCommand::Function(*command)
            }
            c => c,
        }
    }

    use super::super::super::ui_tests::setup;
    use super::super::{FunctionCommand, FunctionEvent, OPERATIONS, parse_operation};
    use super::{Backup, SessionView, operation_text};
    use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, px, size};
    use rv_session::SessionCommand;

    fn fixture(view: &Entity<SessionView>, cx: &mut VisualTestContext) {
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preview_environment("records", window, cx)
            })
        });
    }
    #[gpui::test]
    fn home_backup_browse_keeps_active_and_stashed_operations(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        fixture(&view, cx);
        while peer.commands.try_recv().is_ok() {}
        for stashed in [false, true] {
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.functions.selected = OPERATIONS.iter().find(|s| s.id == "app-edit");
                    view.functions.busy = true;
                    view.functions.operation = Some((32, "original-operation".into()));
                    if stashed {
                        view.functions.stash_panel();
                        view.functions.selected = None;
                        view.functions.busy = false;
                        view.functions.operation = None;
                    }
                    let selected = view.functions.selected.map(|s| s.id);
                    let operation = view.functions.operation.clone();
                    view.show_home_environment(Some("other.app".into()), window, cx);
                    assert_eq!(view.functions.selected.map(|s| s.id), selected);
                    assert_eq!(view.functions.operation, operation);
                    assert!(view.functions.app_state_unsettled());
                    view.requested_environment = Some("other.app".into());
                    view.tick_home_environment(cx);
                    assert_eq!(view.requested_environment.as_deref(), Some("other.app"));
                })
            });
            assert!(peer.commands.try_recv().is_err());
        }
    }
    #[gpui::test]
    fn environment_catalog_is_independent_and_excludes_system_apps(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preview_environment("common", window, cx)
            })
        });
        let shortcuts = view.read_with(cx, |view, _| view.apps.shortcuts.clone());
        while peer.commands.try_recv().map(unscoped).is_ok() {}
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(
            cx.debug_bounds("environment-app-com.apple.mobilesafari")
                .is_none()
        );
        assert!(cx.debug_bounds("environment-app-fr.vinted").is_some());
        let add = cx.debug_bounds("environment-add-app").unwrap();
        cx.simulate_click(add.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        view.read_with(cx, |view, _| {
            assert!(!view.apps.picker_open);
            assert!(view.functions.environment.catalog_open);
        });
        assert!(cx.debug_bounds("app-picker").is_none());
        assert!(
            cx.debug_bounds("environment-app-com.apple.AppStore")
                .is_none()
        );
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.toggle_environment_app("fr.vinted", "Vinted", cx);
                assert!(
                    !view
                        .functions
                        .environment
                        .apps
                        .iter()
                        .any(|app| app.bundle_id == "fr.vinted")
                );
                assert_eq!(view.apps.shortcuts, shortcuts);
                view.choose_environment_app("com.apple.mobilesafari", "Safari", window, cx);
                assert!(view.functions.target_app.is_none());
                view.toggle_environment_app("com.apple.AppStore", "App Store", cx);
                assert!(
                    !view
                        .functions
                        .environment
                        .apps
                        .iter()
                        .any(|app| app.bundle_id.starts_with("com.apple."))
                );
            })
        });
        assert!(
            std::iter::from_fn(|| peer.commands.try_recv().map(unscoped).ok())
                .all(|command| !matches!(command, SessionCommand::Function(_)))
        );
    }
    #[gpui::test]
    fn app_first_grid_and_record_takeover_replace_switch(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.preview_environment("common", window, cx)
            })
        });
        while peer.commands.try_recv().map(unscoped).is_ok() {}
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("environment-app-fr.vinted").is_some());
        assert!(cx.debug_bounds("function-execute").is_none());
        let app = cx.debug_bounds("environment-app-fr.vinted").unwrap();
        cx.simulate_click(app.center(), Modifiers::default());
        let commands: Vec<_> =
            std::iter::from_fn(|| peer.commands.try_recv().map(unscoped).ok()).collect();
        assert_eq!(
            commands
                .iter()
                .filter(|cmd| matches!(cmd, SessionCommand::Function(_)))
                .count(),
            1
        );
        assert!(commands.iter().any(|cmd|matches!(cmd,SessionCommand::Function(FunctionCommand::Invoke{id,text}) if id=="app-list" && text=="App=fr.vinted")));
        fixture(&view, cx);
        while peer.commands.try_recv().map(unscoped).is_ok() {}
        cx.simulate_resize(size(px(640.), px(400.)));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("switch-baseline:1").is_none());
        let actions = cx.debug_bounds("environment-actions").unwrap();
        for selector in ["environment-refresh"] {
            let button = cx.debug_bounds(selector).unwrap();
            assert!(button.left() >= actions.left() && button.right() <= actions.right());
            assert!(button.top() >= actions.top() && button.bottom() <= actions.bottom());
        }
        view.read_with(cx, |view, _| {
            assert_eq!(view.functions.environment.records.len(), 2);
            assert!(
                view.functions
                    .environment
                    .records
                    .iter()
                    .all(|backup| backup.snapshot)
            );
            assert_eq!(view.functions.environment.records[0].name, "工作账号 A");
        });
        assert!(cx.debug_bounds("switch-snapshot:spare").is_none());
        let row = cx.debug_bounds("takeover-snapshot:spare").unwrap();
        cx.simulate_click(row.center(), Modifiers::default());
        cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
        assert!(
            std::iter::from_fn(|| peer.commands.try_recv().map(unscoped).ok())
                .all(|command| !matches!(command, SessionCommand::Function(_)))
        );
    }
    #[gpui::test]
    fn stale_record_and_readonly_do_not_dispatch_mutations(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        fixture(&view, cx);
        while peer.commands.try_recv().map(unscoped).is_ok() {}
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let stale = Backup {
                    app: "com.other.app".into(),
                    id: "snapshot:spare".into(),
                    name: "stale".into(),
                    note: "".into(),
                    date: "".into(),
                    snapshot: true,
                };
                view.environment_action("app-delete", Some(stale), window, cx);
                view.execute_function(window, cx);
            })
        });
        assert!(peer.commands.try_recv().map(unscoped).is_err());
        let (readonly, cx, mut peer) = setup(cx, true);
        fixture(&readonly, cx);
        while peer.commands.try_recv().map(unscoped).is_ok() {}
        cx.update(|window, cx| {
            readonly.update(cx, |view, cx| {
                view.functions.busy = false;
                view.environment_action("app-edit", None, window, cx);
                view.functions
                    .text
                    .update(cx, |input, cx| input.set_value("名称=不应保存", window, cx));
                view.execute_function(window, cx);
            })
        });
        assert!(peer.commands.try_recv().map(unscoped).is_err());
    }
    #[gpui::test]
    fn background_app_list_does_not_replace_visible_environment(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        fixture(&view, cx);
        while peer.commands.try_recv().is_ok() {}
        cx.update(|window, cx| view.update(cx, |view, cx| {
            view.functions.selected = OPERATIONS.iter().find(|s| s.id == "app-list");
            view.functions.busy = true;
            view.functions.stash_panel();
            view.select_function(OPERATIONS.iter().find(|s| s.id == "app-edit").unwrap(), window, cx);
            let before = view.functions.environment.records.iter().map(|r| r.id.clone()).collect::<Vec<_>>();
            view.apply_function_event(FunctionEvent::ForOperation { id:"app-list".into(), event:Box::new(FunctionEvent::State {task:32,record:serde_json::json!({
                "operation_id":"old-list","status":"completed","result":{"bundle_id":"fr.vinted","backups":[{"backup_id":"background-only"}]}
            })})}, cx);
            assert_eq!(view.functions.environment.records.iter().map(|r| r.id.clone()).collect::<Vec<_>>(), before);
            assert_eq!(view.functions.selected.unwrap().id,"app-edit");
            assert!(view.functions.environment.open);
        }));
        assert!(peer.commands.try_recv().is_err());
    }
    #[gpui::test]
    fn completed_edit_refreshes_list_but_unknown_never_replays(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        fixture(&view, cx);
        while peer.commands.try_recv().map(unscoped).is_ok() {}
        cx.update(|window,cx|view.update(cx,|view,cx| {
            view.environment_action("app-edit",None,window,cx);
            view.functions.busy=true;
            view.apply_function_event(FunctionEvent::State {task:32,record:serde_json::json!({"operation_id":"save-1","status":"completed","result":{}})},cx);
        }));
        assert!(
            matches!(peer.commands.try_recv().map(unscoped),Ok(SessionCommand::Function(FunctionCommand::Invoke{id,..})) if id=="app-list")
        );
        view.update(cx,|view,cx| {
            view.functions.selected=OPERATIONS.iter().find(|spec|spec.id=="app-edit");
            view.apply_function_event(FunctionEvent::State {task:32,record:serde_json::json!({"operation_id":"save-2","status":"completed","error":{"domain":"TVAppStateServices","code":3}})},cx);
            assert!(!view.functions.uncertain);
            assert_eq!(view.functions.selected.unwrap().id, "app-list");
        });
        assert!(
            matches!(peer.commands.try_recv().map(unscoped), Ok(SessionCommand::Function(FunctionCommand::Invoke { id, .. })) if id == "app-list")
        );
    }
    #[gpui::test]
    fn app_grid_returns_to_regular_functions_and_reopens_cleanly(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        fixture(&view, cx);
        while peer.commands.try_recv().map(unscoped).is_ok() {}
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.open_app_environment(None, window, cx);
                view.return_function_menu(window, cx);
                assert!(!view.functions.environment.open);
                assert!(view.functions.open);
                assert_eq!(view.functions.category, "Apple");
                view.apply_function_event(
                    FunctionEvent::AccountState(Ok(serde_json::json!({
                        "app_store":{"status":"not_signed_in","account":null}
                    }))),
                    cx,
                );
                view.select_function(&OPERATIONS[0], window, cx);
                assert_eq!(view.functions.selected.unwrap().task, 31);
                view.open_app_environment(None, window, cx);
                view.functions.open = false;
                view.toggle_functions(window, cx);
                assert!(!view.functions.environment.open);
                assert!(view.functions.selected.is_none());
            })
        });
        assert!(peer.commands.try_recv().map(unscoped).is_err());
    }
    #[gpui::test]
    fn backup_toolbar_only_exposes_refresh(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        fixture(&view, cx);
        while peer.commands.try_recv().is_ok() {}
        cx.update(|window, cx| window.draw(cx).clear(cx));
        for selector in [
            "environment-launch",
            "environment-stop",
            "environment-status",
            "environment-save",
            "app-baseline",
            "app-describe",
        ] {
            assert!(
                cx.debug_bounds(selector).is_none(),
                "obsolete toolbar action {selector}"
            );
        }
        let refresh = cx.debug_bounds("environment-refresh").unwrap();
        cx.simulate_click(refresh.center(), Modifiers::default());
        assert!(std::iter::from_fn(||peer.commands.try_recv().map(unscoped).ok()).any(|cmd|matches!(cmd,SessionCommand::Function(FunctionCommand::Invoke{id,..}) if id=="app-list")));
    }
    #[test]
    fn record_identifiers_are_injected_and_cannot_be_overridden() {
        let spec = OPERATIONS
            .iter()
            .find(|spec| spec.id == "app-edit")
            .unwrap();
        let text = operation_text(
            spec,
            "com.example.app",
            Some("snapshot:opaque"),
            "工作账号 A----备用环境",
        );
        let operation = parse_operation(spec, &text).unwrap();
        assert_eq!(operation.params["backup_id"], "snapshot:opaque");
        assert!(
            parse_operation(
                spec,
                &operation_text(
                    spec,
                    "com.example.app",
                    Some("snapshot:opaque"),
                    "备份=snapshot:other"
                )
            )
            .is_err()
        );
    }
}
