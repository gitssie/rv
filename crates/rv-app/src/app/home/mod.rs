//! Home controller. Phone sessions, persistence, shared cards and App renderers
//! have explicit boundaries; no file import or manual App creation exists here.
mod avatar;
mod backup_info;
mod business_ui;
mod cards;
pub(in crate::app) mod control;
mod deletion;
mod devices;
mod editor;
mod handoff;
mod plugins;
mod proxies;
mod task_config;
mod token_expiry;
mod token_source;
use super::{AddressBookApp, Modal};
use gpui::*;
use gpui_component::input::{InputEvent, InputState, TextareaState};
pub(in crate::app) use proxies::ProxyChoice;
use rv_core::workspace::*;
use rv_core::{ConnectionId, StorePaths};
use std::collections::HashMap;

pub(super) struct PhoneWindow {
    pub view: WeakEntity<crate::session_window::SessionView>,
    pub window: WindowHandle<gpui_component::Root>,
}
#[derive(Default)]
pub(super) struct CardContext {
    pub expiry: TokenExpirations,
    pub proxy_name: Option<String>,
    pub execution: AppExecution,
    pub keepalive_enabled: bool,
}
pub(super) struct HomeState {
    pub apps: Vec<AppInstance>,
    pub icons: HashMap<String, std::sync::Arc<RenderImage>>,
    pub links: HashMap<AppId, Vec<BackupLink>>,
    pub previews: HashMap<AppId, rv_business::data::BusinessRow>,
    pub card_context: HashMap<AppId, CardContext>,
    pub available_tags: Vec<String>,
    pub(in crate::app) plugins: plugins::RenderRegistry,
    pub device_search: Entity<InputState>,
    pub online_only: bool,
    pub filter: AppFilter,
    pub tag: Option<String>,
    pub loading: bool,
    pub error: Option<String>,
    pub sequence: u64,
    pub sessions: HashMap<ConnectionId, PhoneWindow>,
    pub controls: HashMap<ConnectionId, control::PhoneControl>,
    pub control_loading: HashMap<ConnectionId, u64>,
    pub control_revision: u64,
    pub editing: Option<AppInstance>,
    pub token_expirations: TokenExpirations,
    pub proxies: proxies::ProxyView,
    pub deletion: Option<deletion::AppDeletion>,
    pub task_worker: Option<rv_business::tasks::TaskWorker>,
    pub change_subscription: Option<gpui::Task<()>>,
    pub jobs: std::collections::HashSet<AppId>,
    pub token_updates: Option<Task<()>>,
    pub product_syncs: std::collections::HashSet<(AppId, String)>,
    pub manual_syncs: std::collections::HashSet<(AppId,rv_business::Operation)>,
    pub backup_syncing: HashMap<AppId, (ConnectionId, EntityId)>,
    pub handoff_running: HashMap<AppId, EntityId>,
    pub avatar_preparing: std::collections::HashSet<AppId>,
    pub avatar_setting: std::collections::HashSet<AppId>,
    pub business: business_ui::BusinessView,
    pub task_config: task_config::TaskConfigView,
    pub component_dialog: Option<u8>,
    pub login_username: Entity<InputState>,
    pub login_password: Entity<InputState>,
    pub tags: Entity<InputState>,
    pub note: Entity<TextareaState>,
}
impl HomeState {
    pub fn new(paths: &StorePaths, window: &mut Window, cx: &mut Context<AddressBookApp>) -> Self {
        let initial = read_home(paths, "", AppFilter::All, None);
        let (apps, links, available_tags, error) = match initial {
            Ok((a, b, tags)) => (a, b, tags, None),
            Err(e) => (vec![], HashMap::new(), vec![], Some(e)),
        };
        Self {
            apps,
            links,
            previews: HashMap::new(),
            card_context: HashMap::new(),
            available_tags,
            error,
            plugins: plugins::RenderRegistry::default(),
            device_search: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("搜索设备…")
                    .clean_on_escape()
            }),
            login_username: cx.new(|cx| InputState::new(window, cx).placeholder("尚未同步用户名")),
            login_password: cx.new(|cx| InputState::new(window, cx).placeholder("尚未保存密码")),
            tags: cx.new(|cx| InputState::new(window, cx).placeholder("多个标签，用逗号分隔")),
            note: cx.new(|cx| TextareaState::new(window, cx).rows(3).placeholder("备注")),
            icons: HashMap::new(),
            online_only: false,
            filter: AppFilter::All,
            tag: None,
            loading: false,
            sequence: 0,
            sessions: HashMap::new(),
            controls: HashMap::new(),
            control_loading: HashMap::new(),
            control_revision: 0,
            editing: None,
            token_expirations: Default::default(),
            proxies: proxies::ProxyView::new(window, cx),
            deletion: None,
            task_worker: None,
            change_subscription: None,
            jobs: Default::default(),
            token_updates: None,
            product_syncs: Default::default(),
            manual_syncs: Default::default(),
            backup_syncing: Default::default(),
            handoff_running: Default::default(),
            avatar_preparing: Default::default(),
            avatar_setting: Default::default(),
            business: business_ui::BusinessView::new(window, cx),
            task_config: task_config::TaskConfigView::new(window, cx),
            component_dialog: None,
        }
    }
}
type HomeData = (
    Vec<AppInstance>,
    HashMap<AppId, Vec<BackupLink>>,
    Vec<String>,
);
fn read_home(
    paths: &StorePaths,
    text: &str,
    filter: AppFilter,
    tag: Option<&str>,
) -> Result<HomeData, String> {
    let store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    let apps = store
        .list(&AppQuery {
            text,
            filter,
            tag,
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
    let links = apps
        .iter()
        .map(|a| store.backups(&a.id).map(|v| (a.id.clone(), v)))
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    Ok((apps, links, store.tags().map_err(|e| e.to_string())?))
}
impl AddressBookApp {
    pub(crate) fn remember_app_icon(
        &mut self,
        bundle: String,
        image: std::sync::Arc<RenderImage>,
        cx: &mut Context<Self>,
    ) {
        self.home.icons.insert(bundle, image);
        cx.notify();
    }
    pub(in crate::app) fn app_avatar(&self, bundle: &str) -> gpui_component::avatar::Avatar {
        let avatar = gpui_component::avatar::Avatar::new()
            .placeholder(crate::assets::FunctionIcon::Smartphone);
        match self.home.icons.get(bundle) {
            Some(image) => avatar.src(image.clone()),
            None => avatar,
        }
    }
    pub(in crate::app) fn virtual_app_avatar(
        &self,
        app: &AppInstance,
    ) -> gpui_component::avatar::Avatar {
        match self
            .home
            .plugins
            .for_bundle(&app.bundle_id)
            .avatar_path(app, self.book.paths())
        {
            Some(path) => gpui_component::avatar::Avatar::new().src(path),
            None => self.app_avatar(&app.bundle_id),
        }
    }
    fn reload_token_expirations(&mut self, id: &AppId) {
        match WorkspaceStore::open(self.book.paths()).and_then(|s| s.auth(id)) {
            Ok(auth) => {
                self.home.token_expirations =
                    auth.map(|a| a.token_expirations()).unwrap_or_default();
                self.home.card_context.entry(id.clone()).or_default().expiry =
                    self.home.token_expirations;
            }
            Err(error) => {
                tracing::error!(app_id=id.as_str(),error=%error,"Token expiry read failed");
                self.home.token_expirations = Default::default();
            }
        }
    }
    pub(super) fn subscribe_home(&mut self, cx: &mut Context<Self>) {
        self.subscribe_business(cx);
        self.subscribe_proxy_form(cx);
        self.reload_home(cx);
        self._subscriptions.push(
            cx.subscribe(&self.search, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.reload_home(cx);
                }
            }),
        );
        self._subscriptions.push(
            cx.subscribe(&self.home.device_search, |_, _, _: &InputEvent, cx| {
                cx.notify()
            }),
        );
    }
    pub(super) fn reload_home(&mut self, cx: &mut Context<Self>) {
        self.home.sequence += 1;
        let sequence = self.home.sequence;
        let paths = self.book.paths().clone();
        let cached_icons: std::collections::HashSet<_> = self.home.icons.keys().cloned().collect();
        let query = self.query(cx);
        let filter = self.home.filter;
        let tag = self.home.tag.clone();
        self.home.loading = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let data = read_home(&paths, &query, filter, tag.as_deref())?;
                    let store = WorkspaceStore::open(&paths).map_err(|e| e.to_string())?;
                    let proxy_names: HashMap<_, _> = store
                        .proxies()
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .map(|proxy| (proxy.id, proxy.name))
                        .collect();
                    let mut card_context = HashMap::new();
                    for app in &data.0 {
                        if rv_business::strategy(&app.bundle_id).is_none() {
                            continue;
                        }
                        let expiry = store
                            .auth(&app.id)
                            .map_err(|e| e.to_string())?
                            .map(|auth| auth.token_expirations())
                            .unwrap_or_default();
                        let proxy_name = store
                            .proxy_binding(&app.id)
                            .map_err(|e| e.to_string())?
                            .and_then(|id| proxy_names.get(&id).cloned());
                        card_context.insert(
                            app.id.clone(),
                            CardContext {
                                expiry,
                                proxy_name,
                                execution: store.execution(&app.id).map_err(|e| e.to_string())?,
                                keepalive_enabled: store.configured_tasks(&app.id).map_err(|e| e.to_string())?
                                    .iter().any(|task| task.kind == "refresh" && task.enabled),
                            },
                        );
                    }
                    let mut previews = HashMap::new();
                    for app in &data.0 {
                        if rv_business::strategy(&app.bundle_id).is_none() {
                            continue;
                        }
                        if let Ok(mut page) = rv_business::data::read_data(
                            &paths,
                            &app.id,
                            rv_business::data::Dataset::Products,
                            "",
                            0,
                        ) {
                            if let Some(mut row) = page.rows.drain(..).next() {
                                rv_business::assets::local(&paths, std::slice::from_mut(&mut row));
                                previews.insert(app.id.clone(), row);
                            }
                        }
                    }
                    let bundles: std::collections::BTreeSet<_> =
                        data.0.iter().map(|app| app.bundle_id.clone()).collect();
                    let icons: HashMap<_, _> = bundles
                        .into_iter()
                        .filter(|bundle| !cached_icons.contains(bundle))
                        .filter_map(|bundle| {
                            crate::app_icons::load(&paths, &bundle).map(|image| (bundle, image))
                        })
                        .collect();
                    Ok::<_, String>((data, previews, icons, card_context))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.home.sequence != sequence {
                    return;
                }
                this.home.loading = false;
                match result {
                    Ok(((apps, links, tags), previews, icons, card_context)) => {
                        this.home.card_context = card_context;
                        this.home.icons.extend(icons);
                        this.home.previews = previews;
                        this.home.apps = apps;
                        this.home.links = links;
                        this.home.available_tags = tags;
                        this.home.error = None;
                        let ids: Vec<_> = this
                            .home
                            .apps
                            .iter()
                            .map(|app| app.bundle_id.clone())
                            .collect();
                        for control in this.home.controls.values() {
                            control.view.update(cx, |view, cx| {
                                view.request_home_app_icons(ids.clone(), cx)
                            });
                        }
                    }
                    Err(error) => {
                        tracing::error!(error=%error, "Virtual App list read failed");
                        this.home.error = Some("读取账号列表失败，请查看日志".into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn open_app_card(&mut self, id: AppId, window: &mut Window, cx: &mut Context<Self>) {
        let (app, login) = match WorkspaceStore::open(self.book.paths())
            .and_then(|store| Ok((store.load(&id)?, store.login_credentials(&id)?))) {
            Ok(data) => data,
            Err(error) => {
                self.set_status(format!("读取失败：{error}"), cx);
                return;
            }
        };
        let username = login.as_ref().map(|login|login.username.clone())
            .or_else(|| rv_business::shop::ShopProfile::from_app(&app).and_then(|shop|shop.login))
            .unwrap_or_default();
        let password = login.and_then(|login|login.password).unwrap_or_default();
        self.home.login_username.update(cx, |state,cx| state.set_value(username,window,cx));
        self.home.login_password.update(cx, |state,cx| state.set_value(password,window,cx));
        self.home
            .tags
            .update(cx, |s, cx| s.set_value(app.tags.join(", "), window, cx));
        self.home
            .note
            .update(cx, |s, cx| s.set_value(app.note.clone(), window, cx));
        match WorkspaceStore::open(self.book.paths())
            .and_then(|store| Ok((store.proxies()?, store.proxy_binding(&id)?)))
        {
            Ok((entries, binding)) => {
                self.home.proxies.entries = entries;
                self.home.proxies.binding = binding;
                self.sync_app_proxy_binding(window, cx);
            }
            Err(error) => {
                self.set_status(format!("读取代理失败：{error}"), cx);
                return;
            }
        }
        self.reload_token_expirations(&id);
        self.home.editing = Some(app);
        self.modal = Modal::App;
        cx.notify();
    }
    pub(super) fn open_app_detail(
        &mut self,
        id: AppId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_app_card(id, window, cx);
        if self.home.editing.as_ref().is_some_and(|app| {
            !self
                .home
                .plugins
                .for_bundle(&app.bundle_id)
                .datasets()
                .is_empty()
        }) {
            self.modal = Modal::Shop;
            self.reset_business_view(window, cx);
        }
    }
    pub(super) fn save_app_card(&mut self, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        let Some(mut app) = self.home.editing.clone() else {
            return;
        };
        app.tags = super::parse_labels(&self.home.tags.read(cx).value().replace('，', ","));
        app.note = self.home.note.read(cx).value().to_string();
        let login = if rv_business::strategy(&app.bundle_id).is_some() {
            let username = self.home.login_username.read(cx).value().to_string();
            let password = self.home.login_password.read(cx).value().to_string();
            if username.is_empty() && password.is_empty() { None } else {
                Some(AppLoginCredentials { username, password: (!password.is_empty()).then_some(password) })
            }
        } else { None };
        match WorkspaceStore::open(self.book.paths())
            .and_then(|mut s| s.save_with_proxy_and_login(&app, self.home.proxies.binding.as_deref(), login.as_ref()))
        {
            Ok(_) => {
                self.modal = Modal::None;
                self.home.editing = None;
                self.status = "Ready".into();
                self.reload_home(cx);
            }
            Err(e) => self.set_status(format!("保存失败：{e}"), cx),
        }
    }
    pub(crate) fn import_app_profile(
        &mut self,
        id: &AppId,
        profile: &rv_core::workspace::ShadowAppState,
        source: Option<rv_core::ConnectionId>,
        owner: EntityId,
        cx: &mut Context<Self>,
    ) -> bool {
        if self
            .home
            .backup_syncing
            .get(id)
            .is_some_and(|(_, current)| *current != owner)
        {
            return false;
        }
        self.home.backup_syncing.remove(id);
        if self.read_only {
            return false;
        }
        let mut imported = false;
        match WorkspaceStore::open(self.book.paths())
            .and_then(|mut store| store.import_app_state(id, source, profile))
        {
            Ok(()) => {
                imported = true;
                match rv_business::tasks::initialize(self.book.paths(), id) {
                    Ok(()) => {
                        let bound = WorkspaceStore::open(self.book.paths())
                            .and_then(|s| s.proxy_binding(id))
                            .ok()
                            .flatten()
                            .is_some();
                        self.set_status(
                            if bound {
                                "账号备份已保存，任务按现有计划执行"
                            } else {
                                "账号资料已保存，请编辑虚拟 App 关联代理，任务随后自动执行"
                            },
                            cx,
                        );
                    }
                    Err(error) => {
                        tracing::error!(app_id=id.as_str(),error=%error,"Task workflow initialization failed");
                        self.set_status("账号资料已保存，初始化任务创建失败，请查看日志", cx)
                    }
                }
            }
            Err(error) => {
                let message = self.task_error_message("保存账号资料", None, &error);
                self.set_status(message, cx);
            }
        }
        self.reload_chat(cx);
        self.refresh_editing_baseline(id);
        self.reload_home(cx);
        imported
    }
    fn refresh_editing_baseline(&mut self, id: &AppId) {
        if self.home.editing.as_ref().is_some_and(|app| app.id == *id) {
            self.reload_token_expirations(id);
            if let Ok(latest) =
                WorkspaceStore::open(self.book.paths()).and_then(|store| store.load(id))
            {
                // Input entities retain the user's unsaved tags and note.
                self.home.editing = Some(latest);
            }
        }
    }
    pub(super) fn confirm_backup_profile_sync(
        &mut self,
        id: AppId,
        link: BackupLink,
        bundle: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only || self.home.backup_syncing.contains_key(&id) {
            return;
        }
        let owner = cx.entity();
        let source = self
            .backup_source_labels(std::slice::from_ref(&link))
            .join("");
        crate::ui::confirm_backup_sync(source, window, cx, move |_, cx| {
            owner.update(cx, |this, cx| {
                this.read_backup_profile(id.clone(), link.clone(), bundle.clone(), cx);
            });
        });
    }
    pub(crate) fn begin_backup_sync(
        &mut self,
        id: &AppId,
        source: ConnectionId,
        owner: EntityId,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.read_only || self.home.backup_syncing.contains_key(id) {
            return false;
        }
        self.home.backup_syncing.insert(id.clone(), (source, owner));
        self.set_status("正在从手机读取账号备份…", cx);
        true
    }
    pub(crate) fn backup_sync_failed(
        &mut self,
        id: &AppId,
        owner: EntityId,
        cx: &mut Context<Self>,
    ) {
        if self
            .home
            .backup_syncing
            .get(id)
            .is_some_and(|(_, current)| *current != owner)
        {
            return;
        }
        self.home.backup_syncing.remove(id);
        self.set_status("账号备份同步失败，请查看日志", cx);
    }
    pub(crate) fn backup_sync_disconnected(&mut self, owner: EntityId, cx: &mut Context<Self>) {
        let interrupted: Vec<_> = self
            .home
            .handoff_running
            .iter()
            .filter(|(_, source)| **source == owner)
            .map(|(id, _)| id.clone())
            .collect();
        let refresh_owner = !interrupted.is_empty();
        for id in interrupted {
            self.home.handoff_running.remove(&id);
            self.set_status("手机连接已结束，交接状态保留，RV 任务保持停止", cx);
        }
        if refresh_owner {
            self.reload_home(cx);
        }
        let count = self.home.backup_syncing.len();
        self.home
            .backup_syncing
            .retain(|_, (_, source_owner)| *source_owner != owner);
        if self.home.backup_syncing.len() != count {
            self.set_status("手机已断开，账号备份同步未完成", cx);
        }
    }

    fn read_backup_profile(
        &mut self,
        id: AppId,
        link: BackupLink,
        bundle: String,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        let Some(view) = self
            .home
            .controls
            .get(&link.device_id)
            .map(|s| s.view.clone())
        else {
            self.set_status("请先连接来源手机，再读取账号资料", cx);
            return;
        };
        if self.home.backup_syncing.contains_key(&id) {
            return;
        }
        let started = view.update(cx, |view, cx| {
            view.read_home_profile(id.clone(), bundle, link.backup_id, cx)
        });
        if started {
            self.begin_backup_sync(&id, link.device_id, view.entity_id(), cx);
        } else {
            self.set_status("请先连接来源手机并完成当前备份操作", cx);
        }
    }
    pub(super) fn run_shop_sync(
        &mut self,id:AppId,operation:rv_business::Operation,cx:&mut Context<Self>,
    ) {
        if self.read_only || !self.home.manual_syncs.insert((id.clone(),operation)){return;}
        self.home.jobs.insert(id.clone());
        let paths=self.book.paths().clone();
        cx.notify();
        cx.spawn(async move |this,cx| {
            let result=rv_business::shop_sync::synchronize(paths,id.clone(),operation).await;
            let _=this.update(cx,|this,cx| {
                this.home.manual_syncs.remove(&(id.clone(),operation));
                if !this.home.manual_syncs.iter().any(|(app,_)|app==&id)&&!this.home.avatar_setting.contains(&id){this.home.jobs.remove(&id);}
                match result {
                    Ok(_)=>this.set_status(format!("{}完成",operation.label()),cx),
                    Err(error)=>{
                        tracing::error!(app_id=%id.as_str(),operation=operation.key(),%error,"Manual shop sync failed");
                        this.set_error_status(format!("{}失败：{error}",operation.label()),cx);
                    }
                }
                cx.notify();
            });
        }).detach();
    }
    pub(super) fn refresh_card_product(&mut self, id: AppId, product_id: String, cx: &mut Context<Self>) {
        if self.read_only || !self.home.product_syncs.insert((id.clone(), product_id.clone())) { return; }
        let paths = self.book.paths().clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = rv_business::shop_sync::refresh_product(paths, id.clone(), product_id.clone()).await;
            let _ = this.update(cx, |this, cx| {
                this.home.product_syncs.remove(&(id.clone(), product_id.clone()));
                match result {
                    Ok(_) => this.set_status("商品刷新完成", cx),
                    Err(error) => {
                        tracing::error!(app_id=%id.as_str(), %product_id, %error, "Manual product refresh failed");
                        this.set_error_status(format!("商品刷新失败：{error}"), cx);
                    }
                }
                cx.notify();
            });
        }).detach();
    }
    pub(super) fn change_app(
        &mut self,
        id: AppId,
        change: impl FnOnce(&mut AppInstance),
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        let result = WorkspaceStore::open(self.book.paths()).and_then(|mut s| {
            let mut app = s.load(&id)?;
            change(&mut app);
            s.save(&app)
        });
        match result {
            Ok(_) => self.reload_home(cx),
            Err(e) => self.set_status(format!("保存失败：{e}"), cx),
        }
    }
    pub(crate) fn register_phone_window(
        &mut self,
        id: ConnectionId,
        view: WeakEntity<crate::session_window::SessionView>,
        window: WindowHandle<gpui_component::Root>,
        cx: &mut Context<Self>,
    ) {
        let released = view.entity_id();
        if let Some(entity) = view.upgrade() {
            self._subscriptions
                .push(cx.observe_release(&entity, move |this, _, cx| {
                    if !this
                        .home
                        .sessions
                        .get(&id)
                        .is_some_and(|s| s.view.entity_id() == released)
                    {
                        return;
                    }
                    this.home.sessions.remove(&id);

                    cx.notify();
                }));
        }
        self.home.sessions.insert(id, PhoneWindow { view, window });
        cx.notify();
    }
    pub(super) fn focus_phone(&mut self, id: ConnectionId, cx: &mut Context<Self>) -> bool {
        if let Some(session) = self.home.sessions.get(&id)
            && session.view.upgrade().is_some()
            && session
                .window
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return true;
        }
        self.home.sessions.remove(&id);
        false
    }
    pub(super) fn show_phone_backups(
        &mut self,
        id: ConnectionId,
        bundle: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(control) = self.home.controls.get(&id) else {
            self.set_status("ZXTouch 正在连接，请稍后重试", cx);
            return;
        };
        control.view.update(cx, |view, cx| {
            view.show_home_environment(bundle, window, cx)
        });
        self.modal = Modal::PhoneControl(id);
        cx.notify();
    }
    pub(super) fn phone_online(&self, id: ConnectionId, cx: &App) -> bool {
        self.home
            .controls
            .get(&id)
            .is_some_and(|s| s.view.read(cx).home_connected())
    }
    pub(crate) fn phone_changed(
        &mut self,
        id: ConnectionId,
        foreground: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(connection) = self.book.get_mut(id)
            && connection.current_bundle_id != foreground
        {
            connection.current_bundle_id = foreground;
            self.persist();
        }
        cx.notify();
    }
}
