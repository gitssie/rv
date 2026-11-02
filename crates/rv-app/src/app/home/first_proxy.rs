//! One proxy choice for a first-import batch; existing bindings are preserved.
use super::*;
use gpui_component::{
    Disableable,
    button::{Button, ButtonVariants},
    dialog::{Dialog, DialogFooter},
    form::{Field, Form},
    select::{Select, SelectState},
    v_flex,
};
use rv_session::AppCommand;

pub(in crate::app) struct FirstProxyBatch {
    id: String,
    device: ConnectionId,
    commands: Vec<AppCommand>,
    session: WeakEntity<crate::session_window::SessionView>,
    titles: Vec<String>,
    select: Option<Entity<SelectState<Vec<ProxyChoice>>>>,
    error: Option<String>,
}

impl AddressBookApp {
    pub(crate) fn stage_initial_profiles(
        &mut self,
        device: ConnectionId,
        commands: Vec<AppCommand>,
        session: WeakEntity<crate::session_window::SessionView>,
        cx: &mut Context<Self>,
    ) -> Vec<AppCommand> {
        let Ok(store) = WorkspaceStore::open(self.book.paths()) else {
            return vec![];
        };
        let mut ready = vec![];
        let mut pending = vec![];
        let mut titles = vec![];
        for command in commands {
            let AppCommand::ReadProfile { app_id, .. } = &command else {
                continue;
            };
            match store.proxy_binding(app_id) {
                Ok(Some(_)) => ready.push(command),
                Ok(None) => {
                    if self.home.first_proxy.iter().any(|batch| batch.commands.iter().any(|command| matches!(command, AppCommand::ReadProfile { app_id: other, .. } if other == app_id))) { continue; }
                    if let Ok(app) = store.load(app_id) {
                        titles.push(app.title);
                        pending.push(command);
                    }
                }
                Err(error) => tracing::error!(%error,"Initial proxy binding read failed"),
            }
        }
        if !pending.is_empty() {
            self.home.first_proxy.push_back(FirstProxyBatch {
                id: uuid::Uuid::new_v4().to_string(),
                device,
                commands: pending,
                session,
                titles,
                select: None,
                error: None,
            });
            cx.notify();
        }
        ready
    }

    pub(in crate::app) fn prepare_first_proxy_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(self.modal, Modal::None | Modal::PhoneControl(_)) {
            return;
        }
        let Some(batch) = self.home.first_proxy.front_mut() else {
            return;
        };
        let choices =
            match WorkspaceStore::open(self.book.paths()).and_then(|store| store.proxies()) {
                Ok(entries) => entries
                    .into_iter()
                    .map(|proxy| ProxyChoice {
                        id: proxy.id,
                        title: proxy.name,
                    })
                    .collect(),
                Err(error) => {
                    batch.error = Some(format!("读取代理失败：{error}"));
                    vec![]
                }
            };
        batch.select = Some(cx.new(|cx| SelectState::new(choices, None, window, cx)));
        self.home.first_proxy_return = Some(self.modal.clone());
        self.modal = Modal::FirstProxy;
    }

    pub(in crate::app) fn configure_first_proxy_dialog(
        &self,
        dialog: Dialog,
        cx: &mut Context<Self>,
    ) -> Dialog {
        let Some(batch) = self.home.first_proxy.front() else {
            return dialog;
        };
        let Some(select) = &batch.select else {
            return dialog;
        };
        let body = v_flex()
            .gap_3()
            .child(crate::typography::caption(
                format!(
                    "为本次发现的 {} 个未配置店铺统一选择代理。已有代理绑定保持原样。",
                    batch.commands.len()
                ),
                cx,
            ))
            .child(
                Form::vertical().child(
                    Field::new()
                        .label("代理")
                        .child(Select::new(select).accessibility_label("首次同步代理")),
                ),
            )
            .child(
                v_flex()
                    .id("first-proxy-shops")
                    .max_h(px(180.))
                    .overflow_y_scroll()
                    .children(
                        batch
                            .titles
                            .iter()
                            .map(|title| div().text_sm().child(title.clone())),
                    ),
            )
            .children(
                batch
                    .error
                    .iter()
                    .map(|error| div().text_sm().child(error.clone())),
            )
            .child(crate::typography::caption(
                "没有可用代理时，先取消并在设置 → 代理中添加，再刷新手机备份。",
                cx,
            ));
        let confirm = cx.entity();
        let cancel = confirm.clone();
        let closed_batch = batch.id.clone();
        dialog
            .title("选择店铺代理")
            .child(body)
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("first-proxy-cancel")
                            .label("取消")
                            .on_click(cx.listener(|this, _, _, cx| this.close_first_proxy(cx))),
                    )
                    .child(
                        Button::new("first-proxy-save")
                            .primary()
                            .label("确认并同步")
                            .disabled(self.read_only)
                            .on_click(cx.listener(|this, _, _, cx| this.confirm_first_proxy(cx))),
                    ),
            )
            .on_ok(move |_, _, cx| {
                confirm.update(cx, |this, cx| this.confirm_first_proxy(cx));
                false
            })
            .on_cancel(move |_, _, cx| {
                cancel.update(cx, |this, cx| this.close_first_proxy(cx));
                true
            })
            .on_close(cx.listener(move |this, _, _, cx| {
                if this
                    .home
                    .first_proxy
                    .front()
                    .is_some_and(|batch| batch.id == closed_batch)
                {
                    this.close_first_proxy(cx);
                }
            }))
    }

    pub(in crate::app) fn close_first_proxy(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.modal, Modal::FirstProxy) {
            return;
        }
        self.home.first_proxy.pop_front();
        self.modal = self.home.first_proxy_return.take().unwrap_or(Modal::None);
        cx.notify();
    }

    fn confirm_first_proxy(&mut self, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let Some(batch) = self.home.first_proxy.front() else {
            return;
        };
        let proxy = batch
            .select
            .as_ref()
            .and_then(|select| select.read(cx).selected_value().cloned());
        let Some(proxy) = proxy else {
            self.home.first_proxy.front_mut().unwrap().error = Some("请选择代理后确认".into());
            cx.notify();
            return;
        };
        let ids: Vec<_> = batch
            .commands
            .iter()
            .filter_map(|command| match command {
                AppCommand::ReadProfile { app_id, .. } => Some(app_id.clone()),
                _ => None,
            })
            .collect();
        let result = WorkspaceStore::open(self.book.paths())
            .and_then(|mut store| store.bind_initial_proxies(&ids, &proxy));
        if let Err(error) = result {
            self.home.first_proxy.front_mut().unwrap().error = Some(error.to_string());
            cx.notify();
            return;
        }
        let batch = self.home.first_proxy.front().unwrap();
        let started = batch.session.upgrade().is_some_and(|session| {
            session.update(cx, |session, cx| {
                session.continue_initial_profiles(batch.device, batch.commands.clone(), cx)
            })
        });
        self.close_first_proxy(cx);
        self.reload_home(cx);
        self.set_status(
            if started {
                "代理已绑定，正在同步账号备份…"
            } else {
                "代理已绑定，请连接来源手机并再次刷新备份完成初始化"
            },
            cx,
        );
    }
}
