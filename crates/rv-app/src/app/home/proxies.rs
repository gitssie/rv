//! Proxy configuration UI uses standard GPUI Component forms and selects.
use super::*;
use gpui::prelude::FluentBuilder as _;
use gpui_component::button::ButtonVariants as _;
use gpui_component::{
    ActiveTheme,
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
use gpui_component::{Icon, IconName, WindowExt, dialog::DialogButtonProps, list::ListItem};

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
    pub protocol_select: Entity<SelectState<Vec<String>>>,
    protocol: rv_core::workspace::ProxyProtocol,
    pub rotation_select: Entity<SelectState<Vec<String>>>,
    pub password_rotation: bool,
    session_marker: Entity<InputState>,
    pub front_select: Entity<SelectState<Vec<ProxyChoice>>>,
    pub front_id: Option<String>,
    draft_id: String,
    engine_path: Entity<InputState>,
    engine_info: Option<Result<(String, String), String>>,
    engine_generation: u64,
    pub binding_select: Entity<SelectState<Vec<ProxyChoice>>>,
    pub entries: Vec<SocksProxy>,
    pub binding: Option<String>,
    editing: Option<String>,
    name: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    change_url: Entity<InputState>,
    probe_url: Entity<InputState>,
    exclusive: bool,
    probe_generation: u64,
    probing: bool,
    observation: Option<(Vec<u8>, String)>,
    error: Option<String>,
}
impl ProxyView {
    pub fn new(window: &mut Window, cx: &mut Context<AddressBookApp>) -> Self {
        Self {
            protocol_select: cx.new(|cx| {
                let mut select = SelectState::new(
                    vec!["SOCKS5".to_owned(), "HTTP".to_owned()],
                    None,
                    window,
                    cx,
                );
                select.set_selected_value(&"SOCKS5".to_owned(), window, cx);
                select
            }),
            rotation_select: cx.new(|cx| {
                let mut select = SelectState::new(
                    vec!["HTTP URL".to_owned(), "密码 Session".to_owned()],
                    None,
                    window,
                    cx,
                );
                select.set_selected_value(&"HTTP URL".to_owned(), window, cx);
                select
            }),
            password_rotation: false,
            session_marker: cx.new(|cx| InputState::new(window, cx).default_value("_session-")),
            protocol: Default::default(),
            front_select: cx.new(|cx| {
                SelectState::new(
                    vec![ProxyChoice {
                        id: String::new(),
                        title: "不使用前置代理".into(),
                    }],
                    None,
                    window,
                    cx,
                )
            }),
            front_id: None,
            draft_id: uuid::Uuid::new_v4().to_string(),
            engine_path: cx
                .new(|cx| InputState::new(window, cx).placeholder("留空自动查找 sing-box")),
            engine_info: None,
            engine_generation: 0,
            binding_select: cx.new(|cx| {
                SelectState::new(
                    vec![ProxyChoice {
                        id: String::new(),
                        title: "未关联代理".into(),
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
            password: cx.new(|cx| InputState::new(window, cx)),
            change_url: cx.new(|cx| InputState::new(window, cx).placeholder("可选")),
            probe_url: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(rv_core::workspace::ProxyIpConfig::default().probe_url)
            }),
            exclusive: false,
            probe_generation: 0,
            probing: false,
            observation: None,
            error: None,
        }
    }
}
impl AddressBookApp {
    pub(super) fn subscribe_proxy_form(&mut self, cx: &mut Context<Self>) {
        for input in [
            self.home.proxies.name.clone(),
            self.home.proxies.host.clone(),
            self.home.proxies.port.clone(),
            self.home.proxies.username.clone(),
            self.home.proxies.password.clone(),
            self.home.proxies.change_url.clone(),
            self.home.proxies.session_marker.clone(),
            self.home.proxies.probe_url.clone(),
        ] {
            self._subscriptions.push(cx.subscribe(
                &input,
                |this, _, event: &gpui_component::input::InputEvent, cx| {
                    if matches!(event, gpui_component::input::InputEvent::Change) {
                        this.home.proxies.observation = None;
                        cx.notify();
                    }
                },
            ));
        }
    }

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
        self.refresh_proxy_engine(cx);
    }
    fn edit_proxy(
        &mut self,
        proxy: Option<SocksProxy>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = &mut self.home.proxies;
        view.probe_generation = view.probe_generation.wrapping_add(1);
        view.probing = false;
        view.observation = None;
        view.editing = proxy.as_ref().map(|p| p.id.clone());
        view.draft_id = uuid::Uuid::new_v4().to_string();
        view.front_id = proxy.as_ref().and_then(|p| p.front_proxy_id.clone());
        view.protocol = proxy.as_ref().map(|p| p.protocol).unwrap_or_default();
        let protocol = view.protocol.label().to_string();
        view.protocol_select
            .update(cx, |s, cx| s.set_selected_value(&protocol, window, cx));
        view.password_rotation = proxy.as_ref().is_some_and(|p| {
            matches!(
                p.ip.rotation,
                rv_core::workspace::ProxyRotation::PasswordSession { .. }
            )
        });
        let method = if view.password_rotation {
            "密码 Session"
        } else {
            "HTTP URL"
        }
        .to_owned();
        view.rotation_select
            .update(cx, |s, cx| s.set_selected_value(&method, window, cx));
        let marker = proxy
            .as_ref()
            .and_then(|p| match &p.ip.rotation {
                rv_core::workspace::ProxyRotation::PasswordSession { marker } => {
                    Some(marker.clone())
                }
                _ => None,
            })
            .unwrap_or_else(|| "_session-".into());
        view.session_marker
            .update(cx, |s, cx| s.set_value(marker, window, cx));
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
            (
                &view.change_url,
                proxy
                    .as_ref()
                    .and_then(|p| p.ip.change_url.clone())
                    .unwrap_or_default(),
            ),
            (
                &view.probe_url,
                proxy
                    .as_ref()
                    .map(|p| {
                        if p.ip.probe_url == "https://ipwho.is/?fields=success,ip,type" {
                            rv_core::workspace::ProxyIpConfig::default().probe_url
                        } else {
                            p.ip.probe_url.clone()
                        }
                    })
                    .unwrap_or_else(|| rv_core::workspace::ProxyIpConfig::default().probe_url),
            ),
        ] {
            input.update(cx, |s, cx| s.set_value(value, window, cx));
        }
        view.exclusive = proxy.is_some_and(|p| p.mode == ProxyMode::Exclusive);
        view.error = None;
        self.sync_proxy_choices(window, cx);
        cx.notify();
    }
    fn proxy_form(&self, cx: &Context<Self>) -> Result<SocksProxy, String> {
        let view = &self.home.proxies;
        let port = view
            .port
            .read(cx)
            .value()
            .parse::<u16>()
            .ok()
            .filter(|p| *p > 0)
            .ok_or_else(|| "端口必须为 1–65535".to_owned())?;
        Ok(SocksProxy {
            protocol: view.protocol,
            front_proxy_id: view.front_id.clone(),
            id: view
                .editing
                .clone()
                .unwrap_or_else(|| view.draft_id.clone()),
            name: view.name.read(cx).value().trim().into(),
            host: view.host.read(cx).value().trim().into(),
            port,
            username: view.username.read(cx).value().into(),
            password: view.password.read(cx).value().into(),
            ip: rv_core::workspace::ProxyIpConfig {
                rotation: if view.password_rotation {
                    rv_core::workspace::ProxyRotation::PasswordSession {
                        marker: view.session_marker.read(cx).value().to_string(),
                    }
                } else {
                    rv_core::workspace::ProxyRotation::Http
                },
                change_url: {
                    let url = view.change_url.read(cx).unmask_value().trim().to_owned();
                    (!url.is_empty()).then_some(url)
                },
                probe_url: view.probe_url.read(cx).value().trim().to_owned(),
            },
            mode: if view.exclusive {
                ProxyMode::Exclusive
            } else {
                ProxyMode::Shared
            },
        })
    }
    fn inspect_proxy(&mut self, cx: &mut Context<Self>) {
        if self.home.proxies.probing {
            return;
        }
        let proxy = match self.proxy_form(cx) {
            Ok(proxy) => proxy,
            Err(error) => {
                self.home.proxies.error = Some(error);
                cx.notify();
                return;
            }
        };
        let key = proxy_form_key(&proxy);
        let paths = self.book.paths().clone();
        let view = &mut self.home.proxies;
        view.probe_generation = view.probe_generation.wrapping_add(1);
        let generation = view.probe_generation;
        view.probing = true;
        view.observation = None;
        view.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { rv_business::proxy::inspect_blocking(&paths, &proxy) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.home.proxies.probe_generation != generation {
                    return;
                }
                this.home.proxies.probing = false;
                if this.proxy_form(cx).is_ok_and(|p| proxy_form_key(&p) == key) {
                    let text = match result {
                        Ok(body) => body,
                        Err(error) => error,
                    };
                    this.home.proxies.observation = Some((key, text));
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(in crate::app) fn save_proxy_config(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        let proxy = match self.proxy_form(cx) {
            Ok(proxy) => proxy,
            Err(error) => {
                self.home.proxies.error = Some(error);
                cx.notify();
                return;
            }
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
            .child(crate::typography::label("代理", cx))
            .child(
                div()
                    .id("app-proxy-binding")
                    .debug_selector(|| "app-proxy-binding".into())
                    .child(
                        Select::new(&self.home.proxies.binding_select)
                            .accessibility_label("代理")
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
            title: "未关联代理".into(),
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
        let view = &self.home.proxies;
        let is_front = view.editing.as_ref().is_some_and(|id| {
            view.entries
                .iter()
                .any(|p| p.front_proxy_id.as_ref() == Some(id))
        });
        let mut items = vec![ProxyChoice {
            id: String::new(),
            title: "不使用前置代理".into(),
        }];
        if !is_front {
            items.extend(
                view.entries
                    .iter()
                    .filter(|p| Some(&p.id) != view.editing.as_ref() && p.front_proxy_id.is_none())
                    .map(|p| ProxyChoice {
                        id: p.id.clone(),
                        title: format!("{} · {}:{}", p.name, p.host, p.port),
                    }),
            );
        }
        let selected = view.front_id.clone().unwrap_or_default();
        view.front_select.update(cx, |s, cx| {
            s.set_items(items, window, cx);
            s.set_selected_value(&selected, window, cx);
        });
        self.sync_app_proxy_binding(window, cx);
    }
    pub(in crate::app) fn set_proxy_protocol(&mut self, value: &str, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        self.home.proxies.protocol = if value == "HTTP" {
            rv_core::workspace::ProxyProtocol::Http
        } else {
            rv_core::workspace::ProxyProtocol::Socks5
        };
        self.home.proxies.probe_generation = self.home.proxies.probe_generation.wrapping_add(1);
        self.home.proxies.probing = false;
        self.home.proxies.observation = None;
        cx.notify();
    }
    pub(in crate::app) fn set_proxy_front(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let view = &mut self.home.proxies;
        view.front_id = (!id.is_empty()).then(|| id.to_string());
        view.probe_generation = view.probe_generation.wrapping_add(1);
        view.probing = false;
        view.observation = None;
        cx.notify();
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
    fn refresh_proxy_engine(&mut self, cx: &mut Context<Self>) {
        self.home.proxies.engine_generation += 1;
        let generation = self.home.proxies.engine_generation;
        self.home.proxies.engine_info = None;
        let paths = self.book.paths().clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rv_business::proxy::engine::info(&paths)
                        .map(|info| (info.version, info.path.display().to_string()))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.home.proxies.engine_generation == generation {
                    this.home.proxies.engine_info = Some(result);
                    cx.notify();
                }
            });
        })
        .detach();
    }
    fn configure_proxy_engine(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let saved = WorkspaceStore::open(self.book.paths()).and_then(|s| s.proxy_engine_path());
        let path = match saved {
            Ok(path) => path.unwrap_or_default(),
            Err(error) => {
                self.home.proxies.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        self.home
            .proxies
            .engine_path
            .update(cx, |input, cx| input.set_value(path, window, cx));
        let owner = cx.entity();
        let input = self.home.proxies.engine_path.clone();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let owner = owner.clone();
            let input = input.clone();
            dialog
                .title("sing-box 程序")
                .description("使用自行安装的 sing-box，留空自动查找。")
                .child(
                    Form::vertical()
                        .child(Field::new().label("程序路径").child(Input::new(&input))),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text("保存")
                        .cancel_text("取消"),
                )
                .on_ok(move |_, _, cx| {
                    owner.update(cx, |this, cx| {
                        if this.read_only {
                            return true;
                        }
                        let path = this
                            .home
                            .proxies
                            .engine_path
                            .read(cx)
                            .value()
                            .trim()
                            .to_string();
                        match WorkspaceStore::open(this.book.paths()).and_then(|mut s| {
                            s.save_proxy_engine_path((!path.is_empty()).then_some(path.as_str()))
                        }) {
                            Ok(()) => {
                                this.refresh_proxy_engine(cx);
                                true
                            }
                            Err(error) => {
                                this.home.proxies.error = Some(error.to_string());
                                cx.notify();
                                false
                            }
                        }
                    })
                })
        });
    }
    pub(in crate::app) fn render_proxy_manager(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = &self.home.proxies;
        let list = v_flex()
            .w(px(220.))
            .max_w(relative(0.32))
            .h_full()
            .min_h_0()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                crate::ui::tool_actions()
                    .child(crate::typography::label("代理列表", cx))
                    .child(div().flex_1())
                    .child(
                        Button::new("proxy-new")
                            .debug_selector(|| "proxy-new".into())
                            .ghost()
                            .icon(IconName::Plus)
                            .tooltip("新增代理")
                            .disabled(self.read_only)
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.edit_proxy(None, window, cx)
                                }),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .id("proxy-list")
                    .debug_selector(|| "proxy-list".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(view.entries.is_empty(), |list| {
                        list.items_center()
                            .justify_center()
                            .child(crate::ui::empty_icon(cx))
                    })
                    .children(view.entries.iter().map(|proxy| {
                        let id = proxy.id.clone();
                        let selector = format!("proxy-item-{id}");
                        div()
                            .id(SharedString::from(selector.clone()))
                            .debug_selector(move || selector.clone())
                            .child(
                                ListItem::new(SharedString::from(format!("proxy-row-{id}")))
                                    .selected(view.editing.as_ref() == Some(&id))
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .items_center()
                                            .child(Icon::new(IconName::Globe))
                                            .child(
                                                v_flex()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .text_ellipsis()
                                                            .child(proxy.name.clone()),
                                                    )
                                                    .child(crate::typography::caption(
                                                        format!("{}:{}", proxy.host, proxy.port),
                                                        cx,
                                                    )),
                                            ),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.select_proxy_config(&id, window, cx)
                                    })),
                            )
                    })),
            );
        let path = view
            .front_id
            .as_ref()
            .and_then(|id| view.entries.iter().find(|p| &p.id == id))
            .map(|front| format!("RV → {} → {}", front.name, view.name.read(cx).value()));
        let form = Form::horizontal()
            .child(
                Field::new()
                    .label("名称")
                    .items_center()
                    .child(Input::new(&view.name).disabled(self.read_only)),
            )
            .child(
                Field::new().label("协议").items_center().child(
                    div()
                        .id("proxy-protocol")
                        .debug_selector(|| "proxy-protocol".into())
                        .child(
                            Select::new(&view.protocol_select)
                                .accessibility_label("代理协议")
                                .disabled(self.read_only),
                        ),
                ),
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
                Field::new()
                    .label("用户名")
                    .items_center()
                    .child(Input::new(&view.username).disabled(self.read_only)),
            )
            .child(
                Field::new()
                    .label("密码")
                    .items_center()
                    .child(Input::new(&view.password).disabled(self.read_only)),
            )
            .child(
                Field::new().label("前置代理").items_center().child(
                    div()
                        .id("proxy-front")
                        .debug_selector(|| "proxy-front".into())
                        .child(
                            Select::new(&view.front_select)
                                .accessibility_label("前置代理")
                                .disabled(self.read_only),
                        ),
                ),
            )
            .when_some(path, |form, path| {
                form.child(
                    Field::new()
                        .label("连接路径")
                        .child(crate::typography::caption(path, cx)),
                )
            })
            .child(
                Field::new().label("换 IP 方式").items_center().child(
                    Select::new(&view.rotation_select)
                        .accessibility_label("换 IP 方式")
                        .disabled(self.read_only),
                ),
            )
            .child(if view.password_rotation {
                Field::new()
                    .label("Session 标记")
                    .items_center()
                    .child(Input::new(&view.session_marker).disabled(self.read_only))
            } else {
                Field::new()
                    .label("换 IP URL")
                    .items_center()
                    .child(Input::new(&view.change_url).disabled(self.read_only))
            })
            .child(
                Field::new()
                    .label("IP 查询 URL")
                    .items_center()
                    .child(Input::new(&view.probe_url).disabled(self.read_only)),
            )
            .child(
                Field::new().label("出口检测").items_center().child(
                    v_flex()
                        .gap_2()
                        .child(
                            Button::new("proxy-inspect")
                                .label(if view.probing {
                                    "检测中…"
                                } else {
                                    "检测 IP"
                                })
                                .loading(view.probing)
                                .on_click(cx.listener(|this, _, _, cx| this.inspect_proxy(cx))),
                        )
                        .children(
                            view.observation
                                .as_ref()
                                .filter(|(key, _)| {
                                    self.proxy_form(cx)
                                        .is_ok_and(|p| proxy_form_key(&p) == *key)
                                })
                                .map(|(_, text)| crate::typography::caption(text.clone(), cx)),
                        ),
                ),
            )
            .child(
                Field::new().label("排他使用").items_center().child(
                    h_flex().justify_end().child(
                        Switch::new("proxy-exclusive")
                            .tooltip("控制以此代理为最终出口的任务并发")
                            .checked(view.exclusive)
                            .disabled(self.read_only)
                            .on_click(cx.listener(|this, value: &bool, _, cx| {
                                this.home.proxies.exclusive = *value;
                                cx.notify();
                            })),
                    ),
                ),
            );
        let detail = v_flex()
            .id("proxy-detail")
            .debug_selector(|| "proxy-detail".into())
            .flex_1()
            .min_w_0()
            .h_full()
            .min_h_0()
            .pl_4()
            .gap_4()
            .child(
                h_flex()
                    .items_center()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(crate::typography::label(
                                if view.editing.is_some() {
                                    view.entries
                                        .iter()
                                        .find(|p| Some(&p.id) == view.editing.as_ref())
                                        .map(|p| p.name.clone())
                                        .unwrap_or_default()
                                } else {
                                    "新增代理".into()
                                },
                                cx,
                            ))
                            .child(crate::typography::caption(view.protocol.label(), cx)),
                    )
                    .child(
                        Button::new("proxy-delete")
                            .debug_selector(|| "proxy-delete".into())
                            .ghost()
                            .icon(IconName::Delete)
                            .tooltip("删除代理")
                            .disabled(self.read_only || view.editing.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.delete_selected_proxy(window, cx)
                            })),
                    ),
            )
            .child(
                v_flex()
                    .id("proxy-form-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(form)
                    .children(
                        view.error
                            .as_ref()
                            .map(|error| crate::typography::caption(error.clone(), cx)),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .pt_4()
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
            );
        let (status, tooltip) = match &view.engine_info {
            Some(Ok((version, path))) => (format!("sing-box {version}"), format!("已安装：{path}")),
            Some(Err(error)) => ("sing-box 未就绪".into(), error.clone()),
            None => (
                "正在检测 sing-box…".into(),
                "检测外部程序，不连接代理".into(),
            ),
        };
        v_flex()
            .id("settings-proxies")
            .debug_selector(|| "settings-proxies".into())
            .w_full()
            .gap_3()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(crate::typography::label("代理", cx))
                    .child(
                        Button::new("proxy-engine-status")
                            .ghost()
                            .label(status)
                            .tooltip(tooltip)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_proxy_engine(cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("proxy-engine-settings")
                            .ghost()
                            .icon(IconName::Settings)
                            .tooltip("sing-box 程序路径")
                            .disabled(self.read_only)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.configure_proxy_engine(window, cx)
                            })),
                    ),
            )
            .child(
                GroupBox::new().outline().child(
                    h_flex()
                        .h(px(
                            (f32::from(window.viewport_size().height) - 180.).max(360.)
                        ))
                        .min_h_0()
                        .items_start()
                        .child(list)
                        .child(detail),
                ),
            )
            .into_any_element()
    }
}

// Exclude generated draft IDs so new, unsaved configurations can be tested.
fn proxy_form_key(proxy: &SocksProxy) -> Vec<u8> {
    use std::hash::{Hash, Hasher};
    let mut proxy = proxy.clone();
    proxy.id.clear();
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_vec(&proxy)
        .unwrap_or_default()
        .hash(&mut hash);
    hash.finish().to_le_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::{StorePaths, WorkspaceStore};
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
                    (
                        &app.home.proxies.change_url,
                        "https://example.test/change/fixture-secret",
                    ),
                    (&app.home.proxies.probe_url, "https://example.test/ip"),
                ] {
                    input.update(cx, |s, cx| s.set_value(value, window, cx));
                }
            })
        });
        app.update(cx, |app, cx| {
            app.home.proxies.protocol_select.update(cx, |_, cx| {
                cx.emit(gpui_component::select::SelectEvent::<Vec<String>>::Confirm(
                    Some("HTTP".into()),
                ))
            })
        });
        cx.run_until_parked();
        click(cx, "proxy-save");
        let entries = WorkspaceStore::open(&paths).unwrap().proxies().unwrap();
        assert_eq!(entries.len(), 1);
        let id = entries[0].id.clone();
        assert_eq!(entries[0].protocol, rv_core::workspace::ProxyProtocol::Http);
        assert_eq!(entries[0].password, "secret");
        assert_eq!(
            entries[0].ip.change_url.as_deref(),
            Some("https://example.test/change/fixture-secret")
        );
        assert_eq!(entries[0].ip.probe_url, "https://example.test/ip");
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
        click(cx, &format!("proxy-item-{id}"));
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
    #[gpui::test]
    fn list_selects_one_detail_and_front_reuses_existing_record(cx: &mut TestAppContext) {
        use rv_core::workspace::{ProxyMode, SocksProxy};
        let (app, cx, dir) = setup(cx, vec![]);
        let paths = StorePaths::in_dir(dir.path().into());
        let mut store = WorkspaceStore::open(&paths).unwrap();
        let front = SocksProxy {
            protocol: Default::default(),
            id: uuid::Uuid::new_v4().to_string(),
            name: "本地前置".into(),
            host: "127.0.0.1".into(),
            port: 12334,
            username: String::new(),
            password: String::new(),
            mode: ProxyMode::Shared,
            front_proxy_id: None,
            ip: Default::default(),
        };
        let mut exit = front.clone();
        exit.id = uuid::Uuid::new_v4().to_string();
        exit.name = "法国住宅代理".into();
        exit.host = "resi.proxyon.io".into();
        exit.port = 1111;
        store.save_proxy(&front).unwrap();
        store.save_proxy(&exit).unwrap();
        cx.update(|window, cx| app.update(cx, |app, cx| app.open_proxy_manager(window, cx)));
        cx.run_until_parked();
        click(cx, &format!("proxy-item-{}", exit.id));
        app.update(cx, |app, cx| {
            app.home.proxies.front_select.update(cx, |_, cx| {
                cx.emit(
                    gpui_component::select::SelectEvent::<Vec<super::ProxyChoice>>::Confirm(Some(
                        front.id.clone(),
                    )),
                )
            })
        });
        cx.run_until_parked();
        click(cx, "proxy-save");
        let entries = store.proxies().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries
                .iter()
                .find(|p| p.id == exit.id)
                .unwrap()
                .front_proxy_id
                .as_deref(),
            Some(front.id.as_str())
        );
        assert_eq!(
            entries.iter().find(|p| p.id == front.id).unwrap().port,
            12334
        );
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("proxy-list").is_some());
        assert!(cx.debug_bounds("proxy-detail").is_some());
        assert!(cx.debug_bounds("proxy-front").is_some());
    }
}
