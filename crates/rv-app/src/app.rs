use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::*;
use gpui_component::{
    Root,
    input::{InputEvent, InputState},
};

use rv_core::{
    AddressBook, ClipboardMode, ConnectRequest, Connection, ConnectionId, EncryptionMode,
    LocalCursorMode, QualityPreset, StoreError, TransferFolders, parse_server,
};

use crate::actions::*;
use crate::session_window;
use crate::theme;

mod connections;
mod dialogs;
mod layout;
mod sidebar;
mod toolbar;

/// Window body: address book plus GPUI Component overlay layers.
///
/// Dialogs must not be painted from inside `AddressBookApp::render` — the
/// dialog builder would `read` that entity while it is still being updated.
pub struct WindowRoot {
    address_book: Entity<AddressBookApp>,
}

impl WindowRoot {
    pub fn new(address_book: Entity<AddressBookApp>) -> Self {
        Self { address_book }
    }
}

impl Render for WindowRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .relative()
            .child(self.address_book.clone())
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Grid,
    List,
}

#[derive(Clone, PartialEq, Eq)]
enum SidebarFilter {
    All,
    Recents,
    Label(String),
}

impl SidebarFilter {
    fn title(&self) -> String {
        match self {
            Self::All => "全部连接".into(),
            Self::Recents => "最近使用".into(),
            Self::Label(l) => l.clone(),
        }
    }
}

#[derive(Clone)]
enum Modal {
    None,
    Connection,
    DeleteConfirm { id: ConnectionId, name: String },
    Preferences,
}

const DEFAULT_ZXTOUCH_PORT: u16 = 6000;

pub struct AddressBookApp {
    book: AddressBook,
    read_only: bool,
    search: Entity<InputState>,
    name_input: Entity<InputState>,
    server_input: Entity<InputState>,
    username_input: Entity<InputState>,
    password_input: Entity<InputState>,
    labels_input: Entity<InputState>,
    zxtouch_port_input: Entity<InputState>,
    collapsed: bool,
    view_mode: ViewMode,
    filter: SidebarFilter,
    selected: Option<ConnectionId>,
    remember_password: bool,
    encryption: EncryptionMode,
    quality: QualityPreset,
    clipboard_mode: ClipboardMode,
    local_cursor_mode: LocalCursorMode,
    view_only: bool,
    shared: bool,
    editing: Option<ConnectionId>,
    modal: Modal,
    status: SharedString,
    credential_busy: bool,
    connection_error: Option<SharedString>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl AddressBookApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut status: SharedString = "Ready".into();
        let mut read_only = false;
        let book = match rv_core::StorePaths::default_dir()
            .and_then(AddressBook::load_or_quarantine)
        {
            Ok((book, warnings)) => {
                for w in &warnings {
                    tracing::warn!("{w}");
                }
                if let Some(w) = warnings.first() {
                    status = w.clone().into();
                }
                book
            }
            Err(e) => {
                tracing::warn!("address book unavailable: {e}");
                read_only = true;
                status =
                    format!("Address book unavailable ({e}); changes will not be saved").into();
                let scratch = std::env::temp_dir().join(format!("rv-{}", std::process::id()));
                AddressBook::load(rv_core::StorePaths::in_dir(scratch)).expect("temp address book")
            }
        };

        Self::with_book(book, read_only, status, window, cx)
    }

    fn with_book(
        book: AddressBook,
        read_only: bool,
        status: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        theme::apply(book.prefs().theme, window, cx);

        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("搜索连接…")
                .clean_on_escape()
        });
        let name_input = cx.new(|cx| InputState::new(window, cx).placeholder("例如：办公 Mac"));
        let server_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("host, host:1, host::5901, [::1]:5900")
        });
        let username_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Mac 登录用户名（可选）"));
        let password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("密码（可选）")
                .masked(true)
        });
        let labels_input = cx.new(|cx| InputState::new(window, cx).placeholder("工作, 测试, 家用"));

        let zxtouch_port_input = cx.new(|cx| InputState::new(window, cx).placeholder("6000"));

        let mut subscriptions = Vec::new();
        for input in [
            &name_input,
            &server_input,
            &username_input,
            &password_input,
            &labels_input,
            &zxtouch_port_input,
        ] {
            subscriptions.push(cx.subscribe_in(
                input,
                window,
                |this, _, ev: &InputEvent, window, cx| {
                    if matches!(ev, InputEvent::PressEnter { .. })
                        && matches!(this.modal, Modal::Connection)
                    {
                        this.submit_connect(true, window, cx);
                    }
                },
            ));
        }
        let this = cx.entity().downgrade();
        subscriptions.push(window.observe_window_appearance(move |window, cx| {
            if let Some(this) = this.upgrade() {
                let pref = this.read(cx).book.prefs().theme;
                theme::apply(pref, window, cx);
            }
        }));

        let focus = cx.focus_handle();
        focus.focus(window, cx);

        Self {
            book,
            read_only,
            search,
            name_input,
            server_input,
            username_input,
            password_input,
            labels_input,
            zxtouch_port_input,
            collapsed: false,
            view_mode: ViewMode::Grid,
            filter: SidebarFilter::All,
            selected: None,
            remember_password: true,
            encryption: EncryptionMode::LetServerChoose,
            quality: QualityPreset::Auto,
            clipboard_mode: ClipboardMode::Utf8,
            local_cursor_mode: LocalCursorMode::Automatic,
            view_only: false,
            shared: true,
            editing: None,
            modal: Modal::None,
            status,
            credential_busy: false,
            connection_error: None,
            focus,
            _subscriptions: subscriptions,
        }
    }

    fn persist(&mut self) {
        if self.read_only {
            return;
        }
        if let Err(e) = self.book.save() {
            self.status = format!("Save failed: {e}").into();
        }
    }

    pub(crate) fn remember_app_shortcuts(
        &mut self,
        id: ConnectionId,
        shortcuts: Vec<rv_core::AppShortcut>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.read_only {
            return Err("地址簿为只读，快捷栏修改仅在本次会话有效".into());
        }
        let Some(connection) = self.book.get_mut(id) else {
            return Err("连接已删除，快捷栏修改仅在本次会话有效".into());
        };
        if connection.app_shortcuts == shortcuts {
            return Ok(());
        }
        let previous = std::mem::replace(&mut connection.app_shortcuts, shortcuts);
        if let Err(error) = self.book.save() {
            if let Some(connection) = self.book.get_mut(id) {
                connection.app_shortcuts = previous;
            }
            let message = format!("快捷栏保存失败：{error}");
            self.status = message.clone().into();
            cx.notify();
            return Err(message);
        }
        cx.notify();
        Ok(())
    }

    pub(crate) fn environment_apps(&self, id: ConnectionId) -> Vec<rv_core::AppShortcut> {
        self.book
            .get(id)
            .map(|c| c.environment_apps.clone())
            .unwrap_or_default()
    }
    pub(crate) fn remember_environment_apps(
        &mut self,
        id: ConnectionId,
        apps: Vec<rv_core::AppShortcut>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.read_only {
            return Err("地址簿为只读，环境 App 列表仅在本次会话有效".into());
        }
        let connection = self
            .book
            .get_mut(id)
            .ok_or("连接已删除，环境 App 列表仅在本次会话有效")?;
        let previous = std::mem::replace(&mut connection.environment_apps, apps);
        if let Err(error) = self.book.save() {
            if let Some(connection) = self.book.get_mut(id) {
                connection.environment_apps = previous;
            }
            return Err(format!("环境 App 列表保存失败：{error}"));
        }
        cx.notify();
        Ok(())
    }
    pub(crate) fn remember_transfer_folders(
        &mut self,
        id: ConnectionId,
        folders: TransferFolders,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        if let Some(connection) = self.book.get_mut(id)
            && connection.transfer_folders != folders
        {
            connection.transfer_folders = folders;
            self.persist();
            cx.notify();
        }
    }

    pub(crate) fn save_retried_password(
        &mut self,
        request: &ConnectRequest,
        remember: bool,
        cx: &mut Context<Self>,
    ) -> Result<Option<ConnectionId>, StoreError> {
        if self.read_only {
            return Err(StoreError::Credential("address book is read-only".into()));
        }
        let id = match request.connection_id {
            Some(id) if self.book.get(id).is_some() => id,
            Some(_) => return Err(StoreError::UnknownConnection),
            None if remember => {
                let mut connection = Connection::new(&request.name, &request.host, request.port);
                connection.username = request.username.clone();
                connection.encryption = request.encryption;
                connection.quality = request.quality;
                connection.clipboard = request.clipboard;
                connection.local_cursor = request.local_cursor;
                connection.view_only = request.view_only;
                connection.zxtouch_port = request.zxtouch_port;
                connection.shared = request.shared;
                connection.last_connected = Some(unix_now());
                let id = connection.id;
                self.book.upsert(connection);
                self.selected = Some(id);
                id
            }
            None => return Ok(None),
        };
        if remember {
            if let Some(password) = request.password.as_deref() {
                self.book.paths().save_password(id, password)?;
            }
        } else {
            self.book.paths().delete_password(id)?;
        }
        if let Some(connection) = self.book.get_mut(id) {
            connection.remember_password = remember;
        }
        self.book.save()?;
        cx.notify();
        Ok(Some(id))
    }

    fn set_status(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.status = text.into();
        cx.notify();
    }

    fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().to_string()
    }

    fn visible(&self, cx: &App) -> Vec<Connection> {
        let q = self.query(cx);
        match &self.filter {
            SidebarFilter::All => self.book.filtered(&q, None),
            SidebarFilter::Recents => {
                let matching: HashSet<ConnectionId> =
                    self.book.filtered(&q, None).iter().map(|c| c.id).collect();
                self.book
                    .recents(20)
                    .into_iter()
                    .filter(|c| matching.contains(&c.id))
                    .collect()
            }
            SidebarFilter::Label(label) => self.book.filtered(&q, Some(label)),
        }
    }

    fn open_editor(
        &mut self,
        conn: Option<&Connection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.credential_busy {
            return;
        }
        let (name, server, username, labels) = match conn {
            Some(c) => (
                c.name.clone(),
                c.server_display(),
                c.username.clone().unwrap_or_default(),
                c.labels.join(", "),
            ),
            None => Default::default(),
        };
        self.connection_error = None;
        self.editing = conn.map(|c| c.id);
        self.encryption = conn.map_or(EncryptionMode::LetServerChoose, |c| c.encryption);
        self.quality = conn.map_or(QualityPreset::Auto, |c| c.quality);
        self.clipboard_mode = conn.map_or(ClipboardMode::Utf8, |c| c.clipboard);
        self.local_cursor_mode = conn.map_or(LocalCursorMode::Automatic, |c| c.local_cursor);
        self.view_only = conn.is_some_and(|c| c.view_only);
        let port = conn
            .and_then(|c| c.zxtouch_port)
            .unwrap_or(DEFAULT_ZXTOUCH_PORT)
            .to_string();
        self.zxtouch_port_input
            .update(cx, |input, cx| input.set_value(port, window, cx));
        self.shared = conn.is_none_or(|c| c.shared);
        self.remember_password = conn.is_none_or(|c| c.remember_password);
        self.name_input
            .update(cx, |s, cx| s.set_value(name, window, cx));
        self.server_input
            .update(cx, |s, cx| s.set_value(server, window, cx));
        self.username_input
            .update(cx, |s, cx| s.set_value(username, window, cx));
        self.password_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.labels_input
            .update(cx, |s, cx| s.set_value(labels, window, cx));
        self.modal = Modal::Connection;
        let first = if conn.is_some() {
            &self.name_input
        } else {
            &self.server_input
        };
        first.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    fn on_new(&mut self, _: &NewConnection, window: &mut Window, cx: &mut Context<Self>) {
        self.open_editor(None, window, cx);
    }

    fn on_properties(&mut self, _: &OpenProperties, window: &mut Window, cx: &mut Context<Self>) {
        let Some(conn) = self.selected.and_then(|id| self.book.get(id).cloned()) else {
            self.set_status("Select a connection first", cx);
            return;
        };
        self.open_editor(Some(&conn), window, cx);
    }

    fn on_duplicate(&mut self, _: &DuplicateSelected, _: &mut Window, cx: &mut Context<Self>) {
        let Some(conn) = self.selected.and_then(|id| self.book.get(id).cloned()) else {
            return;
        };
        let mut copy = conn;
        copy.id = ConnectionId::new();
        copy.name = format!("{} copy", copy.name);
        copy.last_connected = None;
        copy.remember_password = false;
        self.selected = Some(copy.id);
        self.book.upsert(copy);
        self.persist();
        cx.notify();
    }

    fn submit_connect(&mut self, connect: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.credential_busy {
            return;
        }
        self.connection_error = None;
        let name = self.name_input.read(cx).value().to_string();
        let server = self.server_input.read(cx).value().to_string();
        let username = self.username_input.read(cx).value().trim().to_string();
        let password = self.password_input.read(cx).unmask_value().to_string();
        let labels = parse_labels(&self.labels_input.read(cx).value());
        let (host, port) = match parse_server(&server) {
            Ok(v) => v,
            Err(e) => {
                self.connection_error = Some(e.into());
                cx.notify();
                return;
            }
        };
        let port_text = self.zxtouch_port_input.read(cx).value().trim().to_string();
        let zxtouch_port = if port_text.is_empty() {
            self.zxtouch_port_input.update(cx, |input, cx| {
                input.set_value(DEFAULT_ZXTOUCH_PORT.to_string(), window, cx)
            });
            Some(DEFAULT_ZXTOUCH_PORT)
        } else {
            match port_text.parse::<u16>() {
                Ok(port) if port > 0 && port_text.bytes().all(|byte| byte.is_ascii_digit()) => {
                    Some(port)
                }
                _ => {
                    self.connection_error = Some("ZXTouch 端口须为 1–65535".into());
                    cx.notify();
                    return;
                }
            }
        };
        let mut conn = self
            .editing
            .and_then(|id| self.book.get(id).cloned())
            .unwrap_or_else(|| Connection::new(&name, &host, port));
        conn.name = if name.trim().is_empty() {
            format!("{host}:{port}")
        } else {
            name.trim().to_string()
        };
        conn.host = host;
        conn.port = port;
        conn.username = (!username.is_empty()).then_some(username);
        conn.labels = labels;
        conn.encryption = self.encryption;
        conn.quality = self.quality;
        conn.clipboard = self.clipboard_mode;
        conn.local_cursor = self.local_cursor_mode;
        conn.view_only = self.view_only;
        conn.zxtouch_port = zxtouch_port;
        conn.shared = self.shared;
        let forget_password = conn.remember_password && !self.remember_password;
        conn.remember_password = self.remember_password;
        let id = conn.id;
        let remember = conn.remember_password;
        let paths = self.book.paths().clone();
        self.credential_busy = true;
        self.set_status("Preparing connection…", cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if forget_password {
                        paths.delete_password(id)?;
                    }
                    if remember && !password.is_empty() {
                        paths.save_password(id, &password)?;
                    }
                    if remember && password.is_empty() && connect {
                        paths.load_password(id)
                    } else {
                        Ok((!password.is_empty()).then_some(password))
                    }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.credential_busy = false;
                match result {
                    Ok(password) => this.finish_submit(conn, password, connect, window, cx),
                    Err(error) => {
                        this.connection_error = Some(error.to_string().into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn finish_submit(
        &mut self,
        mut conn: Connection,
        password: Option<String>,
        connect: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let req = ConnectRequest::from_connection(&conn, password);
        if connect {
            conn.last_connected = Some(unix_now());
        }
        self.selected = Some(conn.id);
        self.modal = Modal::None;
        self.password_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        let saved_name = conn.name.clone();
        self.book.upsert(conn);
        self.persist();
        if let SidebarFilter::Label(label) = &self.filter
            && !self
                .selected
                .and_then(|id| self.book.get(id))
                .is_some_and(|c| c.labels.contains(label))
        {
            self.filter = SidebarFilter::All;
        }
        if connect {
            self.launch(req, cx);
        } else {
            self.set_status(format!("Saved {saved_name}"), cx);
        }
        self.focus.focus(window, cx);
    }

    fn on_connect(&mut self, _: &ConnectSelected, window: &mut Window, cx: &mut Context<Self>) {
        match self.modal.clone() {
            Modal::Connection => self.submit_connect(true, window, cx),
            Modal::DeleteConfirm { id, .. } => self.confirm_delete(id, cx),
            Modal::Preferences => self.close_modal(cx),
            Modal::None => {
                let Some(id) = self.selected else {
                    self.set_status("Select a connection to connect", cx);
                    return;
                };
                self.connect_id(id, cx);
            }
        }
    }

    fn connect_id(&mut self, id: ConnectionId, cx: &mut Context<Self>) {
        if self.credential_busy {
            return;
        }
        let Some(conn) = self.book.get(id) else {
            return;
        };
        let remember = conn.remember_password;
        let paths = self.book.paths().clone();
        self.credential_busy = true;
        self.set_status(
            if remember {
                "Loading saved password…"
            } else {
                "Preparing connection…"
            },
            cx,
        );
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if remember {
                        paths.load_password(id)
                    } else {
                        Ok(None)
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.credential_busy = false;
                match result {
                    Ok(password) => {
                        let Some(mut conn) = this.book.get(id).cloned() else {
                            this.set_status("Connection no longer exists", cx);
                            return;
                        };
                        conn.last_connected = Some(unix_now());
                        let req = ConnectRequest::from_connection(&conn, password);
                        this.book.upsert(conn);
                        this.persist();
                        this.launch(req, cx);
                    }
                    Err(error) => this.set_status(error.to_string(), cx),
                }
            });
        })
        .detach();
    }

    fn launch(&mut self, req: ConnectRequest, cx: &mut Context<Self>) {
        self.status = format!("Connecting to {}…", req.display_name()).into();
        let title = req.display_name().to_string();
        let prefs = self.book.prefs();
        let transfer_folders = req
            .connection_id
            .and_then(|id| self.book.get(id))
            .map(|connection| connection.transfer_folders.clone())
            .unwrap_or_default();
        let options = session_window::SessionOptions {
            scale: prefs.default_scale,
            pin_toolbar: prefs.pin_toolbar,
            menu_key: prefs.menu_key.clone(),
            hide_shots: prefs.hide_screenshots,
            thumb_path: req.connection_id.map(|id| self.book.paths().thumb_path(id)),
            address_book: Some(cx.entity().downgrade()),
            transfer_folders,
            app_shortcuts: req
                .connection_id
                .and_then(|id| self.book.get(id))
                .map(|conn| conn.app_shortcuts.clone())
                .unwrap_or_else(rv_core::default_app_shortcuts),
            remember_password: req
                .connection_id
                .and_then(|id| self.book.get(id))
                .is_some_and(|conn| conn.remember_password),
        };
        session_window::open(req, title, options, cx);
        cx.notify();
    }

    /// Connect to a command-line target. Reuses a saved entry with the same
    /// host and port; otherwise connects ad hoc without touching the book.
    pub fn connect_target(&mut self, target: &str, _: &mut Window, cx: &mut Context<Self>) {
        match parse_server(target) {
            Ok((host, port)) => {
                let existing = self
                    .book
                    .connections()
                    .iter()
                    .find(|c| c.host.eq_ignore_ascii_case(&host) && c.port == port)
                    .map(|c| c.id);
                if let Some(id) = existing {
                    self.selected = Some(id);
                    self.connect_id(id, cx);
                } else {
                    let conn = Connection::new("", &host, port);
                    let mut req = ConnectRequest::from_connection(&conn, None);
                    req.connection_id = None;
                    self.launch(req, cx);
                }
            }
            Err(e) => self.set_status(e, cx),
        }
    }

    fn ask_delete(&mut self, id: ConnectionId, cx: &mut Context<Self>) {
        if self.credential_busy {
            return;
        }
        let name = self
            .book
            .get(id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "this connection".into());
        self.selected = Some(id);
        self.modal = Modal::DeleteConfirm { id, name };
        cx.notify();
    }

    fn confirm_delete(&mut self, id: ConnectionId, cx: &mut Context<Self>) {
        match self.book.remove(id) {
            Ok(()) => {
                if self.selected == Some(id) {
                    self.selected = None;
                }
                self.persist();
                self.status = "Connection removed".into();
                if let SidebarFilter::Label(label) = &self.filter
                    && !self.book.labels().contains(label)
                {
                    self.filter = SidebarFilter::All;
                }
            }
            Err(e) => {
                self.status = format!("Delete failed: {e}").into();
            }
        }
        self.modal = Modal::None;
        cx.notify();
    }

    fn close_modal(&mut self, cx: &mut Context<Self>) {
        if self.credential_busy {
            return;
        }
        self.modal = Modal::None;
        cx.notify();
    }

    fn on_close_modal(&mut self, _: &CloseModal, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.modal, Modal::None) {
            return;
        }
        self.close_modal(cx);
        self.focus.focus(window, cx);
    }

    fn on_delete(&mut self, _: &DeleteSelected, window: &mut Window, cx: &mut Context<Self>) {
        match self.modal.clone() {
            Modal::DeleteConfirm { id, .. } => self.confirm_delete(id, cx),
            Modal::None => {
                if self.search.focus_handle(cx).is_focused(window) {
                    return;
                }
                let Some(id) = self.selected else {
                    self.set_status("Select a connection to delete", cx);
                    return;
                };
                self.ask_delete(id, cx);
            }
            _ => {}
        }
    }

    fn on_prefs(&mut self, _: &OpenPreferences, _: &mut Window, cx: &mut Context<Self>) {
        self.modal = Modal::Preferences;
        cx.notify();
    }

    fn on_toggle_view(&mut self, _: &ToggleViewMode, _: &mut Window, cx: &mut Context<Self>) {
        self.view_mode = match self.view_mode {
            ViewMode::Grid => ViewMode::List,
            ViewMode::List => ViewMode::Grid,
        };
        cx.notify();
    }

    fn on_toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.collapsed = !self.collapsed;
        cx.notify();
    }

    fn on_focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.focus(window, cx));
    }

    fn select(&mut self, id: ConnectionId, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(id);
        self.focus.focus(window, cx);
        cx.notify();
    }
}

impl Focusable for AddressBookApp {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Human-friendly age of a unix timestamp.
fn relative_time(ts: i64) -> String {
    let delta = (unix_now() - ts).max(0);
    match delta {
        d if d < 60 => "刚刚使用".into(),
        d if d < 3600 => format!("{} 分钟前", d / 60),
        d if d < 86_400 => format!("{} 小时前", d / 3600),
        d if d < 30 * 86_400 => format!("{} 天前", d / 86_400),
        d if d < 365 * 86_400 => format!("{} 个月前", d / (30 * 86_400)),
        d => format!("{} 年前", d / (365 * 86_400)),
    }
}

fn parse_labels(text: &str) -> Vec<String> {
    let mut out: Vec<String> = text
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the `gpui::*` glob would shadow `#[test]`.
    use super::{parse_labels, relative_time, unix_now};

    #[test]
    fn labels_are_trimmed_sorted_and_deduped() {
        assert_eq!(
            parse_labels(" work, lab ,,work, home "),
            vec!["home", "lab", "work"]
        );
        assert!(parse_labels("  ").is_empty());
    }

    #[test]
    fn relative_time_buckets() {
        let now = unix_now();
        assert_eq!(relative_time(now), "刚刚使用");
        assert_eq!(relative_time(now - 120), "2 分钟前");
        assert_eq!(relative_time(now - 7200), "2 小时前");
        assert_eq!(relative_time(now - 3 * 86_400), "3 天前");
    }
}

#[cfg(test)]
mod ui_tests;

#[cfg(all(test, target_os = "macos"))]
mod visual_tests;
