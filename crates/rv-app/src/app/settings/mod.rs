//! Full-window settings use the component library's Settings hierarchy.
mod apple_accounts;
mod general;
mod tasks;
use super::{AddressBookApp, Modal};
use gpui::*;
use gpui_component::{
    IconName,
    button::{Button, ButtonVariants},
    h_flex,
    select::{SelectEvent, SelectState},
    setting::{SelectIndex, SettingGroup, SettingItem, SettingPage, Settings},
    v_flex,
};
use rv_core::{ScaleMode, ThemePref};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SettingsPage {
    General,
    Proxies,
    AppleAccounts,
    Tasks,
}
pub(super) struct SettingsState {
    pub page: Option<SettingsPage>,
    pub revision: u64,
    apple_accounts: apple_accounts::AppleAccountsView,
    tasks: tasks::TasksView,
    pub theme: Entity<SelectState<Vec<String>>>,
    pub scale: Entity<SelectState<Vec<String>>>,
    pub menu_key: Entity<SelectState<Vec<String>>>,
}
impl SettingsState {
    pub fn new(window: &mut Window, cx: &mut Context<AddressBookApp>) -> Self {
        Self {
            page: None,
            apple_accounts: apple_accounts::AppleAccountsView::new(window, cx),
            tasks: tasks::TasksView::new(window, cx),
            revision: 0,
            theme: cx.new(|cx| {
                SelectState::new(
                    vec!["跟随系统".into(), "浅色".into(), "深色".into()],
                    None,
                    window,
                    cx,
                )
            }),
            scale: cx.new(|cx| {
                SelectState::new(
                    vec!["适应窗口".into(), "100%".into(), "拉伸".into()],
                    None,
                    window,
                    cx,
                )
            }),
            menu_key: cx.new(|cx| {
                SelectState::new(
                    vec!["F8".into(), "F7".into(), "F12".into()],
                    None,
                    window,
                    cx,
                )
            }),
        }
    }
}
impl AddressBookApp {
    pub(super) fn render_settings_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .h_full()
            .flex_1()
            .min_w_0()
            .px_2()
            .items_center()
            .child(
                Button::new("settings-back")
                    .occlude()
                    .debug_selector(|| "settings-back".into())
                    .ghost()
                    .icon(IconName::ArrowLeft)
                    .tooltip("返回主界面")
                    .accessibility_label("返回主界面")
                    .on_click(cx.listener(|this, _, window, cx| this.close_settings(window, cx))),
            )
    }
    pub(super) fn subscribe_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.subscribe_apple_accounts(window, cx);
        self.subscribe_task_list(window, cx);
        self._subscriptions.push(cx.subscribe_in(
            &self.settings.theme,
            window,
            |this, _, event: &SelectEvent<Vec<String>>, window, cx| {
                let SelectEvent::Confirm(Some(value)) = event else {
                    return;
                };
                if this.read_only {
                    return;
                }
                this.book.prefs_mut().theme = match value.as_str() {
                    "浅色" => ThemePref::Light,
                    "深色" => ThemePref::Dark,
                    _ => ThemePref::System,
                };
                this.persist();
                crate::theme::apply(this.book.prefs().theme, window, cx);
                cx.notify();
            },
        ));
        self._subscriptions.push(cx.subscribe(
            &self.settings.scale,
            |this, _, event: &SelectEvent<Vec<String>>, cx| {
                let SelectEvent::Confirm(Some(value)) = event else {
                    return;
                };
                if this.read_only {
                    return;
                }
                this.book.prefs_mut().default_scale = match value.as_str() {
                    "100%" => ScaleMode::Actual,
                    "拉伸" => ScaleMode::Stretch,
                    _ => ScaleMode::Fit,
                };
                this.persist();
                cx.notify();
            },
        ));
        self._subscriptions.push(cx.subscribe(
            &self.settings.menu_key,
            |this, _, event: &SelectEvent<Vec<String>>, cx| {
                let SelectEvent::Confirm(Some(value)) = event else {
                    return;
                };
                if this.read_only {
                    return;
                }
                this.book.prefs_mut().menu_key = value.to_ascii_lowercase();
                this.persist();
                cx.notify();
            },
        ));
        self._subscriptions.push(cx.subscribe(
            &self.home.proxies.rotation_select,
            |this, _, event: &SelectEvent<Vec<String>>, cx| {
                if !this.read_only {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        this.home.proxies.password_rotation = value == "密码 Session";
                        cx.notify();
                    }
                }
            },
        ));
        self._subscriptions.push(cx.subscribe(
            &self.home.proxies.protocol_select,
            |this, _, event: &SelectEvent<Vec<String>>, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    this.set_proxy_protocol(value, cx);
                }
            },
        ));
        self._subscriptions.push(cx.subscribe_in(
            &self.home.proxies.front_select,
            window,
            |this, _, event: &SelectEvent<Vec<super::home::ProxyChoice>>, _window, cx| {
                if let SelectEvent::Confirm(Some(id)) = event {
                    this.set_proxy_front(id, cx);
                }
            },
        ));
        self._subscriptions.push(cx.subscribe(
            &self.home.proxies.binding_select,
            |this, _, event: &SelectEvent<Vec<super::home::ProxyChoice>>, cx| {
                if let SelectEvent::Confirm(Some(id)) = event {
                    if this.read_only || !matches!(this.modal, Modal::App) {
                        return;
                    }
                    this.home.proxies.binding = if id.is_empty() {
                        None
                    } else {
                        Some(id.clone())
                    };
                    cx.notify();
                }
            },
        ));
    }
    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.credential_busy
            || self
                .home
                .deletion
                .as_ref()
                .is_some_and(|deletion| deletion.busy)
        {
            return;
        }
        self.close_modal(cx);
        let prefs = self.book.prefs();
        let theme = match prefs.theme {
            ThemePref::System => "跟随系统",
            ThemePref::Light => "浅色",
            ThemePref::Dark => "深色",
        }
        .to_owned();
        let scale = match prefs.default_scale {
            ScaleMode::Fit => "适应窗口",
            ScaleMode::Actual => "100%",
            ScaleMode::Stretch => "拉伸",
        }
        .to_owned();
        let key = prefs.menu_key.to_ascii_uppercase();
        for (state, value) in [
            (&self.settings.theme, theme),
            (&self.settings.scale, scale),
            (&self.settings.menu_key, key),
        ] {
            state.update(cx, |s, cx| s.set_selected_value(&value, window, cx));
        }
        self.load_proxy_manager(window, cx);
        self.load_apple_accounts(cx);
        self.settings.page = Some(SettingsPage::General);
        self.settings.revision = self.settings.revision.wrapping_add(1);
        self.focus.focus(window, cx);
        cx.notify();
    }
    #[cfg(test)]
    pub(super) fn select_settings_page(
        &mut self,
        page: SettingsPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings.page == Some(page) {
            return;
        }
        if page == SettingsPage::Proxies {
            self.open_proxy_manager(window, cx);
        } else {
            self.settings.page = Some(page);
            self.settings.revision = self.settings.revision.wrapping_add(1);
            self.focus.focus(window, cx);
            cx.notify();
        }
    }
    pub(super) fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.page = None;
        self.clear_proxy_form(window, cx);
        self.focus.focus(window, cx);
        cx.notify();
    }
    fn render_settings_content(
        &mut self,
        page: SettingsPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.settings.page != Some(page) {
            self.settings.page = Some(page);
            if page == SettingsPage::AppleAccounts {
                self.load_apple_accounts(cx);
            }
            if page == SettingsPage::Tasks {
                self.load_task_list(cx);
            }
            self.focus.focus(window, cx);
        }
        match page {
            SettingsPage::General => self.render_general_settings(cx),
            SettingsPage::Proxies => self.render_proxy_manager(window, cx),
            SettingsPage::AppleAccounts => self.render_apple_accounts(cx),
            SettingsPage::Tasks => self.render_task_list(cx),
        }
    }
    pub(super) fn render_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let page = self.settings.page.unwrap_or(SettingsPage::General);
        let owner = cx.entity();
        let general = SettingPage::new("通用")
            .icon(IconName::Settings)
            .resettable(false)
            .header_style(&StyleRefinement::default().hidden())
            .group(SettingGroup::new().item(SettingItem::render({
                let owner = owner.clone();
                move |_, window, cx| {
                    owner.update(cx, |this, cx| {
                        this.render_settings_content(SettingsPage::General, window, cx)
                    })
                }
            })));
        let proxies = SettingPage::new("代理")
            .icon(IconName::Globe)
            .resettable(false)
            .header_style(&StyleRefinement::default().hidden())
            .group(SettingGroup::new().item(SettingItem::render({
                let owner = owner.clone();
                move |_, window, cx| {
                    owner.update(cx, |this, cx| {
                        this.render_settings_content(SettingsPage::Proxies, window, cx)
                    })
                }
            })));
        let tasks = SettingPage::new("任务")
            .icon(IconName::Redo)
            .resettable(false)
            .header_style(&StyleRefinement::default().hidden())
            .group(SettingGroup::new().item(SettingItem::render({
                let owner = owner.clone();
                move |_, window, cx| {
                    owner.update(cx, |this, cx| {
                        this.render_settings_content(SettingsPage::Tasks, window, cx)
                    })
                }
            })));
        let accounts = SettingPage::new("Apple ID")
            .icon(IconName::User)
            .resettable(false)
            .header_style(&StyleRefinement::default().hidden())
            .group(
                SettingGroup::new().item(SettingItem::render(move |_, window, cx| {
                    owner.update(cx, |this, cx| {
                        this.render_settings_content(SettingsPage::AppleAccounts, window, cx)
                    })
                })),
            );
        v_flex()
            .id("settings-page")
            .debug_selector(|| "settings-page".into())
            .flex_1()
            .min_h_0()
            .w_full()
            .bg(crate::theme::surface(cx))
            .child(
                Settings::new(format!("rv-settings-{}", self.settings.revision))
                    .sidebar_style(&StyleRefinement::default().bg(crate::theme::sidebar(cx)))
                    .header_style(&StyleRefinement::default().hidden())
                    .default_selected_index(SelectIndex {
                        page_ix: match page {
                            SettingsPage::General => 0,
                            SettingsPage::Proxies => 1,
                            SettingsPage::AppleAccounts => 2,
                            SettingsPage::Tasks => 3,
                        },
                        group_ix: None,
                    })
                    .page(general)
                    .page(proxies)
                    .page(accounts)
                    .page(tasks),
            )
            .into_any_element()
    }
}
