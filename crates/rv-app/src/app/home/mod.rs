//! Home controller. Phone sessions, persistence, shared cards and App renderers
//! have explicit boundaries; no file import or manual App creation exists here.
mod business_ui;
mod cards;
pub(in crate::app) mod control;
mod deletion;
mod devices;
mod editor;
mod plugins;
mod proxies;
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
pub(super) struct HomeState {
    pub apps: Vec<AppInstance>,
    pub icons: HashMap<String, std::sync::Arc<RenderImage>>,
    pub links: HashMap<AppId, Vec<BackupLink>>,
    pub previews: HashMap<AppId, rv_business::data::BusinessRow>,
    pub available_tags: Vec<String>,
    pub(in crate::app) plugins: plugins::RenderRegistry,
    pub device_search: Entity<InputState>,
    pub online_only: bool,
    pub filter: AppFilter,
    pub sort: AppSort,
    pub tag: Option<String>,
    pub loading: bool,
    pub error: Option<String>,
    pub sequence: u64,
    pub sessions: HashMap<ConnectionId, PhoneWindow>,
    pub controls: HashMap<ConnectionId, control::PhoneControl>,
    pub control_loading: HashMap<ConnectionId, u64>,
    pub control_revision: u64,
    pub editing: Option<AppInstance>,
    pub proxies: proxies::ProxyView,
    pub deletion: Option<deletion::AppDeletion>,
    pub tasks: Vec<AppTask>,
    pub jobs: std::collections::HashSet<AppId>,
    pub business: business_ui::BusinessView,
    pub component_dialog: Option<u8>,
    pub title: Entity<InputState>,
    pub tags: Entity<InputState>,
    pub note: Entity<TextareaState>,
}
impl HomeState {
    pub fn new(paths: &StorePaths, window: &mut Window, cx: &mut Context<AddressBookApp>) -> Self {
        let initial = read_home(paths, "", AppFilter::All, AppSort::Updated, None);
        let (apps, links, available_tags, error) = match initial {
            Ok((a, b, tags)) => (a, b, tags, None),
            Err(e) => (vec![], HashMap::new(), vec![], Some(e)),
        };
        Self {
            apps,
            links,
            previews: HashMap::new(),
            available_tags,
            error,
            plugins: plugins::RenderRegistry::default(),
            device_search: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("搜索设备…")
                    .clean_on_escape()
            }),
            title: cx.new(|cx| InputState::new(window, cx).placeholder("名称")),
            tags: cx.new(|cx| InputState::new(window, cx).placeholder("多个标签，用逗号分隔")),
            note: cx.new(|cx| TextareaState::new(window, cx).rows(3).placeholder("备注")),
            icons: HashMap::new(),
            online_only: false,
            filter: AppFilter::All,
            sort: AppSort::Updated,
            tag: None,
            loading: false,
            sequence: 0,
            sessions: HashMap::new(),
            controls: HashMap::new(),
            control_loading: HashMap::new(),
            control_revision: 0,
            editing: None,
            proxies: proxies::ProxyView::new(window, cx),
            deletion: None,
            tasks: vec![],
            jobs: Default::default(),
            business: business_ui::BusinessView::new(window, cx),
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
    sort: AppSort,
    tag: Option<&str>,
) -> Result<HomeData, String> {
    let store = WorkspaceStore::open(paths).map_err(|e| e.to_string())?;
    let apps = store
        .list(&AppQuery {
            text,
            filter,
            sort,
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
    pub(super) fn subscribe_home(&mut self, cx: &mut Context<Self>) {
        self.subscribe_business(cx);
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
        let sort = self.home.sort;
        let tag = self.home.tag.clone();
        self.home.loading = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let data = read_home(&paths, &query, filter, sort, tag.as_deref())?;
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
                    Ok::<_, String>((data, previews, icons))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.home.sequence != sequence {
                    return;
                }
                this.home.loading = false;
                match result {
                    Ok(((apps, links, tags), previews, icons)) => {
                        this.home.icons.extend(icons);
                        this.home.previews = previews;
                        this.load_preview_images(sequence, cx);
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
                    Err(e) => this.home.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn load_preview_images(&mut self, sequence: u64, cx: &mut Context<Self>) {
        let paths = self.book.paths().clone();
        let entries: Vec<_> = self
            .home
            .previews
            .iter()
            .map(|(id, row)| (id.clone(), row.clone()))
            .collect();
        cx.spawn(async move |this, cx| {
            let previews = cx
                .background_executor()
                .spawn(async move {
                    entries
                        .into_iter()
                        .map(|(id, mut row)| {
                            rv_business::assets::hydrate(
                                &paths,
                                &id,
                                std::slice::from_mut(&mut row),
                            );
                            (id, row)
                        })
                        .collect()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.home.sequence == sequence {
                    this.home.previews = previews;
                    cx.notify();
                }
            });
        })
        .detach();
    }
    pub(super) fn open_app_card(&mut self, id: AppId, window: &mut Window, cx: &mut Context<Self>) {
        let app = match WorkspaceStore::open(self.book.paths()).and_then(|s| s.load(&id)) {
            Ok(app) => app,
            Err(error) => {
                self.set_status(format!("读取失败：{error}"), cx);
                return;
            }
        };
        self.home
            .title
            .update(cx, |s, cx| s.set_value(app.title.clone(), window, cx));
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
        self.home.editing = Some(app);
        self.home.tasks = WorkspaceStore::open(self.book.paths())
            .and_then(|store| store.tasks(&id))
            .unwrap_or_default();
        self.modal = Modal::App;
        cx.notify();
    }
    pub(super) fn open_app_detail(
        &mut self,
        id: AppId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.read_only {
            match WorkspaceStore::open(self.book.paths())
                .and_then(|store| store.record_use(&id, super::unix_now()))
            {
                Ok(()) => self.reload_home(cx),
                Err(error) => self.set_status(format!("记录最近使用失败：{error}"), cx),
            }
        }
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
        app.title = self.home.title.read(cx).value().to_string();
        app.tags = super::parse_labels(&self.home.tags.read(cx).value().replace('，', ","));
        app.note = self.home.note.read(cx).value().to_string();
        match WorkspaceStore::open(self.book.paths())
            .and_then(|mut s| s.save_with_proxy(&app, self.home.proxies.binding.as_deref()))
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
        profile: &serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        match WorkspaceStore::open(self.book.paths())
            .and_then(|mut store| store.import_profile(id, profile))
        {
            Ok(()) => self.set_status("账号资料已保存，可脱离手机执行任务", cx),
            Err(error) => self.set_status(format!("账号资料保存失败：{error}"), cx),
        }
        self.reload_chat(cx);
        self.refresh_editing_baseline(id);
        self.reload_home(cx);
    }
    fn refresh_editing_baseline(&mut self, id: &AppId) {
        if self.home.editing.as_ref().is_some_and(|app| app.id == *id) {
            if let Ok(latest) =
                WorkspaceStore::open(self.book.paths()).and_then(|store| store.load(id))
            {
                // Input entities retain the user's unsaved title, tags and note.
                self.home.editing = Some(latest);
            }
        }
    }
    pub(super) fn read_backup_profile(
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
        view.update(cx, |view, cx| {
            view.read_home_profile(id, bundle, link.backup_id, cx)
        });
    }
    pub(super) fn run_offline_job(
        &mut self,
        id: AppId,
        operation: rv_business::Operation,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_offline_job(id, operation, None, cx);
    }
    pub(super) fn dispatch_offline_job(
        &mut self,
        id: AppId,
        operation: rv_business::Operation,
        scheduled: Option<rv_business::schedules::ScheduledJob>,
        cx: &mut Context<Self>,
    ) {
        if self.read_only || !self.home.jobs.insert(id.clone()) {
            return;
        }
        let paths = self.book.paths().clone();
        let task_id = id.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let runner = rv_business::OfflineRunner::new(None)?;
                    if let Some(job) = scheduled {
                        runner.run_scheduled_blocking(&paths, &job)
                    } else {
                        runner.run_blocking(&paths, &task_id, operation)
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.home.jobs.remove(&id);
                match result {
                    Ok(()) => this.set_status(format!("{}已完成", operation.label()), cx),
                    Err(message) => this.set_status(message, cx),
                };
                if this.home.editing.as_ref().is_some_and(|app| app.id == id) {
                    this.home.tasks = WorkspaceStore::open(this.book.paths())
                        .and_then(|store| store.tasks(&id))
                        .unwrap_or_default();
                }
                this.refresh_editing_baseline(&id);
                if this.home.editing.as_ref().is_some_and(|app| app.id == id) {
                    this.reload_business_data(cx);
                }
                this.reload_home(cx);
            });
        })
        .detach();
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
