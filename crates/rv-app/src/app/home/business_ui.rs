//! Shared host data controller; Bundle renderers own the row presentation.
use super::*;
use crate::{assets::FunctionIcon, theme};
use gpui::prelude::FluentBuilder as _;
use gpui_component::{
    Disableable, Icon,
    badge::Badge,
    button::{Button, ButtonVariants},
    h_flex,
    tab::{Tab, TabBar},
    v_flex,
};
use rv_business::data::{BusinessPage, Dataset};
use rv_core::events::{ChangeDomain, EntityChange, WorkspaceChanged};
pub(in crate::app) struct BusinessView {
    pub dataset: Option<Dataset>,
    pub show_tasks: bool,
    pub page: BusinessPage,
    pub counts: [usize; 2],
    pub loading: bool,
    pub error: Option<String>,
    pub sequence: u64,
}
impl BusinessView {
    pub fn new(_window: &mut Window, _cx: &mut Context<AddressBookApp>) -> Self {
        Self {
            dataset: None,
            show_tasks: false,
            page: Default::default(),
            counts: [0; 2],
            loading: false,
            error: None,
            sequence: 0,
        }
    }
}
impl AddressBookApp {
    pub(in crate::app) fn render_shop_operations(&self, cx: &mut Context<Self>) -> AnyElement {
        use gpui_component::{
            IconName,
            menu::{DropdownMenu, PopupMenuItem},
        };
        let Some(app) = &self.home.editing else {
            return div().into_any_element();
        };
        let id = app.id.clone();
        let bundle = app.bundle_id.clone();
        let host_owned = self.execution_mode(&id) == ExecutionMode::Rv;
        let operations = self.home.plugins.for_bundle(&bundle).offline_operations();
        let links = self.home.links.get(&id).cloned().unwrap_or_default();
        let mut toolbar = h_flex()
            .id("shop-operations")
            .debug_selector(|| "shop-operations".into())
            .gap_1()
            .items_center();
        for &operation in operations {
            let icon = match operation {
                rv_business::Operation::Session => Icon::new(FunctionIcon::Refresh),
                rv_business::Operation::Inbox => Icon::new(FunctionIcon::Chat),
                rv_business::Operation::NotificationStats => Icon::new(IconName::Bell),
                rv_business::Operation::Notifications => Icon::new(IconName::Bell),
                rv_business::Operation::Refresh => Icon::new(FunctionIcon::Refresh),
                rv_business::Operation::Products => Icon::new(FunctionIcon::Products),
                rv_business::Operation::Orders => Icon::new(FunctionIcon::Orders),
            };
            let id = id.clone();
            toolbar = toolbar.child(
                Button::new(SharedString::from(format!(
                    "shop-action-{}",
                    operation.key()
                )))
                .ghost()
                .icon(icon)
                .tooltip(if operation == rv_business::Operation::Inbox {
                    "消息"
                } else {
                    operation.label()
                })
                .accessibility_label(operation.label())
                .loading(self.home.manual_syncs.contains(&(id.clone(),operation)))
                .disabled(
                    (self.read_only && operation != rv_business::Operation::Inbox)
                        || self.home.manual_syncs.contains(&(id.clone(),operation)),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    if operation == rv_business::Operation::Inbox {
                        this.open_chat_for(id.clone(), window, cx);
                    } else {
                        this.run_shop_sync(id.clone(), operation, cx);
                    }
                })),
            );
        }
        let source_online = links
            .iter()
            .any(|link| self.phone_online(link.device_id, cx));
        let read = Button::new("shop-read-profile")
            .ghost()
            .icon(IconName::ArrowDown)
            .tooltip(if source_online {
                "从手机同步账号备份"
            } else {
                "请先连接来源手机"
            })
            .accessibility_label("同步账号备份")
            .loading(self.home.backup_syncing.contains_key(&id))
            .disabled(
                self.read_only
                    || !host_owned
                    || !source_online
                    || self.home.backup_syncing.contains_key(&id),
            );
        let read: AnyElement = if let [link] = links.as_slice() {
            let link = link.clone();
            read.on_click(cx.listener(move |this, _, window, cx| {
                this.confirm_backup_profile_sync(
                    id.clone(),
                    link.clone(),
                    bundle.clone(),
                    window,
                    cx,
                )
            }))
            .into_any_element()
        } else if links.len() > 1 {
            let book = cx.entity();
            let online: std::collections::HashSet<_> = links
                .iter()
                .filter(|link| self.phone_online(link.device_id, cx))
                .map(|link| link.device_id)
                .collect();
            let source_titles = self.backup_source_labels(&links);
            read.dropdown_menu(move |mut menu, _, _| {
                for (link, source_title) in links.iter().zip(&source_titles) {
                    let book = book.clone();
                    let id = id.clone();
                    let bundle = bundle.clone();
                    let link = link.clone();
                    menu = menu.item(
                        PopupMenuItem::new(source_title.clone())
                            .disabled(!online.contains(&link.device_id))
                            .on_click(move |_, window, cx| {
                                book.update(cx, |this, cx| {
                                    this.confirm_backup_profile_sync(
                                        id.clone(),
                                        link.clone(),
                                        bundle.clone(),
                                        window,
                                        cx,
                                    )
                                })
                            }),
                    );
                }
                menu
            })
            .into_any_element()
        } else {
            read.into_any_element()
        };
        toolbar
            .child(self.render_token_source(cx))
            .child(self.render_set_avatar_button(cx))
            .child(read)
            .child(self.render_phone_handoff_button(cx))
            .into_any_element()
    }
    pub(super) fn subscribe_business(&mut self, cx: &mut Context<Self>) {
        let bus = match rv_core::events::workspace_bus(self.book.paths()) {
            Ok(bus) => bus,
            Err(error) => {
                tracing::error!(%error, "Workspace EventBus unavailable");
                return;
            }
        };
        // Register before starting writers. The subscription owns its registration
        // and is removed automatically when the view's async task ends.
        let events = bus.subscribe::<WorkspaceChanged>();
        let tokens = bus.subscribe::<rv_core::events::PhoneTokenResult>();
        self.home.token_updates = Some(cx.spawn(async move |this,cx| {
            while let Ok(event) = tokens.recv().await {
                if !event.manual { continue; }
                let event=event.clone();
                if this.update(cx,|this,cx| match &event.result {
                    Ok(())=>this.set_status("已获取手机当前 Token",cx),
                    Err(error)=>this.set_error_status(error.clone(),cx),
                }).is_err(){break;}
            }
        }));
        if !self.read_only {
            self.home.task_worker = Some(rv_business::tasks::TaskWorker::spawn(
                self.book.paths().clone(),
            ));
        }
        self.home.change_subscription = Some(cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                let mut changes: std::collections::BTreeSet<_> =
                    event.changes.iter().cloned().collect();
                while let Ok(event) = events.try_recv() {
                    changes.extend(event.changes.iter().cloned());
                }
                if this
                    .update(cx, |this, cx| this.apply_workspace_changes(&changes, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
    }
    fn apply_workspace_changes(
        &mut self,
        changes: &std::collections::BTreeSet<EntityChange>,
        cx: &mut Context<Self>,
    ) {
        use ChangeDomain::*;
        let has = |domains: &[ChangeDomain]| changes.iter().any(|c| domains.contains(&c.domain));
        let editing = self.home.editing.as_ref().map(|app| app.id.clone());
        let shop_changed = |domains: &[ChangeDomain]| {
            changes.iter().any(|c| {
                domains.contains(&c.domain)
                    && c.app_id
                        .as_deref()
                        .is_none_or(|id| editing.as_ref().is_some_and(|app| app.as_str() == id))
            })
        };
        if has(&[Apps, Credentials, Execution, Products]) {
            self.reload_home(cx);
        }
        if shop_changed(&[Apps, Credentials, Execution]) {
            if let Some(id) = &editing {
                self.refresh_editing_baseline(id);
            }
        }
        let scopes = [
            has(&[Apps, Credentials, Execution]),
            has(&[Apps, Conversations]),
            has(&[Apps, Notifications]),
        ];
        if scopes.iter().any(|v| *v) {
            self.reload_chat_data(scopes, false, cx);
        }
        if self.chat.window.is_some()
            && changes.iter().any(|c| {
                matches!(c.domain, Messages | Customers | Conversations)
                    && c.conversation_id
                        .as_ref()
                        .is_none_or(|id| self.chat.selected.as_ref() == Some(id))
                    && c.app_id.as_deref().is_none_or(|id| {
                        self.chat
                            .account
                            .as_ref()
                            .is_some_and(|app| app.as_str() == id)
                    })
            })
        {
            self.reload_chat_thread(cx);
        }
        if matches!(self.modal, Modal::Shop) {
            if shop_changed(&[Products, Orders]) {
                self.reload_business_data(cx);
            }
            if self.home.business.show_tasks && shop_changed(&[Tasks, Execution]) {
                self.reload_shop_tasks(cx);
            }
        }
        if has(&[Tasks]) && self.settings.page == Some(crate::app::settings::SettingsPage::Tasks) {
            self.load_task_list(cx);
        }
    }
    pub(super) fn reset_business_view(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.home.business.sequence += 1;
        self.home.business.show_tasks = false;
        self.home.task_config.editing = None;
        self.home.task_config.rows.clear();
        self.home.business.dataset = self.home.editing.as_ref().and_then(|app| {
            self.home
                .plugins
                .for_bundle(&app.bundle_id)
                .datasets()
                .first()
                .copied()
        });
        self.home.business.page = Default::default();
        self.home.business.counts = [0; 2];
        self.reload_business_data(cx);
    }
    pub(super) fn reload_business_data(&mut self, cx: &mut Context<Self>) {
        let (Some(app), Some(dataset)) = (self.home.editing.as_ref(), self.home.business.dataset)
        else {
            return;
        };
        let id = app.id.clone();
        let paths = self.book.paths().clone();
        self.home.business.sequence += 1;
        let sequence = self.home.business.sequence;
        self.home.business.loading = true;
        self.home.business.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut page = rv_business::data::read_all(&paths, &id, dataset)?;
                    rv_business::assets::local(&paths, &mut page.rows);
                    let other = if dataset == Dataset::Products {
                        Dataset::Orders
                    } else {
                        Dataset::Products
                    };
                    let other_count =
                        rv_business::data::read_data(&paths, &id, other, "", 0)?.total;
                    let counts = if dataset == Dataset::Products {
                        [page.total, other_count]
                    } else {
                        [other_count, page.total]
                    };
                    Ok::<_, String>((page, counts))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.home.business.sequence != sequence || this.home.editing.is_none() {
                    return;
                }
                this.home.business.loading = false;
                match result {
                    Ok((page, counts)) => {
                        this.home.business.counts = counts;
                        this.home.business.page = page;
                    }
                    Err(error) => this.home.business.error = Some(error),
                };
                cx.notify();
            });
        })
        .detach();
    }
    fn select_business_dataset(&mut self, dataset: Dataset, cx: &mut Context<Self>) {
        self.home.business.show_tasks = false;
        self.home.business.dataset = Some(dataset);
        self.home.business.page = Default::default();
        self.reload_business_data(cx);
    }
    pub(in crate::app) fn render_business_data(&self, cx: &mut Context<Self>) -> AnyElement {
        let (Some(app), Some(dataset)) = (self.home.editing.as_ref(), self.home.business.dataset)
        else {
            return div().into_any_element();
        };
        let view = &self.home.business;
        let plugin = self.home.plugins.for_bundle(&app.bundle_id);
        let tabs = TabBar::new("shop-datasets")
            .underline()
            .selected_index(if view.show_tasks {
                2
            } else if dataset == Dataset::Products {
                0
            } else {
                1
            })
            .on_click(cx.listener(|this, index: &usize, _, cx| {
                if *index == 2 {
                    this.home.business.show_tasks = true;
                    this.reload_shop_tasks(cx);
                    return;
                }
                this.select_business_dataset(
                    if *index == 0 {
                        Dataset::Products
                    } else {
                        Dataset::Orders
                    },
                    cx,
                );
            }))
            .children(plugin.datasets().iter().map(|&tab| {
                let count = view.counts[if tab == Dataset::Products { 0 } else { 1 }];
                Tab::new()
                    .prefix(Icon::new(if tab == Dataset::Products {
                        FunctionIcon::Products
                    } else {
                        FunctionIcon::Orders
                    }))
                    .label(tab.label())
                    .aria_label(tab.label())
                    .when(count > 0, |tab| {
                        tab.suffix(Badge::new().count(count).color(theme::accent(cx)))
                    })
            }))
            .child(
                Tab::new()
                    .prefix(Icon::new(gpui_component::IconName::Settings2))
                    .label("任务配置"),
            );
        let toolbar = h_flex()
            .gap_2()
            .items_center()
            .flex_shrink_0()
            .child(tabs)
            .child(div().flex_1())
            .when(!view.show_tasks,|bar| {
                let operation=dataset.operation();
                let id=app.id.clone();
                let busy=self.home.manual_syncs.contains(&(id.clone(),operation));
                bar.child(Button::new("shop-data-refresh")
                    .ghost()
                    .icon(FunctionIcon::Refresh)
                    .tooltip(if busy {"正在同步"}else{operation.label()})
                    .accessibility_label(operation.label())
                    .debug_selector(||"shop-data-refresh".into())
                    .loading(busy)
                    .disabled(self.read_only || busy)
                    .on_click(cx.listener(move |this,_,_,cx|this.run_shop_sync(id.clone(),operation,cx))))
            })
            .when(view.show_tasks, |bar| {
                bar.child(
                    Button::new("shop-task-list")
                        .ghost()
                        .icon(gpui_component::IconName::File)
                        .tooltip("执行列表")
                        .accessibility_label("执行列表")
                        .debug_selector(|| "shop-task-list".into())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_task_execution_list(window, cx)
                        })),
                )
            });
        let body = div()
            .id("shop-data-body")
            .debug_selector(|| "shop-data-body".into())
            .flex_1()
            .min_h_0();
        let body = if view.show_tasks {
            if self.home.task_config.rows.is_empty() {
                body.flex()
                    .items_center()
                    .justify_center()
                    .child(crate::ui::empty_icon(cx))
            } else {
                body.overflow_y_scroll()
                    .child(self.render_shop_task_config(cx))
            }
        } else if let Some(error) = &view.error {
            body.flex()
                .items_center()
                .justify_center()
                .child(crate::typography::caption(error.clone(), cx))
        } else if view.loading {
            body.flex().items_center().justify_center().child(
                div()
                    .id("shop-data-loading")
                    .debug_selector(|| "shop-data-loading".into())
                    .child(gpui_component::spinner::Spinner::new()),
            )
        } else if view.page.rows.is_empty() {
            body.flex()
                .items_center()
                .justify_center()
                .child(crate::ui::empty_icon(cx))
        } else {
            body.overflow_y_scroll()
                .children(view.page.rows.iter().map(|row| {
                    div()
                        .py_2()
                        .border_b_1()
                        .border_color(theme::line(cx))
                        .child(plugin.business_row(row, cx))
                }))
        };
        v_flex()
            .h_full()
            .min_h_0()
            .gap_2()
            .child(toolbar)
            .child(body)
            .into_any_element()
    }
}
