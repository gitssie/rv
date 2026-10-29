use super::*;
use gpui_component::{
    Disableable,
    button::{Button, ButtonVariants},
    h_flex,
    menu::{DropdownMenu, PopupMenuItem},
};
use rv_core::workspace::TokenSource;

impl AddressBookApp {
    pub(super) fn render_token_source(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(app) = &self.home.editing else {
            return div().into_any_element();
        };
        let id = app.id.clone();
        let source = WorkspaceStore::open(self.book.paths())
            .and_then(|s| s.token_source(&id))
            .unwrap_or_default();
        let devices = self.book.connections().to_vec();
        let owner = cx.entity();
        let menu_id = id.clone();
        let mut row = h_flex().gap_1().items_center().child(
            Button::new("shop-token-source")
                .ghost()
                .label(match source {
                    TokenSource::Rv => "Token：RV",
                    TokenSource::Phone(_) => "Token：手机",
                })
                .tooltip("选择 Token 来源；业务请求共用当前 Token，刷新责任由来源决定")
                .debug_selector(|| "shop-token-source".into())
                .loading(self.home.handoff_running.contains_key(&id))
                .disabled(self.read_only || self.home.handoff_running.contains_key(&id))
                .dropdown_menu(move |menu, _, _| {
                    let rv_owner = owner.clone();
                    let rv_id = menu_id.clone();
                    let mut menu = menu.item(PopupMenuItem::new("RV 提供并刷新").on_click(
                        move |_, window, cx| {
                            rv_owner.update(cx, |this, cx| {
                                this.confirm_token_rv_takeover(&rv_id, window, cx)
                            });
                        },
                    ));
                    for device in &devices {
                        let phone_owner = owner.clone();
                        let app = menu_id.clone();
                        let device_id = device.id;
                        menu = menu.item(
                            PopupMenuItem::new(format!("手机：{}", device.name)).on_click(
                                move |_, _, cx| {
                                    phone_owner.update(cx, |this, cx| {
                                        this.change_phone_token_source(
                                            &app,
                                            device_id,
                                            cx,
                                        )
                                    });
                                },
                            ),
                        );
                    }
                    menu
                }),
        );
        if let TokenSource::Phone(device) = source {
            row = row.child(
                Button::new("shop-phone-token")
                    .ghost()
                    .icon(crate::assets::FunctionIcon::Refresh)
                    .debug_selector(|| "shop-phone-token".into())
                    .tooltip("从手机获取当前 Token")
                    .disabled(self.read_only)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !device.is_some_and(|phone| this.phone_online(phone, cx)) {
                            this.set_error_status("提供 Token 的手机尚未连接", cx);
                            return;
                        }
                        match WorkspaceStore::open(this.book.paths())
                            .and_then(|s| s.request_phone_token(&id, true))
                        {
                            Ok(()) => this.set_status("正在从手机获取 Token…", cx),
                            Err(error) => this.set_error_status(error.to_string(), cx),
                        }
                    })),
            );
        }
        row.into_any_element()
    }
    fn confirm_token_rv_takeover(&mut self, id: &AppId, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let result = WorkspaceStore::open(self.book.paths()).and_then(|store| {
            Ok((store.token_source(id)?, store.backups(id)?))
        });
        let (source, links) = match result {
            Ok(value) => value,
            Err(error) => { self.set_error_status(error.to_string(), cx); return; }
        };
        let TokenSource::Phone(device) = source else { return; };
        let Some(link) = links.into_iter().find(|link| Some(link.device_id) == device) else {
            self.set_error_status("请连接来源手机并刷新该账号的备份列表", cx);
            return;
        };
        let id = id.clone();
        let owner = cx.entity();
        crate::ui::confirm_execution_handoff(
            crate::ui::ExecutionHandoffPrompt {
                title: "交给 RV",
                icon: crate::assets::FunctionIcon::Monitor,
                device: self.takeover_phone_name(link.device_id),
                backup: link.title.clone(),
                acknowledgement: Some("已保存该环境的最新备份，且手机不再使用该环境"),
            },
            window,
            cx,
            move |_, cx| {
                owner.update(cx, |this, cx| {
                    this.start_token_rv_takeover(id.clone(), link.clone(), cx)
                });
            },
        );
    }
    fn start_token_rv_takeover(&mut self, id: AppId, link: BackupLink, cx: &mut Context<Self>) {
        let Some(view) = self.home.controls.get(&link.device_id).map(|control| control.view.clone()) else {
            self.set_error_status("请先连接提供备份的手机", cx);
            return;
        };
        if !view.read(cx).can_handoff_backup() {
            self.set_error_status("请先连接手机并完成当前备份操作", cx);
            return;
        }
        let bundle = match WorkspaceStore::open(self.book.paths()).and_then(|store| store.load(&id)) {
            Ok(app) => app.bundle_id,
            Err(error) => { self.set_error_status(error.to_string(), cx); return; }
        };
        let owner = view.entity_id();
        match self.prepare_rv_takeover(&id, &bundle, &link.backup_id, link.device_id, owner, cx) {
            Ok(command) => {
                if !view.update(cx, |view, cx| view.takeover_backup(command, cx)) {
                    self.rv_takeover_finished(&id, owner, false, cx);
                }
            }
            Err(error) => self.set_error_status(error, cx),
        }
    }
    fn change_phone_token_source(&mut self, id: &AppId, device: ConnectionId, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        match WorkspaceStore::open(self.book.paths())
            .and_then(|mut store| store.set_token_source(id, TokenSource::Phone(Some(device))))
        {
            Ok(()) => self.set_status("Token 来源已设为手机，RV 不刷新令牌", cx),
            Err(error) => self.set_error_status(error.to_string(), cx),
        }
        cx.notify();
    }
}
