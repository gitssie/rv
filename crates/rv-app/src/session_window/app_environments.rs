use super::super::apps_ui::app_icon;
use super::*;
use gpui_component::{button::ButtonCustomVariant, popover::Popover};
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
#[derive(Clone)]
pub(super) struct EnvironmentUi {
    pub open: bool,
    apps: Vec<rv_core::AppShortcut>,
    catalog_open: bool,
    records: Vec<Backup>,
    current: Option<String>,
    loaded: bool,
    restore_pending: bool,
    pub backup: Option<Backup>,
}
impl EnvironmentUi {
    pub fn new(apps: Vec<rv_core::AppShortcut>) -> Self {
        Self {
            open: false,
            apps: apps
                .into_iter()
                .filter(|app| backup_candidate(&app.bundle_id))
                .collect(),
            catalog_open: false,
            records: vec![],
            current: None,
            loaded: false,
            restore_pending: false,
            backup: None,
        }
    }
    pub fn clear_records(&mut self) {
        self.records.clear();
        self.current = None;
        self.loaded = false;
        self.restore_pending = false;
        self.backup = None;
    }
}
impl SessionView {
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
        self.functions
            .selected
            .is_some_and(|spec| !(self.functions.environment.open && spec.id == "app-list"))
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
        self.functions.target_app = Some((id.into(), name.into()));
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
        if self.functions.uncertain || record.get("error").is_some() {
            return;
        }
        let Some(spec) = self.functions.selected else {
            return;
        };
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
            self.functions.environment.records = result["backups"]
                .as_array()
                .unwrap()
                .iter()
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
            self.functions.environment.current =
                result["current_backup_id"].as_str().map(str::to_owned);
            self.functions.environment.restore_pending =
                result["restore_pending"].as_bool().unwrap_or(false);
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
                    .any(|row| row.id == backup.id && row.app == backup.app))
        {
            return;
        }
        let Some(spec) = OPERATIONS.iter().find(|spec| spec.id == action) else {
            return;
        };
        self.functions.environment.backup = backup.clone();
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
    pub(super) fn render_environment_context(&self, _cx: &Context<Self>) -> impl IntoElement {
        let target = self.functions.target_app.as_ref();
        h_flex()
            .gap_2()
            .py_1()
            .when_some(target, |row, (id, name)| {
                row.child(app_icon(self.apps.icon_for(id), 24.))
                    .child(div().text_size(px(12.)).child(name.clone()))
            })
            .when_some(self.functions.environment.backup.as_ref(), |row, backup| {
                row.child(
                    div()
                        .text_xs()
                        .text_color(theme::toolbar_muted())
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
        let editor = self
            .functions
            .selected
            .is_some_and(|spec| spec.id != "app-list");
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
        v_flex()
            .id("app-environment")
            .debug_selector(|| "app-environment".into())
            .w(px(360f32.min(width - 16.)))
            .max_h(px((height - 16.).max(100.)))
            .overflow_y_scroll()
            .p_2()
            .gap_2()
            .rounded_lg()
            .border_1()
            .border_color(theme::toolbar_line())
            .bg(theme::toolbar())
            .text_color(theme::toolbar_fg())
            .shadow_lg()
            .text_size(px(12.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .when(target.is_some(), |panel| {
                panel.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            Button::new("environment-back")
                                .ghost()
                                .text_color(theme::toolbar_fg())
                                .size(px(24.))
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
                                .text_color(theme::toolbar_fg())
                                .size(px(24.))
                                .icon(IconName::Close)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.functions.open = false;
                                    this.clear_function_text(window, cx);
                                    this.focus.focus(window, cx);
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when(target.is_none(), |panel| {
                panel
                    .when(self.functions.environment.restore_pending, |panel| {
                        panel.child(div().text_xs().child(
                            "恢复未完成：请重新切换目标备份或查看运行状态，完成恢复后再保存。",
                        ))
                    })
                    .child(self.render_function_navigation(cx))
                    .when(catalog, |panel| {
                        panel.child(
                            h_flex()
                                .justify_between()
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(theme::toolbar_muted())
                                        .child("添加环境 App"),
                                )
                                .child(
                                    Button::new("environment-catalog-done")
                                        .ghost()
                                        .small()
                                        .h(px(24.))
                                        .text_color(theme::toolbar_fg())
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
                                    .rounded_md()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(rgb(0x29333e)))
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
                                                .child(Icon::new(IconName::Check).size(px(12.))),
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
                                    Button::new("environment-add-app")
                                        .debug_selector(|| "environment-add-app".into())
                                        .ghost()
                                        .text_color(theme::toolbar_fg())
                                        .w(px(80.))
                                        .h(px(80.))
                                        .tooltip("添加 App")
                                        .child(
                                            v_flex().gap_2().items_center().child(
                                                div()
                                                    .size(px(38.))
                                                    .border_1()
                                                    .rounded_md()
                                                    .border_color(theme::toolbar_line())
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .child(Icon::new(IconName::Plus).size(px(20.))),
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
                                )
                            }),
                    )
                    .when(environment_apps.is_empty(), |panel| {
                        panel.child(div().text_color(theme::toolbar_muted()).child(if catalog {
                            "暂无可添加的 App"
                        } else {
                            "点击 + 添加 App"
                        }))
                    })
            })
            .when_some(target.as_ref(), |panel, (id, _)| {
                let launch = id.clone();
                let stop = id.clone();
                let entity = cx.entity();
                panel
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("environment-launch")
                                    .ghost()
                                    .text_color(theme::toolbar_fg())
                                    .small()
                                    .h(px(26.))
                                    .icon(IconName::Play)
                                    .label("打开")
                                    .disabled(!self.app_allowed(id, 2) || locked)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.dispatch_app(AppCommand::Launch(launch.clone()), cx)
                                    })),
                            )
                            .child(
                                Button::new("environment-stop")
                                    .ghost()
                                    .text_color(theme::toolbar_fg())
                                    .small()
                                    .h(px(26.))
                                    .icon(crate::assets::AppActionIcon::Terminate)
                                    .label("停止")
                                    .disabled(!self.app_allowed(id, 3) || locked)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.dispatch_app(AppCommand::Terminate(stop.clone()), cx)
                                    })),
                            )
                            .child(
                                Popover::new("environment-more-menu")
                                    .bg(theme::toolbar())
                                    .text_color(theme::toolbar_fg())
                                    .border_color(theme::toolbar_line())
                                    .p_1()
                                    .rounded_md()
                                    .trigger(
                                        Button::new("environment-more")
                                            .ghost()
                                            .text_color(theme::toolbar_fg())
                                            .small()
                                            .h(px(26.))
                                            .label("更多 ⋯")
                                            .disabled(disabled),
                                    )
                                    .content(move |_, _, cx| {
                                        let state = cx.entity();
                                        v_flex().w(px(144.)).gap_0p5().children(
                                            [
                                                ("创建空白基线", "app-baseline"),
                                                ("环境详情", "app-describe"),
                                                ("运行状态", "app-status"),
                                            ]
                                            .into_iter()
                                            .map(
                                                |(label, action)| {
                                                    let entity = entity.clone();
                                                    let state = state.clone();
                                                    Button::new(action)
                                                        .custom(
                                                            ButtonCustomVariant::new(cx)
                                                                .foreground(theme::toolbar_fg())
                                                                .hover(rgb(0x2a343f).into())
                                                                .active(rgb(0x35404e).into()),
                                                        )
                                                        .small()
                                                        .h(px(26.))
                                                        .w_full()
                                                        .label(label)
                                                        .on_click(move |_, window, cx| {
                                                            state.update(cx, |state, cx| {
                                                                state.dismiss(window, cx)
                                                            });
                                                            entity.update(cx, |this, cx| {
                                                                this.environment_action(
                                                                    action, None, window, cx,
                                                                )
                                                            });
                                                        })
                                                },
                                            ),
                                        )
                                    }),
                            ),
                    )
                    .child(
                        h_flex()
                            .border_t_1()
                            .border_color(theme::toolbar_line())
                            .pt_2()
                            .gap_1()
                            .child(div().flex_1().font_semibold().child("环境与备份"))
                            .child(
                                Button::new("environment-save")
                                    .primary()
                                    .small()
                                    .h(px(26.))
                                    .label("保存当前环境")
                                    .bg(theme::accent(cx))
                                    .text_color(theme::accent_fg(cx))
                                    .disabled(
                                        disabled || self.functions.environment.restore_pending,
                                    )
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.environment_action("app-save", None, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("environment-refresh")
                                    .ghost()
                                    .text_color(theme::toolbar_fg())
                                    .size(px(26.))
                                    .icon(IconName::Redo)
                                    .tooltip("刷新备份")
                                    .disabled(disabled)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.refresh_environment(cx)),
                                    ),
                            ),
                    )
                    .when(self.functions.environment.restore_pending, |panel| {
                        panel.child(div().text_xs().child(
                            "恢复未完成：请重新切换目标备份或查看运行状态，完成恢复后再保存。",
                        ))
                    })
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
                            && self.functions.environment.records.is_empty(),
                        |panel| {
                            panel.child(
                                div()
                                    .py_2()
                                    .text_color(theme::toolbar_muted())
                                    .child("暂无备份，先保存当前环境或创建空白基线"),
                            )
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
                    panel.child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                gpui_component::spinner::Spinner::new()
                                    .small()
                                    .color(theme::toolbar_muted()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::toolbar_muted())
                                    .child("正在读取备份…"),
                            ),
                    )
                },
            )
            .when(
                !self.functions.message.is_empty()
                    && !(self.functions.busy
                        && self
                            .functions
                            .selected
                            .is_some_and(|spec| spec.id == "app-list")),
                |panel| {
                    panel.child(
                        div()
                            .text_xs()
                            .text_color(theme::toolbar_muted())
                            .child(self.functions.message.clone()),
                    )
                },
            )
            .when(self.functions.uncertain, |panel| {
                panel.child(
                    Button::new("environment-resume")
                        .ghost()
                        .text_color(theme::toolbar_fg())
                        .small()
                        .h(px(26.))
                        .label("查询原操作")
                        .disabled(self.functions.busy || !self.functions.available)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some((task, operation_id)) = this.functions.operation.clone() {
                                this.send_function_command(FunctionCommand::Resume {
                                    task,
                                    operation_id,
                                });
                                this.functions.busy = true;
                                cx.notify();
                            }
                        })),
                )
            })
    }
    fn render_backup_row(
        &self,
        backup: &Backup,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let switch = backup.clone();
        let menu_backup = backup.clone();
        let entity = cx.entity();
        let current = self.functions.environment.current.as_ref() == Some(&backup.id);
        h_flex()
            .gap_2()
            .py_2()
            .border_b_1()
            .border_color(theme::toolbar_line())
            .items_center()
            .child(
                Icon::new(IconName::Folder)
                    .size(px(16.))
                    .text_color(theme::toolbar_muted()),
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
                                .text_color(theme::toolbar_muted())
                                .text_ellipsis()
                                .child(backup.note.clone()),
                        )
                    })
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::toolbar_muted())
                            .child(if current {
                                format!(
                                    "{} · {}",
                                    backup.date,
                                    if self.functions.environment.restore_pending {
                                        "恢复未完成"
                                    } else {
                                        "当前环境"
                                    }
                                )
                            } else {
                                backup.date.clone()
                            }),
                    ),
            )
            .child(
                Button::new(format!("switch-{}", backup.id))
                    .ghost()
                    .text_color(theme::toolbar_fg())
                    .debug_selector({
                        let id = backup.id.clone();
                        move || format!("switch-{id}")
                    })
                    .small()
                    .h(px(26.))
                    .label(if current && self.functions.environment.restore_pending {
                        "恢复"
                    } else {
                        "切换"
                    })
                    .disabled(disabled || (current && !self.functions.environment.restore_pending))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.environment_action("app-switch", Some(switch.clone()), window, cx)
                    })),
            )
            .child(
                Popover::new(SharedString::from(format!("backup-actions-{}", backup.id)))
                    .bg(theme::toolbar())
                    .text_color(theme::toolbar_fg())
                    .border_color(theme::toolbar_line())
                    .p_1()
                    .rounded_md()
                    .trigger(
                        Button::new(format!("backup-menu-{}", backup.id))
                            .ghost()
                            .text_color(theme::toolbar_fg())
                            .size(px(26.))
                            .label("⋯")
                            .disabled(disabled),
                    )
                    .content(move |_, _, cx| {
                        let state = cx.entity();
                        v_flex().w(px(168.)).gap_0p5().children(
                            [
                                ("编辑名称与备注", "app-edit", !menu_backup.snapshot),
                                ("复制备份标识", "copy-backup", false),
                                ("删除备份…", "app-delete", !menu_backup.snapshot || current),
                            ]
                            .into_iter()
                            .map(|(label, action, disabled)| {
                                let entity = entity.clone();
                                let state = state.clone();
                                let backup = menu_backup.clone();
                                Button::new(action)
                                    .custom(
                                        ButtonCustomVariant::new(cx)
                                            .foreground(theme::toolbar_fg())
                                            .hover(rgb(0x2a343f).into())
                                            .active(rgb(0x35404e).into()),
                                    )
                                    .small()
                                    .h(px(26.))
                                    .w_full()
                                    .label(label)
                                    .disabled(disabled)
                                    .on_click(move |_, window, cx| {
                                        state.update(cx, |state, cx| state.dismiss(window, cx));
                                        if action == "copy-backup" {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                backup.id.clone(),
                                            ));
                                        } else {
                                            entity.update(cx, |this, cx| {
                                                this.environment_action(
                                                    action,
                                                    Some(backup.clone()),
                                                    window,
                                                    cx,
                                                )
                                            });
                                        }
                                    })
                            }),
                        )
                    }),
            )
            .into_any_element()
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
            self.environment_action("app-save", None, window, cx);
            self.functions.text.update(cx, |input, cx| {
                input.set_value("名称=工作账号 C\n备注=备用环境", window, cx)
            });
        }
    }
}

#[cfg(test)]
mod tests {
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
    fn app_first_grid_and_record_switch_keep_target_context(cx: &mut TestAppContext) {
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
        let row = cx.debug_bounds("switch-snapshot:spare").unwrap();
        cx.simulate_click(row.center(), Modifiers::default());
        assert!(
            peer.commands
                .try_recv()
                .into_iter()
                .all(|cmd| !matches!(cmd, SessionCommand::Function(_)))
        );
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                assert_eq!(
                    view.functions.environment.backup.as_ref().unwrap().id,
                    "snapshot:spare"
                );
                view.functions.text.update(cx, |input, cx| {
                    input.set_value("名称=当前工作账号\n备注=保存后切换", window, cx)
                });
                view.execute_function(window, cx);
            })
        });
        let commands: Vec<_> =
            std::iter::from_fn(|| peer.commands.try_recv().map(unscoped).ok()).collect();
        let text = commands
            .iter()
            .find_map(|cmd| match cmd {
                SessionCommand::Function(FunctionCommand::Invoke { id, text })
                    if id == "app-switch" =>
                {
                    Some(text)
                }
                _ => None,
            })
            .unwrap();
        let operation = parse_operation(
            OPERATIONS
                .iter()
                .find(|spec| spec.id == "app-switch")
                .unwrap(),
            text,
        )
        .unwrap();
        assert_eq!(operation.params["bundle_id"], "fr.vinted");
        assert_eq!(operation.params["backup_id"], "snapshot:spare");
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("function-execute").is_none());
        assert!(cx.debug_bounds("function-input-area").is_some());
        assert!(cx.debug_bounds("function-query-loading").unwrap().bottom() <= px(400.));
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
                view.environment_action("app-save", None, window, cx);
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
            view.select_function(OPERATIONS.iter().find(|s| s.id == "app-save").unwrap(), window, cx);
            let before = view.functions.environment.records.iter().map(|r| r.id.clone()).collect::<Vec<_>>();
            view.apply_function_event(FunctionEvent::ForOperation { id:"app-list".into(), event:Box::new(FunctionEvent::State {task:32,record:serde_json::json!({
                "operation_id":"old-list","status":"completed","result":{"bundle_id":"fr.vinted","backups":[{"backup_id":"background-only"}]}
            })})}, cx);
            assert_eq!(view.functions.environment.records.iter().map(|r| r.id.clone()).collect::<Vec<_>>(), before);
            assert_eq!(view.functions.selected.unwrap().id,"app-save");
            assert!(view.functions.environment.open);
        }));
        assert!(peer.commands.try_recv().is_err());
    }
    #[gpui::test]
    fn completed_save_refreshes_list_but_unknown_never_replays(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        fixture(&view, cx);
        while peer.commands.try_recv().map(unscoped).is_ok() {}
        cx.update(|window,cx|view.update(cx,|view,cx| {
            view.environment_action("app-save",None,window,cx);
            view.functions.busy=true;
            view.apply_function_event(FunctionEvent::State {task:32,record:serde_json::json!({"operation_id":"save-1","status":"completed","result":{}})},cx);
        }));
        assert!(
            matches!(peer.commands.try_recv().map(unscoped),Ok(SessionCommand::Function(FunctionCommand::Invoke{id,..})) if id=="app-list")
        );
        view.update(cx,|view,cx| {
            view.functions.selected=OPERATIONS.iter().find(|spec|spec.id=="app-save");
            view.apply_function_event(FunctionEvent::State {task:32,record:serde_json::json!({"operation_id":"save-2","status":"completed","error":{"domain":"TVAppStateServices","code":3}})},cx);
            assert!(view.functions.uncertain);
            assert!(view.functions.operation.is_some());
        });
        assert!(peer.commands.try_recv().map(unscoped).is_err());
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
    fn pending_restore_blocks_save_and_allows_explicit_recovery(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        fixture(&view, cx);
        while peer.commands.try_recv().map(unscoped).is_ok() {}
        cx.update(|window,cx| view.update(cx, |view,cx| {
            view.functions.selected = OPERATIONS.iter().find(|spec|spec.id=="app-list");
            view.apply_function_event(FunctionEvent::State {task:32,record:serde_json::json!({"operation_id":"pending-list","status":"completed","result":{"bundle_id":"fr.vinted","backups":[],"current_backup_id":"snapshot:current","restore_pending":true}})},cx);
            assert!(view.functions.environment.restore_pending);
            view.environment_action("app-save", None, window, cx);
            assert!(view.functions.selected.is_none());
            view.environment_action("app-status", None, window, cx);
            assert_eq!(view.functions.selected.unwrap().id,"app-status");
        }));
        assert!(matches!(peer.commands.try_recv().map(unscoped).unwrap(),
            SessionCommand::Function(FunctionCommand::Invoke { id, text })
            if id == "app-status" && text == "App=fr.vinted"));
        assert!(peer.commands.try_recv().map(unscoped).is_err());
    }
    #[test]
    fn record_identifiers_are_injected_and_cannot_be_overridden() {
        let spec = OPERATIONS
            .iter()
            .find(|spec| spec.id == "app-switch")
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
