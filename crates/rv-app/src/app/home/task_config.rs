//! Per-shop task configuration. Execution and results stay in the backend/tasks list.
use super::*;
use crate::assets::FunctionIcon;
use gpui::prelude::FluentBuilder as _;
use gpui_component::{
    Disableable, Icon, IconName, StyledExt,
    button::{Button, ButtonVariants},
    form::{Field, Form},
    h_flex,
    input::{Input, InputState},
    v_flex,
};

pub(in crate::app) struct TaskConfigView {
    pub rows: Vec<AppTask>,
    pub editing: Option<String>,
    pub cron: Entity<InputState>,
}
impl TaskConfigView {
    pub fn new(window: &mut Window, cx: &mut Context<AddressBookApp>) -> Self {
        Self {
            rows: vec![],
            editing: None,
            cron: cx.new(|cx| InputState::new(window, cx).placeholder("秒 分 时 日 月 星期 [年]")),
        }
    }
}
fn label(kind: &str) -> &'static str {
    match kind {
        "session" => "基础资料",
        "refresh" => "会话保持",
        "products" => "商品同步",
        "orders" => "订单同步",
        "chat_inbox" => "会话同步",
        "chat_threads" => "消息同步",
        "chat_send" => "消息发送",
        "chat_read" => "已读回执",
        "notification_stats" => "未读统计",
        "notifications" => "通知同步",
        _ => "任务",
    }
}
impl AddressBookApp {
    pub(in crate::app) fn task_error_message(
        &self,
        operation: &'static str,
        task_id: Option<&str>,
        error: &rv_core::StoreError,
    ) -> String {
        let app_id = self.home.editing.as_ref().map(|app| app.id.as_str());
        tracing::error!(operation,task_id,app_id,error=%error,"Task operation failed");
        match error {
            rv_core::StoreError::Invalid(message) => message.clone(),
            rv_core::StoreError::Conflict => "任务已变化，请重试".into(),
            _ => format!("{operation}失败，请查看日志"),
        }
    }

    pub(super) fn reload_shop_tasks(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.home.editing.as_ref().map(|a| a.id.clone()) else {
            return;
        };
        match WorkspaceStore::open(self.book.paths()).and_then(|s| s.configured_tasks(&id)) {
            Ok(mut rows) => {
                rows.sort_by_key(|task| match task.kind.as_str() {
                    "refresh" => 0,
                    "session" => 1,
                    "products" => 2,
                    "orders" => 3,
                    "chat_inbox" => 4,
                    "notification_stats" => 5,
                    "notifications" => 6,
                    _ => 6,
                });
                self.home.task_config.rows = rows;
            }
            Err(e) => self.set_error_status(self.task_error_message("读取任务配置", None, &e), cx),
        }
        cx.notify();
    }
    fn edit_shop_task(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task) = self.home.task_config.rows.iter().find(|t| t.id == id) else {
            return;
        };
        let value = task.cron_expression.clone();
        self.home.task_config.editing = Some(id.to_owned());
        self.home.task_config.cron.update(cx, |s, cx| {
            s.set_value(value, window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }
    fn save_shop_task(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let cron = self.home.task_config.cron.read(cx).value().to_string();
        match WorkspaceStore::open(self.book.paths()).and_then(|mut s| s.reschedule_task(id, &cron))
        {
            Ok(_) => self.home.task_config.editing = None,
            Err(e) => {
                self.set_error_status(self.task_error_message("保存任务计划", Some(id), &e), cx)
            }
        }
        self.reload_shop_tasks(cx);
    }
    fn toggle_shop_task(&mut self, id: &str, enabled: bool, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        match rv_business::tasks::set_task_enabled(self.book.paths(), id, enabled) {
            Ok(task) => self.set_status(format!("{}{}",label(&task.kind),if enabled {"已提交，等待执行"} else {"已停止"}),cx),
            Err(e) => self.set_error_status(self.task_error_message("设置任务启停", Some(id), &e), cx),
        }
        self.reload_shop_tasks(cx);
    }
    pub(super) fn open_task_execution_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings(window, cx);
        self.settings.page = Some(crate::app::settings::SettingsPage::Tasks);
        self.settings.revision = self.settings.revision.wrapping_add(1);
        self.load_task_list(cx);
        cx.notify();
    }
    pub(in crate::app) fn render_shop_task_config(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = &self.home.task_config;
        let phone_refresh = self.home.editing.as_ref().is_some_and(|app|
            WorkspaceStore::open(self.book.paths()).and_then(|s|s.token_source(&app.id)).ok()
                .is_some_and(|source|matches!(source,rv_core::workspace::TokenSource::Phone(_))));
        let mut form = Form::horizontal();
        for task in &view.rows {
            let id = task.id.clone();
            let editing = view.editing.as_deref() == Some(id.as_str());
            let mut controls = h_flex().gap_2().items_center().min_w_0();
            if editing {
                controls = controls
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&view.cron).disabled(self.read_only)),
                    )
                    .child(
                        Button::new(SharedString::from(format!("shop-task-save-{id}")))
                            .primary()
                            .icon(IconName::Check)
                            .tooltip("保存计划")
                            .accessibility_label("保存计划")
                            .disabled(self.read_only)
                            .on_click({
                                let id = id.clone();
                                cx.listener(move |this, _, _, cx| this.save_shop_task(&id, cx))
                            }),
                    )
                    .child(
                        Button::new(SharedString::from(format!("shop-task-cancel-{id}")))
                            .ghost()
                            .icon(IconName::Close)
                            .tooltip("取消编辑")
                            .accessibility_label("取消编辑")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.home.task_config.editing = None;
                                cx.notify();
                            })),
                    );
            } else {
                controls = controls
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_normal()
                            .text_color(crate::theme::muted(cx))
                            .truncate()
                            .child(task.cron_expression.clone()),
                    )
                    .child(
                        Button::new(SharedString::from(format!("shop-task-edit-{id}")))
                            .ghost()
                            .icon(FunctionIcon::Edit)
                            .tooltip("修改计划")
                            .accessibility_label("修改计划")
                            .disabled(self.read_only)
                            .on_click({
                                let id = id.clone();
                                cx.listener(move |this, _, window, cx| {
                                    this.edit_shop_task(&id, window, cx)
                                })
                            }),
                    );
            }
            if task.error_code.as_deref()==Some("access_token") && task.enabled {
                controls=controls.child(crate::typography::caption("等待 Token",cx));
            } else if task.kind=="refresh" && phone_refresh {
                controls=controls.child(crate::typography::caption("手机负责刷新",cx));
            } else if task.status=="running" {
                controls=controls.child(crate::typography::caption("执行中",cx));
            }
            let active = task.enabled
                && (task.next_run_at.is_some()
                    || task.status == "running"
                    || (task.kind == "refresh" && task.status == "unknown"));
            let retry = task.status != "running"
                && (matches!(task.status.as_str(), "failed" | "unknown")
                    || task
                        .last_run_status
                        .as_deref()
                        .is_some_and(|s| matches!(s, "failed" | "unknown")));
            controls = controls.child(
                Button::new(SharedString::from(format!("shop-task-toggle-{id}")))
                    .label(if retry {
                        "重试"
                    } else if active {
                        "停止"
                    } else {
                        "启动"
                    })
                    .disabled(
                        self.read_only
                            || (task.kind=="refresh" && phone_refresh)
                            || (retry && task.status == "running"),
                    )
                    .tooltip(if task.kind=="refresh" && phone_refresh {
                        "Token 来源为手机，由手机 App 刷新"
                    } else if retry {
                        "立即重试任务"
                    } else if active {
                        "停止任务"
                    } else {
                        "启动并立即执行"
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if retry {
                            match rv_business::tasks::run_task_now(this.book.paths(), &id) {
                                Ok(task) => this.set_status(format!("{}已提交，等待执行",label(&task.kind)),cx),
                                Err(error) => this.set_error_status(this.task_error_message("重试任务",Some(&id),&error),cx),
                            }
                            this.reload_shop_tasks(cx);
                        } else {
                            this.toggle_shop_task(&id, !active, cx);
                        }
                    })),
            );
            form = form.child(
                Field::new()
                    .label_fn({
                        let name = label(&task.kind);
                        let icon = match task.kind.as_str() {
                            "products" => FunctionIcon::Products,
                            "orders" => FunctionIcon::Orders,
                            "chat_inbox" | "chat_threads" | "chat_send" | "chat_read" => {
                                FunctionIcon::Chat
                            }
                            "notification_stats" | "notifications" => FunctionIcon::Mail,
                            "refresh" => FunctionIcon::Shield,
                            _ => FunctionIcon::User,
                        };
                        move |_, cx| {
                            h_flex()
                                .gap_2()
                                .items_center()
                                .font_normal()
                                .child(Icon::new(icon.clone()))
                                .child(crate::typography::label(name, cx))
                        }
                    })
                    .items_center()
                    .child(controls),
            );
        }
        v_flex()
            .id("shop-task-config")
            .debug_selector(|| "shop-task-config".into())
            .gap_2()
            .when(view.rows.is_empty(), |v| v.child(crate::ui::empty_icon(cx)))
            .child(form)
            .into_any_element()
    }
}
