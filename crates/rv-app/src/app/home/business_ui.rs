//! Shared host data controller; Bundle renderers own the row presentation.
use super::*;
use crate::{assets::FunctionIcon, theme};
use gpui_component::{
    Disableable, Icon,
    badge::Badge,
    button::{Button, ButtonVariants},
    h_flex,
    skeleton::Skeleton,
    switch::Switch,
    tab::{Tab, TabBar},
    v_flex,
};
use rv_business::data::{BusinessPage, Dataset};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
pub(in crate::app) struct BusinessView {
    pub dataset: Option<Dataset>,
    pub page: BusinessPage,
    pub counts: [usize; 2],
    pub enabled: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub sequence: u64,
}
impl BusinessView {
    pub fn new(_window: &mut Window, _cx: &mut Context<AddressBookApp>) -> Self {
        Self {
            dataset: None,
            page: Default::default(),
            counts: [0; 2],
            enabled: false,
            loading: false,
            error: None,
            sequence: 0,
        }
    }
}
impl AddressBookApp {
    pub(in crate::app) fn render_shop_operations(&self, cx: &mut Context<Self>) -> AnyElement {
        use gpui_component::menu::{DropdownMenu, PopupMenuItem};
        let Some(app) = &self.home.editing else {
            return div().into_any_element();
        };
        let id = app.id.clone();
        let bundle = app.bundle_id.clone();
        let operations = self
            .home
            .plugins
            .for_bundle(&bundle)
            .offline_operations()
            .to_vec();
        let links = self.home.links.get(&id).cloned().unwrap_or_default();
        let book = cx.entity();
        Button::new("shop-operations")
            .debug_selector(|| "shop-operations".into())
            .ghost()
            .icon(gpui_component::IconName::Ellipsis)
            .tooltip("店铺操作")
            .accessibility_label("店铺操作")
            .disabled(self.read_only)
            .dropdown_menu(move |mut menu, _, _| {
                for operation in &operations {
                    let book = book.clone();
                    let id = id.clone();
                    let operation = *operation;
                    menu = menu.item(PopupMenuItem::new(operation.label()).on_click(
                        move |_, _, cx| {
                            book.update(cx, |this, cx| {
                                this.run_offline_job(id.clone(), operation, cx)
                            });
                        },
                    ));
                }
                for link in &links {
                    let book = book.clone();
                    let id = id.clone();
                    let bundle = bundle.clone();
                    let link = link.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format!("读取账号 · {}", link.title)).on_click(
                            move |_, _, cx| {
                                book.update(cx, |this, cx| {
                                    this.read_backup_profile(
                                        id.clone(),
                                        link.clone(),
                                        bundle.clone(),
                                        cx,
                                    )
                                });
                            },
                        ),
                    );
                }
                menu
            })
            .into_any_element()
    }
    pub(super) fn subscribe_business(&mut self, cx: &mut Context<Self>) {
        // This task belongs to the RV window, never to a phone session.
        cx.spawn(async move |this, cx| {
            let mut clock = rv_business::schedules::ScheduleClock::default();
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(30))
                    .await;
                let paths = match this.update(cx, |this, _| {
                    if this.read_only {
                        None
                    } else {
                        Some(this.book.paths().clone())
                    }
                }) {
                    Ok(Some(paths)) => paths,
                    Ok(None) => continue,
                    Err(_) => break,
                };
                let (updated_clock, due) = cx
                    .background_executor()
                    .spawn(async move {
                        let due = clock.due(&paths, now());
                        (clock, due)
                    })
                    .await;
                clock = updated_clock;
                if this
                    .update(cx, |this, cx| match due {
                        Ok(jobs) => {
                            for job in jobs {
                                this.dispatch_offline_job(
                                    job.app_id.clone(),
                                    job.operation,
                                    Some(job),
                                    cx,
                                );
                            }
                        }
                        Err(error) => this.set_status(format!("定时同步读取失败：{error}"), cx),
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }
    pub(super) fn reset_business_view(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.home.business.sequence += 1;
        self.home.business.dataset = self.home.editing.as_ref().and_then(|app| {
            self.home
                .plugins
                .for_bundle(&app.bundle_id)
                .datasets()
                .first()
                .copied()
        });
        self.home.business.page = Default::default();
        self.home.business.enabled = false;
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
                    let enabled =
                        rv_business::schedules::enabled(&paths, &id, dataset.operation())?;
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
                    Ok::<_, String>((page, enabled, counts))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.home.business.sequence != sequence || this.home.editing.is_none() {
                    return;
                }
                this.home.business.loading = false;
                match result {
                    Ok((page, enabled, counts)) => {
                        this.home.business.counts = counts;
                        this.home.business.page = page;
                        this.home.business.enabled = enabled;
                        this.load_detail_images(sequence, cx);
                    }
                    Err(error) => this.home.business.error = Some(error),
                };
                cx.notify();
            });
        })
        .detach();
    }
    fn load_detail_images(&mut self, sequence: u64, cx: &mut Context<Self>) {
        let paths = self.book.paths().clone();
        let rows = self.home.business.page.rows.clone();
        let Some(id) = self.home.editing.as_ref().map(|app| app.id.clone()) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let rows = cx
                .background_executor()
                .spawn(async move {
                    let mut rows = rows;
                    rv_business::assets::hydrate(&paths, &id, &mut rows);
                    rows
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.home.business.sequence == sequence && this.home.editing.is_some() {
                    this.home.business.page.rows = rows;
                    cx.notify();
                }
            });
        })
        .detach();
    }
    fn select_business_dataset(&mut self, dataset: Dataset, cx: &mut Context<Self>) {
        self.home.business.dataset = Some(dataset);
        self.home.business.page = Default::default();
        self.reload_business_data(cx);
    }
    fn toggle_business_schedule(&mut self, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let (Some(app), Some(dataset)) = (self.home.editing.as_ref(), self.home.business.dataset)
        else {
            return;
        };
        let enabled = !self.home.business.enabled;
        match rv_business::schedules::set(
            self.book.paths(),
            &app.id,
            dataset.operation(),
            enabled,
            900,
        ) {
            Ok(()) => {
                self.home.business.enabled = enabled;
                cx.notify();
            }
            Err(error) => self.set_status(error, cx),
        }
    }
    pub(in crate::app) fn render_business_data(&self, cx: &mut Context<Self>) -> AnyElement {
        let (Some(app), Some(dataset)) = (self.home.editing.as_ref(), self.home.business.dataset)
        else {
            return div().into_any_element();
        };
        let view = &self.home.business;
        let plugin = self.home.plugins.for_bundle(&app.bundle_id);
        let id = app.id.clone();
        let mut content = v_flex().gap_2().child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    TabBar::new("shop-datasets")
                        .underline()
                        .selected_index(if dataset == Dataset::Products { 0 } else { 1 })
                        .on_click(cx.listener(|this, index: &usize, _, cx| {
                            this.select_business_dataset(
                                if *index == 0 {
                                    Dataset::Products
                                } else {
                                    Dataset::Orders
                                },
                                cx,
                            )
                        }))
                        .children(plugin.datasets().iter().map(|&tab| {
                            Tab::new().aria_label(tab.label()).child(
                                Badge::new()
                                    .count(
                                        view.counts[if tab == Dataset::Products { 0 } else { 1 }],
                                    )
                                    .color(theme::accent(cx))
                                    .child(Icon::new(if tab == Dataset::Products {
                                        FunctionIcon::Products
                                    } else {
                                        FunctionIcon::Orders
                                    })),
                            )
                        })),
                )
                .child(div().flex_1())
                .child(
                    Button::new("business-sync")
                        .ghost()
                        .icon(FunctionIcon::Refresh)
                        .tooltip("同步")
                        .accessibility_label("同步")
                        .disabled(self.read_only || self.home.jobs.contains(&id))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.run_offline_job(id.clone(), dataset.operation(), cx)
                        })),
                )
                .child(div().text_xs().child("定时"))
                .child(
                    Switch::new("business-schedule")
                        .checked(view.enabled)
                        .disabled(self.read_only || view.loading)
                        .on_click(
                            cx.listener(|this, _: &bool, _, cx| this.toggle_business_schedule(cx)),
                        ),
                ),
        );
        if let Some(error) = &view.error {
            content = content.child(div().text_xs().child(error.clone()));
        } else if view.loading || view.page.rows.is_empty() {
            content = content.child(plugin.detail_placeholder(cx));
        } else {
            content = content.children(view.page.rows.iter().map(|row| {
                div()
                    .py_2()
                    .border_b_1()
                    .border_color(theme::line(cx))
                    .child(plugin.business_row(row, cx))
            }));
        }
        let footer = if let Some(ts) = view
            .page
            .last_sync
            .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
        {
            div()
                .text_xs()
                .text_color(theme::muted(cx))
                .child(format!(
                    "更新于 {}",
                    ts.with_timezone(&chrono::Local).format("%H:%M")
                ))
                .into_any_element()
        } else {
            Skeleton::new()
                .bg(theme::line(cx))
                .w(px(88.))
                .h(px(12.))
                .rounded(theme::radius_small(cx))
                .into_any_element()
        };
        content.child(footer).into_any_element()
    }
}
