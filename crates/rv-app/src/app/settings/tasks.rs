//! Task controls read/write tasks only; no business HTTP requests originate here.
use super::*;
use chrono::{DateTime, Local};
use gpui::prelude::FluentBuilder as _;
use gpui_component::{
    Disableable, Sizable, StyledExt, WindowExt,
    description_list::{DescriptionItem, DescriptionList},
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    table::{Column, DataTable, TableDelegate, TableState},
    tooltip::Tooltip,
};
use rv_core::workspace::{TaskEntry, WorkspaceStore};

pub(super) struct TasksView {
    query: Entity<InputState>,
    cron: Entity<InputState>,
    editing: Option<String>,
    table: Entity<TableState<TasksTable>>,
}
pub(super) struct TasksTable {
    rows: Vec<TaskEntry>,
    columns: Vec<Column>,
    owner: WeakEntity<AddressBookApp>,
    read_only: bool,
}
impl TasksView {
    pub fn new(window: &mut Window, cx: &mut Context<AddressBookApp>) -> Self {
        let owner = cx.entity().downgrade();
        Self {
            query: cx.new(|cx| {
                InputState::new(window, cx).placeholder("搜索店铺、任务类型、状态或错误")
            }),
            cron: cx.new(|cx| InputState::new(window, cx).placeholder("秒 分 时 日 月 星期 [年]")),
            editing: None,
            table: cx.new(|cx| {
                TableState::new(
                    TasksTable {
                        rows: vec![],
                        owner,
                        read_only: false,
                        columns: [
                            ("task", "任务 / 店铺", 220.),
                            ("actions", "操作", 110.),
                            ("status", "状态", 90.),
                            ("reason", "错误 / 等待原因", 230.),
                            ("next", "下次执行", 148.),
                        ]
                        .into_iter()
                        .map(|(id, title, width)| {
                            let column = Column::new(id, title).width(px(width)).movable(false);
                            if matches!(id, "task" | "actions") {
                                column.fixed_left().resizable(false)
                            } else {
                                column
                            }
                        })
                        .collect(),
                    },
                    window,
                    cx,
                )
            }),
        }
    }
}
fn time(value: Option<i64>) -> String {
    value
        .and_then(|v| DateTime::from_timestamp(v, 0))
        .map(|v| {
            v.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_default()
}
fn status(value: &str) -> &str {
    match value {
        "pending" => "待执行",
        "running" => "执行中",
        "completed" => "成功",
        "failed" => "失败",
        "unknown" => "结果待确认",
        "cancelled" => "已取消",
        other => other,
    }
}
fn kind(value: &str) -> &str {
    match value {
        "session" => "店铺基础资料",
        "products" => "商品同步",
        "orders" => "订单同步",
        "chat_inbox" | "inbox" => "会话同步",
        "chat_threads" => "消息同步",
        "notification_stats" => "未读统计",
        "notifications" => "通知同步",
        "set_avatar" => "设置头像",
        "refresh" => "刷新凭据",
        "chat_thread" => "会话内容",
        "chat_send" => "发送消息",
        "chat_read" => "标记已读",
        other => other,
    }
}
fn task_name<'a>(kind_value: &'a str, key: Option<&str>) -> &'a str {
    if key == Some(rv_business::tasks::KEEPALIVE_KEY) {
        "会话保持"
    } else {
        kind(kind_value)
    }
}
fn dependency_blocked(entry: &TaskEntry) -> bool {
    entry.task.enabled
        && entry.task.status == "pending"
        && entry
            .dependency_status
            .as_deref()
            .is_some_and(|s| s != "completed")
}
fn waiting_reason(entry: &TaskEntry) -> String {
    if let Some(message) = entry.task.error_message.as_ref().filter(|s| !s.is_empty()) {
        return message.clone();
    }
    if dependency_blocked(entry) {
        let name = if entry.dependency_kind.as_deref() == Some("refresh") {
            "会话保持"
        } else {
            kind(entry.dependency_kind.as_deref().unwrap_or("前置任务"))
        };
        return format!(
            "等待{name}成功（{}）",
            status(entry.dependency_status.as_deref().unwrap_or("pending"))
        );
    }
    String::new()
}
fn primary_action(entry: &TaskEntry) -> (usize, &'static str) {
    let task = &entry.task;
    if task.status == "running" {
        return (1, if task.enabled { "停止" } else { "停止中" });
    }
    if matches!(task.status.as_str(), "failed" | "unknown")
        || task
            .last_run_status
            .as_deref()
            .is_some_and(|s| matches!(s, "failed" | "unknown"))
    {
        return (0, "重试");
    }
    if task.enabled && (task.next_run_at.is_some() || task.status == "running") {
        (1, "停止")
    } else {
        (1, "启动")
    }
}
fn action_blocker(entry: &TaskEntry, action: usize, read_only: bool) -> Option<String> {
    if action == 3 {
        return None;
    }
    if read_only {
        return Some("当前为只读模式".into());
    }
    if entry.task.status == "running" && (action == 0 || (action == 1 && !entry.task.enabled)) {
        return Some("本轮任务尚未结束".into());
    }
    None
}
impl TableDelegate for TasksTable {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }
    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }
    fn column(&self, index: usize, _: &App) -> Column {
        self.columns[index].clone()
    }
    fn render_td(
        &mut self,
        row: usize,
        col: usize,
        _: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let entry = &self.rows[row];
        let task = &entry.task;
        let key = self.columns[col].key.as_ref();
        if key == "actions" {
            let (action, label) = primary_action(entry);
            let reason = action_blocker(entry, action, self.read_only);
            let owner = self.owner.clone();
            let id = task.id.clone();
            let selector = format!("task-primary-{id}");
            let button = Button::new(SharedString::from(selector.clone()))
                .debug_selector(move || selector.clone())
                .small()
                .label(label)
                .tooltip(reason.clone().unwrap_or_else(|| label.to_string()))
                .disabled(reason.is_some())
                .on_click(move |_, window, cx| {
                    if let Some(owner) = owner.upgrade() {
                        owner.update(cx, |this, cx| {
                            this.task_list_action(&id, action, window, cx)
                        });
                    }
                });
            let owner = self.owner.clone();
            let entry = entry.clone();
            let read_only = self.read_only;
            let more = Button::new(SharedString::from(format!("task-more-{}", task.id)))
                .ghost()
                .small()
                .icon(IconName::Ellipsis)
                .tooltip("更多操作")
                .dropdown_menu(move |mut menu, _, _| {
                    for (index, title) in [
                        (0, "立即执行"),
                        (
                            1,
                            if entry.task.enabled {
                                "停止"
                            } else {
                                "启动"
                            },
                        ),
                        (2, "修改计划"),
                        (3, "详情"),
                    ] {
                        if index == action || (index == 2 && entry.task.kind == "set_avatar") {
                            continue;
                        }
                        let owner = owner.clone();
                        let id = entry.task.id.clone();
                        menu = menu.item(
                            PopupMenuItem::new(title)
                                .disabled(action_blocker(&entry, index, read_only).is_some())
                                .on_click(move |_, window, cx| {
                                    if let Some(owner) = owner.upgrade() {
                                        owner.update(cx, |this, cx| {
                                            this.task_list_action(&id, index, window, cx)
                                        });
                                    }
                                }),
                        );
                    }
                    menu
                });
            return h_flex()
                .gap_1()
                .items_center()
                .child(button)
                .child(more)
                .into_any_element();
        }
        if key == "task" {
            let title = format!(
                "{} · {}",
                task_name(&task.kind, task.task_key.as_deref()),
                entry.app_title
            );
            return div()
                .id((SharedString::from("task-title"), row))
                .text_sm()
                .font_normal()
                .truncate()
                .child(title.clone())
                .tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx))
                .into_any_element();
        }
        let value = match key {
            "status" if !task.enabled => "已停止".to_string(),
            "status" if task.error_code.as_deref()==Some("access_token") => "等待 Token".to_string(),
            "status" if task.error_code.as_deref()==Some("token_source") => "手机提供 Token".to_string(),
            "status" if dependency_blocked(entry) => "等待前置".to_string(),
            "status"
                if task.status == "pending"
                    && task
                        .error_code
                        .as_deref()
                        .is_some_and(|c| matches!(c, "credentials" | "proxy" | "keepalive")) =>
            {
                "等待条件".to_string()
            }
            "status" => status(&task.status).to_owned(),
            "reason" => waiting_reason(entry),
            "next" => time(task.next_run_at),
            _ => String::new(),
        };
        let tooltip = value.clone();
        div()
            .id((SharedString::from("task-cell"), row * 5 + col))
            .text_sm()
            .font_normal()
            .truncate()
            .child(SharedString::from(value))
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .into_any_element()
    }
    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        crate::ui::empty_state("没有匹配的任务", cx)
    }
}
impl AddressBookApp {
    pub(super) fn subscribe_task_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self._subscriptions.push(cx.subscribe_in(
            &self.settings.tasks.query,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.load_task_list(cx);
                }
            },
        ));
    }
    pub(in crate::app) fn load_task_list(&mut self, cx: &mut Context<Self>) {
        let query = self.settings.tasks.query.read(cx).value().to_string();
        let query = match query.trim() {
            "待执行" => "pending",
            "执行中" => "running",
            "成功" => "completed",
            "失败" => "failed",
            "已取消" | "已停止" => "cancelled",
            "结果待确认" => "unknown",
            "商品" | "商品同步" => "products",
            "订单" | "订单同步" => "orders",
            "店铺基础资料" => "session",
            "会话同步" => "chat_inbox",
            "会话保持" => "refresh",
            "未读统计" => "notification_stats",
            "通知" | "通知同步" => "notifications",
            "头像" | "设置头像" => "set_avatar",
            other => other,
        };
        let result = WorkspaceStore::open(self.book.paths()).and_then(|store| {
            let mut rows = store.query_tasks(query)?;
            let mut titles = std::collections::HashMap::new();
            for entry in &mut rows {
                let id = rv_core::workspace::AppId::from_idfv(&entry.task.app_id)?;
                if !titles.contains_key(&id) {
                    let app = store.load(&id)?;
                    titles.insert(
                        id.clone(),
                        self.home
                            .plugins
                            .for_bundle(&app.bundle_id)
                            .display_title(&app),
                    );
                }
                entry.app_title = titles[&id].clone();
            }
            Ok(rows)
        });
        let message = result
            .as_ref()
            .err()
            .map(|error| self.task_error_message("读取任务列表", None, error));
        if let Some(message) = message {
            self.set_error_status(message, cx);
        }
        let view = &mut self.settings.tasks;
        let read_only = self.read_only;
        view.table.update(cx, |table, cx| {
            table.delegate_mut().rows = result.unwrap_or_default();
            table.delegate_mut().read_only = read_only;
            table.refresh(cx);
            cx.notify();
        });
        cx.notify();
    }
    fn task_list_action(
        &mut self,
        id: &str,
        action: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if action == 3 {
            self.show_task_details(id, window, cx);
            return;
        }
        if self.read_only {
            return;
        }
        let result = WorkspaceStore::open(self.book.paths()).and_then(|store| match action {
            0 => rv_business::tasks::run_task_now(self.book.paths(), id).map(|_| ()),
            1 => {
                let task = store.task(id)?;
                let active = task.enabled
                    && (task.next_run_at.is_some()
                        || task.status == "running"
                        || (task.kind == "refresh" && task.status == "unknown"));
                rv_business::tasks::set_task_enabled(self.book.paths(), id, !active).map(|_| ())
            }
            _ => store.task(id).and_then(|task| {
                if task.kind == "set_avatar" {
                    return Err(rv_core::StoreError::Invalid("设置头像为一次性任务".into()));
                }
                self.settings.tasks.editing = Some(task.id);
                self.settings.tasks.cron.update(cx, |input, cx| {
                    input.set_value(task.cron_expression, window, cx)
                });
                Ok(())
            }),
        });
        self.load_task_list(cx);
        if let Err(error) = result {
            self.set_error_status(self.task_error_message("操作任务", Some(id), &error), cx);
        }
        cx.notify();
    }
    fn show_task_details(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self
            .settings
            .tasks
            .table
            .read(cx)
            .delegate()
            .rows
            .iter()
            .find(|e| e.task.id == id)
            .cloned()
        else {
            return;
        };
        let task = &entry.task;
        let fields = vec![
            ("店铺", entry.app_title.clone()),
            (
                "任务",
                task_name(&task.kind, task.task_key.as_deref()).to_string(),
            ),
            (
                "状态",
                if !task.enabled {
                    "已停止".into()
                } else {
                    status(&task.status).into()
                },
            ),
            ("Cron", task.cron_expression.clone()),
            ("下次执行", time(task.next_run_at)),
            ("最近开始", time(task.started_at)),
            ("最近结束", time(task.finished_at)),
            (
                "最近结果",
                task.last_run_status
                    .as_deref()
                    .map(status)
                    .unwrap_or_default()
                    .to_string(),
            ),
            (
                "前置任务",
                entry
                    .dependency_kind
                    .as_deref()
                    .map(|v| {
                        format!(
                            "{} · {}",
                            if v == "refresh" {
                                "会话保持"
                            } else {
                                kind(v)
                            },
                            status(entry.dependency_status.as_deref().unwrap_or("pending"))
                        )
                    })
                    .unwrap_or_default(),
            ),
            ("错误 / 等待原因", waiting_reason(&entry)),
        ];
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("任务详情")
                .child(
                    DescriptionList::horizontal()
                        .columns(1)
                        .children(fields.iter().map(|(label, value)| {
                            DescriptionItem::new(*label).value(if value.is_empty() {
                                "—".to_string()
                            } else {
                                value.clone()
                            })
                        })),
                )
        });
    }
    fn save_task_cron(&mut self, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let Some(id) = self.settings.tasks.editing.clone() else {
            return;
        };
        let cron = self.settings.tasks.cron.read(cx).value().to_string();
        let result = WorkspaceStore::open(self.book.paths())
            .and_then(|mut store| store.reschedule_task(&id, &cron));
        self.load_task_list(cx);
        match result {
            Ok(_) => self.settings.tasks.editing = None,
            Err(e) => {
                self.set_error_status(self.task_error_message("保存任务计划", Some(&id), &e), cx)
            }
        };
        cx.notify();
    }
    pub(super) fn render_task_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = &self.settings.tasks;
        v_flex()
            .id("settings-tasks")
            .gap_4()
            .w_full()
            .min_w_0()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&view.query)
                                .prefix(IconName::Search)
                                .cleanable(true),
                        ),
                    )
                    .child(
                        Button::new("tasks-refresh")
                            .outline()
                            .icon(IconName::Redo)
                            .label("刷新")
                            .on_click(cx.listener(|this, _, _, cx| this.load_task_list(cx))),
                    ),
            )
            .when(view.editing.is_some(), |v| {
                v.child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(div().flex_1().min_w_0().child(Input::new(&view.cron)))
                        .child(
                            Button::new("task-cron-save")
                                .primary()
                                .label("保存计划")
                                .disabled(self.read_only)
                                .on_click(cx.listener(|this, _, _, cx| this.save_task_cron(cx))),
                        )
                        .child(
                            Button::new("task-cron-cancel")
                                .ghost()
                                .label("取消编辑")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.settings.tasks.editing = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .child(
                div()
                    .w_full()
                    .h(px(480.))
                    .child(DataTable::new(&view.table).bordered(true).stripe(true)),
            )
            .child(crate::typography::caption(
                format!(
                    "共 {} 条任务，最多显示最近 500 条",
                    view.table.read(cx).delegate().rows.len()
                ),
                cx,
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{SettingsPage, WorkspaceStore, action_blocker, dependency_blocked, waiting_reason};
    use crate::app::ui_tests::setup;
    use gpui::TestAppContext;
    use gpui::px;
    #[gpui::test]
    fn task_actions_stay_visible_when_narrow_table_scrolls(cx: &mut TestAppContext) {
        let device = rv_core::Connection::new("fixture", "example.invalid", 5900);
        let (app, cx, _dir) = setup(cx, vec![device.clone()]);
        cx.update(|window,cx|app.update(cx,|app,cx| {
            app.home.task_worker.take();
            crate::app::chat::test_support::seed(app.book.paths(),device.id);
            WorkspaceStore::open(app.book.paths()).unwrap().business_transaction(|db| {
                db.execute("UPDATE tasks SET status='unknown',last_run_status='unknown',next_run_at=NULL,error_message='业务请求中断，结果未知' WHERE kind='refresh'",[])?;Ok(())
            }).unwrap();
            app.open_settings(window,cx);
            app.settings.page=Some(SettingsPage::Tasks);app.settings.revision+=1;app.load_task_list(cx);
        }));
        cx.simulate_resize(gpui::size(px(800.), px(620.)));
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let id = app.read_with(cx, |app, cx| {
            let table = app.settings.tasks.table.read(cx);
            let rows = &table.delegate().rows;
            let blocked = rows
                .iter()
                .find(|entry| entry.dependency_kind.as_deref() == Some("refresh"))
                .unwrap();
            assert!(dependency_blocked(blocked));
            assert!(waiting_reason(blocked).contains("结果待确认"));
            let unknown = rows
                .iter()
                .find(|entry| entry.task.kind == "refresh")
                .unwrap();
            assert!(action_blocker(unknown, 0, false).is_none());
            assert_eq!(super::primary_action(unknown), (0, "重试"));
            rows[0].task.id.clone()
        });
        let selector: &'static str = Box::leak(format!("task-primary-{id}").into_boxed_str());
        let before = cx
            .debug_bounds(selector)
            .expect("details action visible initially");
        app.update(cx, |app, cx| {
            app.settings
                .tasks
                .table
                .update(cx, |table, cx| table.scroll_to_col(4, cx))
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let after = cx
            .debug_bounds(selector)
            .expect("details action remains visible after horizontal scroll");
        assert_eq!(before.origin.x, after.origin.x);
        // Explicit retry is available without a separate recovery flow.
        let unknown_id = app.read_with(cx, |app, cx| {
            app.settings
                .tasks
                .table
                .read(cx)
                .delegate()
                .rows
                .iter()
                .find(|entry| entry.task.kind == "refresh")
                .unwrap()
                .task
                .id
                .clone()
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.task_list_action(&unknown_id, 0, window, cx);
                let task = WorkspaceStore::open(app.book.paths())
                    .unwrap()
                    .task(&unknown_id)
                    .unwrap();
                assert_eq!(task.status, "pending");
                assert!(task.enabled);
                assert!(task.next_run_at.unwrap() <= crate::app::unix_now());
            })
        });
    }
}
