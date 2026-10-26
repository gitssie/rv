//! Apple ID registry browser backed by SQLite and the standard DataTable.
use super::*;
use chrono::{DateTime, Local};
use gpui::prelude::FluentBuilder as _;
use gpui_component::{
    input::{Input, InputEvent, InputState},
    switch::Switch,
    table::{Column, DataTable, TableDelegate, TableState},
};
use rv_core::workspace::{AppleAccount, AppleAccountStatus, AppleInitStatus, WorkspaceStore};

pub(super) struct AppleAccountsView {
    query: Entity<InputState>,
    table: Entity<TableState<AppleAccountsTable>>,
    error: Option<String>,
}
pub(super) struct AppleAccountsTable {
    rows: Vec<AppleAccount>,
    columns: Vec<Column>,
    show_passwords: bool,
}
impl AppleAccountsView {
    pub fn new(window: &mut Window, cx: &mut Context<AddressBookApp>) -> Self {
        let query = cx
            .new(|cx| InputState::new(window, cx).placeholder("搜索账号、设备、状态、参数或备注"));
        let table = cx.new(|cx| {
            TableState::new(
                AppleAccountsTable {
                    rows: vec![],
                    show_passwords: false,
                    columns: [
                        ("apple_id", "Apple ID", 250.),
                        ("password", "当前密码", 150.),
                        ("original_password", "原密码", 150.),
                        ("pending_password", "待用新密码", 150.),
                        ("verification_code_url", "取码 URL", 300.),
                        ("device_id", "关联设备 ID", 250.),
                        ("account_status", "账号状态", 110.),
                        ("init_status", "初始化", 110.),
                        ("created_at", "创建时间", 170.),
                        ("updated_at", "更新时间", 170.),
                        ("last_login_at", "最近登录", 170.),
                        ("password_changed_at", "改密时间", 170.),
                        ("params", "参数", 220.),
                        ("note", "备注", 220.),
                        ("last_error", "最近错误", 260.),
                    ]
                    .into_iter()
                    .map(|(id, name, width)| Column::new(id, name).width(px(width)))
                    .collect(),
                },
                window,
                cx,
            )
        });
        Self {
            query,
            table,
            error: None,
        }
    }
}
fn timestamp(value: Option<i64>) -> String {
    value
        .and_then(DateTime::from_timestamp_millis)
        .map(|v| {
            v.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_default()
}
impl TableDelegate for AppleAccountsTable {
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
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let account = &self.rows[row];
        let secret = |value: &str| {
            if value.is_empty() || self.show_passwords {
                value.to_owned()
            } else {
                "••••••••".into()
            }
        };
        let value = match col {
            0 => account.apple_id.clone(),
            1 => secret(&account.password),
            2 => secret(&account.original_password),
            3 => secret(account.pending_password.as_deref().unwrap_or_default()),
            4 => account.verification_code_url.clone().unwrap_or_default(),
            5 => account.device_id.clone().unwrap_or_default(),
            6 => match account.account_status {
                AppleAccountStatus::Unknown => "未确认",
                AppleAccountStatus::Active => "正常",
                AppleAccountStatus::Locked => "已锁定",
                AppleAccountStatus::Disabled => "已停用",
            }
            .into(),
            7 => match account.init_status {
                AppleInitStatus::Pending => "待初始化",
                AppleInitStatus::ChangingPassword => "修改密码中",
                AppleInitStatus::Ready => "已完成",
                AppleInitStatus::Unknown => "待确认",
            }
            .into(),
            8 => timestamp(Some(account.created_at)),
            9 => timestamp(Some(account.updated_at)),
            10 => timestamp(account.last_login_at),
            11 => timestamp(account.password_changed_at),
            12 => {
                let mut params = account.params.clone();
                if let Some(values) = params.as_object_mut() {
                    values.remove("_password_change_origin");
                }
                params.to_string()
            }
            13 => account.note.clone(),
            14 => account.last_error_message.clone().unwrap_or_default(),
            _ => String::new(),
        };
        div().text_sm().truncate().child(SharedString::from(value))
    }
    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        crate::ui::empty_state("没有匹配的 Apple ID", cx)
    }
}
impl AddressBookApp {
    pub(super) fn subscribe_apple_accounts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self._subscriptions.push(cx.subscribe_in(
            &self.settings.apple_accounts.query,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.load_apple_accounts(cx);
                }
            },
        ));
    }
    pub(super) fn load_apple_accounts(&mut self, cx: &mut Context<Self>) {
        let view = &mut self.settings.apple_accounts;
        let query = view.query.read(cx).value().to_string();
        let result =
            WorkspaceStore::open(self.book.paths()).and_then(|s| s.query_apple_accounts(&query));
        view.error = result.as_ref().err().map(|e| e.to_string());
        view.table.update(cx, |table, cx| {
            table.delegate_mut().rows = result.unwrap_or_default();
            table.clear_selection(cx);
            table.scroll_to_row(0, cx);
            table.refresh(cx);
            cx.notify();
        });
        cx.notify();
    }
    pub(super) fn render_apple_accounts(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = &self.settings.apple_accounts;
        let count = view.table.read(cx).delegate().rows.len();
        let show = view.table.read(cx).delegate().show_passwords;
        v_flex()
            .id("settings-apple-accounts")
            .debug_selector(|| "settings-apple-accounts".into())
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
                        Switch::new("apple-passwords-visible")
                            .label("显示密码")
                            .checked(show)
                            .on_click(cx.listener(|this, value: &bool, _, cx| {
                                this.settings.apple_accounts.table.update(cx, |s, cx| {
                                    s.delegate_mut().show_passwords = *value;
                                    cx.notify();
                                });
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("apple-accounts-refresh")
                            .outline()
                            .icon(IconName::Redo)
                            .label("刷新")
                            .on_click(cx.listener(|this, _, _, cx| this.load_apple_accounts(cx))),
                    ),
            )
            .when_some(view.error.as_ref(), |v, error| {
                v.child(gpui_component::alert::Alert::error(
                    "apple-accounts-error",
                    error.clone(),
                ))
            })
            .child(
                div()
                    .w_full()
                    .h(px(480.))
                    .child(DataTable::new(&view.table).bordered(true).stripe(true)),
            )
            .child(crate::typography::caption(format!("共 {count} 条记录"), cx))
            .into_any_element()
    }
}
