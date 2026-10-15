//! Proxy configuration UI uses standard GPUI Component forms and selects.
use super::*;
use gpui_component::button::ButtonVariants as _;
use gpui_component::{
    ActiveTheme, StyledExt,
    form::{Field, Form},
    group_box::{GroupBox, GroupBoxVariants},
    select::{Select, SelectItem, SelectState},
};
use gpui_component::{
    Disableable,
    button::Button,
    h_flex,
    input::{Input, InputState},
    switch::Switch,
    v_flex,
};

#[derive(Clone)]
pub(in crate::app) struct ProxyChoice {
    id: String,
    title: String,
}
impl SelectItem for ProxyChoice {
    type Value = String;
    fn title(&self) -> SharedString {
        self.title.clone().into()
    }
    fn value(&self) -> &String {
        &self.id
    }
}
pub(in crate::app) struct ProxyView {
    pub select: Entity<SelectState<Vec<ProxyChoice>>>,
    pub binding_select: Entity<SelectState<Vec<ProxyChoice>>>,
    pub entries: Vec<SocksProxy>,
    pub binding: Option<String>,
    editing: Option<String>,
    name: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    exclusive: bool,
    error: Option<String>,
}
impl ProxyView {
    pub fn new(window: &mut Window, cx: &mut Context<AddressBookApp>) -> Self {
        Self {
            select: cx.new(|cx| SelectState::new(Vec::<ProxyChoice>::new(), None, window, cx)),
            binding_select: cx.new(|cx| {
                SelectState::new(
                    vec![ProxyChoice {
                        id: String::new(),
                        title: "直连".into(),
                    }],
                    None,
                    window,
                    cx,
                )
            }),
            entries: vec![],
            binding: None,
            editing: None,
            name: cx.new(|cx| InputState::new(window, cx)),
            host: cx.new(|cx| InputState::new(window, cx)),
            port: cx.new(|cx| InputState::new(window, cx).default_value("1080")),
            username: cx.new(|cx| InputState::new(window, cx)),
            password: cx.new(|cx| InputState::new(window, cx).masked(true)),
            exclusive: false,
            error: None,
        }
    }
}
impl AddressBookApp {
    #[cfg(test)]
    pub(in crate::app) fn open_proxy_manager(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.credential_busy
            || self
                .home
                .deletion
                .as_ref()
                .is_some_and(|deletion| deletion.busy)
        {
            return;
        }
        self.load_proxy_manager(window, cx);
        self.modal = Modal::None;
        self.settings.page = Some(super::super::settings::SettingsPage::Proxies);
        self.settings.revision = self.settings.revision.wrapping_add(1);
        self.focus.focus(window, cx);
        cx.notify();
    }
    pub(in crate::app) fn load_proxy_manager(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.home.proxies.error = None;
        self.home.proxies.entries = WorkspaceStore::open(self.book.paths())
            .and_then(|s| s.proxies())
            .unwrap_or_else(|error| {
                self.home.proxies.error = Some(error.to_string());
                vec![]
            });
        let error = self.home.proxies.error.take();
        let proxy = self.home.proxies.entries.first().cloned();
        self.sync_proxy_choices(window, cx);
        self.edit_proxy(proxy, window, cx);
        self.home.proxies.error = error;
    }
    fn edit_proxy(
        &mut self,
        proxy: Option<SocksProxy>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = &mut self.home.proxies;
        view.editing = proxy.as_ref().map(|p| p.id.clone());
        for (input, value) in [
            (
                &view.name,
                proxy.as_ref().map(|p| p.name.clone()).unwrap_or_default(),
            ),
            (
                &view.host,
                proxy.as_ref().map(|p| p.host.clone()).unwrap_or_default(),
            ),
            (
                &view.port,
                proxy
                    .as_ref()
                    .map(|p| p.port.to_string())
                    .unwrap_or("1080".into()),
            ),
            (
                &view.username,
                proxy
                    .as_ref()
                    .map(|p| p.username.clone())
                    .unwrap_or_default(),
            ),
            (
                &view.password,
                proxy
                    .as_ref()
                    .map(|p| p.password.clone())
                    .unwrap_or_default(),
            ),
        ] {
            input.update(cx, |s, cx| s.set_value(value, window, cx));
        }
        view.exclusive = proxy.is_some_and(|p| p.mode == ProxyMode::Exclusive);
        let id = view.editing.clone();
        view.select.update(cx, |s, cx| match id {
            Some(id) => s.set_selected_value(&id, window, cx),
            None => s.set_selected_index(None, window, cx),
        });
        view.error = None;
        cx.notify();
    }
    pub(in crate::app) fn save_proxy_config(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        let view = &self.home.proxies;
        let Ok(port) = view.port.read(cx).value().parse::<u16>() else {
            self.home.proxies.error = Some("端口必须为 1–65535".into());
            cx.notify();
            return;
        };
        let proxy = SocksProxy {
            id: view
                .editing
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name: view.name.read(cx).value().trim().into(),
            host: view.host.read(cx).value().trim().into(),
            port,
            username: view.username.read(cx).value().into(),
            password: view.password.read(cx).unmask_value().into(),
            mode: if view.exclusive {
                ProxyMode::Exclusive
            } else {
                ProxyMode::Shared
            },
        };
        match WorkspaceStore::open(self.book.paths()).and_then(|mut s| {
            s.save_proxy(&proxy)?;
            s.proxies()
        }) {
            Ok(entries) => {
                self.home.proxies.entries = entries;
                self.home.proxies.error = None;
                self.sync_proxy_choices(window, cx);
                self.edit_proxy(Some(proxy), window, cx);
            }
            Err(error) => self.home.proxies.error = Some(error.to_string()),
        }
        cx.notify();
    }
    pub(in crate::app) fn render_proxy_binding(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_1()
            .child(crate::typography::label("网络", cx))
            .child(
                div()
                    .id("app-proxy-binding")
                    .debug_selector(|| "app-proxy-binding".into())
                    .child(
                        Select::new(&self.home.proxies.binding_select)
                            .accessibility_label("网络")
                            .disabled(self.read_only),
                    ),
            )
            .into_any_element()
    }
    pub(in crate::app) fn sync_app_proxy_binding(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = &self.home.proxies;
        let mut items = vec![ProxyChoice {
            id: String::new(),
            title: "直连".into(),
        }];
        items.extend(view.entries.iter().map(|p| ProxyChoice {
            id: p.id.clone(),
            title: p.name.clone(),
        }));
        let selected = view.binding.clone().unwrap_or_default();
        view.binding_select.update(cx, |s, cx| {
            s.set_items(items, window, cx);
            s.set_selected_value(&selected, window, cx);
        });
    }
    pub(in crate::app) fn sync_proxy_choices(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = self
            .home
            .proxies
            .entries
            .iter()
            .map(|p| ProxyChoice {
                id: p.id.clone(),
                title: p.name.clone(),
            })
            .collect();
        self.home
            .proxies
            .select
            .update(cx, |s, cx| s.set_items(items, window, cx));
    }
    pub(in crate::app) fn select_proxy_config(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let proxy = self
            .home
            .proxies
            .entries
            .iter()
            .find(|p| p.id == id)
            .cloned();
        if let Some(proxy) = proxy {
            self.edit_proxy(Some(proxy), window, cx);
        }
    }
    pub(in crate::app) fn clear_proxy_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_proxy(None, window, cx);
    }
    fn cancel_proxy_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let proxy = self
            .home
            .proxies
            .entries
            .iter()
            .find(|p| Some(&p.id) == self.home.proxies.editing.as_ref())
            .cloned();
        self.edit_proxy(proxy, window, cx);
    }
    fn delete_selected_proxy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let Some(id) = self.home.proxies.editing.clone() else {
            return;
        };
        match WorkspaceStore::open(self.book.paths()).and_then(|mut s| {
            s.delete_proxy(&id)?;
            s.proxies()
        }) {
            Ok(entries) => {
                self.home.proxies.entries = entries;
                self.sync_proxy_choices(window, cx);
                let next = self.home.proxies.entries.first().cloned();
                self.edit_proxy(next, window, cx);
            }
            Err(error) => {
                self.home.proxies.error = Some(format!(
                    "删除失败：{error}。若仍被 App 使用，请先更换绑定。"
                ));
            }
        }
        cx.notify();
    }
    pub(in crate::app) fn render_proxy_manager(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = &self.home.proxies;
        div()
            .id("settings-proxies")
            .debug_selector(|| "settings-proxies".into())
            .child(
                GroupBox::new()
                    .outline()
                    .child(
                        v_flex()
                            .gap_4()
                            .child(
                                h_flex()
                                    .justify_between()
                                    .items_center()
                                    .child(div().font_semibold().child("SOCKS5 代理"))
                                    .child(
                                        Button::new("proxy-new")
                                            .debug_selector(|| "proxy-new".into())
                                            .outline()
                                            .icon(gpui_component::IconName::Plus)
                                            .label("新增")
                                            .disabled(self.read_only)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.edit_proxy(None, window, cx)
                                            })),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        div().flex_1().min_w_0().child(
                                            Select::new(&view.select)
                                                .placeholder("选择代理")
                                                .accessibility_label("代理配置"),
                                        ),
                                    )
                                    .child(
                                        Button::new("proxy-delete")
                                            .debug_selector(|| "proxy-delete".into())
                                            .outline()
                                            .icon(gpui_component::IconName::Delete)
                                            .tooltip("删除代理")
                                            .disabled(self.read_only || view.editing.is_none())
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.delete_selected_proxy(window, cx)
                                            })),
                                    ),
                            )
                            .child(div().border_b_1().border_color(cx.theme().border))
                            .child(
                                Form::horizontal()
                                    .child(
                                        Field::new()
                                            .label("名称")
                                            .items_center()
                                            .child(Input::new(&view.name).disabled(self.read_only)),
                                    )
                                    .child(
                                        Field::new()
                                            .label("地址")
                                            .items_center()
                                            .child(Input::new(&view.host).disabled(self.read_only)),
                                    )
                                    .child(
                                        Field::new()
                                            .label("端口")
                                            .items_center()
                                            .child(Input::new(&view.port).disabled(self.read_only)),
                                    )
                                    .child(
                                        Field::new().label("用户名").items_center().child(
                                            Input::new(&view.username).disabled(self.read_only),
                                        ),
                                    )
                                    .child(
                                        Field::new().label("密码").items_center().child(
                                            Input::new(&view.password).disabled(self.read_only),
                                        ),
                                    )
                                    .child(
                                        Field::new().label("排他使用").items_center().child(
                                            h_flex().justify_end().child(
                                                Switch::new("proxy-exclusive")
                                                    .checked(view.exclusive)
                                                    .disabled(self.read_only)
                                                    .on_click(cx.listener(
                                                        |this, value: &bool, _, cx| {
                                                            this.home.proxies.exclusive = *value;
                                                            cx.notify();
                                                        },
                                                    )),
                                            ),
                                        ),
                                    ),
                            )
                            .children(
                                view.error
                                    .as_ref()
                                    .map(|error| crate::typography::caption(error.clone(), cx)),
                            ),
                    )
                    .child(
                        h_flex()
                            .pt_4()
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .gap_2()
                            .justify_end()
                            .child(
                                Button::new("proxy-cancel")
                                    .label("取消")
                                    .debug_selector(|| "proxy-cancel".into())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.cancel_proxy_edit(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("proxy-save")
                                    .primary()
                                    .label("保存")
                                    .debug_selector(|| "proxy-save".into())
                                    .disabled(self.read_only)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_proxy_config(window, cx)
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{ProxyChoice, StorePaths, WorkspaceStore};
    use crate::app::ui_tests::{click, setup};
    use gpui::TestAppContext;
    #[gpui::test]
    fn new_proxy_save_cancel_and_select_use_persisted_identity(cx: &mut TestAppContext) {
        let (app, cx, dir) = setup(cx, vec![]);
        let paths = StorePaths::in_dir(dir.path().into());
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.open_proxy_manager(window, cx);
                for (input, value) in [
                    (&app.home.proxies.name, "法国出口"),
                    (&app.home.proxies.host, "127.0.0.1"),
                    (&app.home.proxies.username, "user"),
                    (&app.home.proxies.password, "secret"),
                ] {
                    input.update(cx, |s, cx| s.set_value(value, window, cx));
                }
            })
        });
        click(cx, "proxy-save");
        let entries = WorkspaceStore::open(&paths).unwrap().proxies().unwrap();
        assert_eq!(entries.len(), 1);
        let id = entries[0].id.clone();
        assert_eq!(entries[0].password, "secret");
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.home
                    .proxies
                    .name
                    .update(cx, |s, cx| s.set_value("discard draft", window, cx))
            })
        });
        click(cx, "proxy-cancel");
        app.read_with(cx, |app, cx| {
            assert_eq!(app.home.proxies.name.read(cx).value(), "法国出口")
        });
        click(cx, "proxy-new");
        app.read_with(cx, |app, cx| {
            assert!(app.home.proxies.editing.is_none());
            assert!(app.home.proxies.password.read(cx).value().is_empty());
        });
        app.update(cx, |app, cx| {
            app.home.proxies.select.update(cx, |_, cx| {
                cx.emit(
                    gpui_component::select::SelectEvent::<Vec<ProxyChoice>>::Confirm(Some(
                        id.clone(),
                    )),
                )
            })
        });
        cx.run_until_parked();
        app.read_with(cx, |app, cx| {
            assert_eq!(app.home.proxies.editing.as_ref(), Some(&id));
            assert_eq!(app.home.proxies.password.read(cx).value(), "secret");
        });
        click(cx, "proxy-save");
        assert_eq!(
            WorkspaceStore::open(&paths)
                .unwrap()
                .proxies()
                .unwrap()
                .len(),
            1
        );
        click(cx, "proxy-delete");
        assert!(
            WorkspaceStore::open(&paths)
                .unwrap()
                .proxies()
                .unwrap()
                .is_empty()
        );
    }
}
