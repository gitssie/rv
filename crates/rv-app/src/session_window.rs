use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    Disableable as _, Icon, IconName, Selectable as _, StyledExt, TitleBar,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{PopupMenu, PopupMenuItem},
    popover::Popover,
    v_flex,
};
use image::{ImageBuffer, Rgba};
use smallvec::SmallVec;

use rv_core::{
    CAD_KEYSYMS, ConnectRequest, Keyboard, LocalCursorMode, ScaleMode, TransferFolders, XK_ALT_L,
    XK_CONTROL_L,
};
use rv_session::{FileCommand, SessionEvent, SessionHandle, TransferStatus};

use crate::actions::*;
use crate::app::AddressBookApp;
use crate::file_transfer_window;
use crate::theme;

pub struct SessionOptions {
    pub scale: ScaleMode,
    pub pin_toolbar: bool,
    pub menu_key: String,
    pub hide_shots: bool,
    pub thumb_path: Option<PathBuf>,
    pub app_icon_cache: Option<rv_core::StorePaths>,
    pub address_book: Option<WeakEntity<AddressBookApp>>,
    pub transfer_folders: TransferFolders,
    pub app_shortcuts: Vec<rv_core::AppShortcut>,
    pub remember_password: bool,
}

/// Pixels of scroll per RFB wheel click. One notch of a mouse wheel is one
/// line, which GPUI reports as this many pixels; trackpads accumulate.
const WHEEL_STEP: f32 = 20.0;

const BTN_LEFT: u8 = 1;
const BTN_MIDDLE: u8 = 2;
const BTN_RIGHT: u8 = 4;
const BTN_WHEEL_UP: u8 = 8;
const BTN_WHEEL_DOWN: u8 = 16;
const BTN_WHEEL_LEFT: u8 = 32;
const BTN_WHEEL_RIGHT: u8 = 64;

pub fn open(req: ConnectRequest, title: String, session: SessionOptions, cx: &mut App) {
    // Open after the current window update finishes. Nesting `open_window`
    // inside another window's constructor tears the session window down.
    cx.spawn(async move |cx| {
        let mut window_options = TitleBar::window_options();
        window_options.window_bounds = Some(WindowBounds::Windowed(Bounds {
            origin: point(px(72.), px(64.)),
            size: theme::control_window_size(),
        }));
        window_options.window_min_size = Some(size(px(640.), px(400.)));
        window_options.titlebar = Some(TitlebarOptions {
            title: Some(title.clone().into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(9.), px(12.))),
        });
        let owner = session.address_book.clone();
        let device = req.connection_id;
        let created = Rc::new(RefCell::new(None));
        let captured = created.clone();
        let result = cx.open_window(window_options, move |window, cx| {
            let view = cx.new(|cx| SessionView::new(req, title, session, window, cx));
            *captured.borrow_mut() = Some(view.downgrade());
            cx.new(|cx| {
                gpui_component::Root::new(cx.new(|cx| SessionShell::new(view, cx)), window, cx)
            })
        });
        if let Ok(handle) = result
            && let Some(view) = created.borrow_mut().take()
        {
            let _ = view.update(cx, |view, _| view.home_window = Some(handle));
            if let Some(device) = device
                && let Some(owner) = owner
            {
                let _ = owner.update(cx, |book, cx| {
                    book.register_phone_window(device, view, handle, cx)
                });
            }
        }
    })
    .detach();
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Connecting,
    Connected,
    Disconnecting,
    /// The session is over; `error` says whether it failed.
    Ended,
}

fn password_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "password check failed",
        "wrong password",
        "invalid password",
        "password rejected",
        "authentication failed",
        "authentication rejected",
        "auth is required but no password provided",
        "mac login was rejected",
    ]
    .iter()
    .any(|reason| message.contains(reason))
}

struct SessionShell {
    view: Entity<SessionView>,
    _updates: Subscription,
}
impl SessionShell {
    fn new(view: Entity<SessionView>, cx: &mut Context<Self>) -> Self {
        // Dialog builders read live operation state, so their layer must update with the session.
        let updates = cx.observe(&view, |_, _, cx| cx.notify());
        Self {
            view,
            _updates: updates,
        }
    }
}
impl Render for SessionShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .relative()
            .child(self.view.clone())
            .children(gpui_component::Root::render_dialog_layer(window, cx))
            .children(gpui_component::Root::render_notification_layer(window, cx))
    }
}

pub struct SessionView {
    functions: functions_ui::FunctionUi,
    assistant: Option<Entity<crate::work_assistant::WorkAssistant>>,
    assistant_open: bool,
    unlock_dialog_open: bool,
    assistant_subscription: Option<Subscription>,
    assistant_batch: Option<crate::photo_transfer::PhotoBatch>,
    assistant_clipboard: Option<String>,
    assistant_progress: String,
    req: ConnectRequest,
    requested_environment: Option<String>,
    home_window: Option<WindowHandle<gpui_component::Root>>,
    control_only: bool,
    pending_home_delete: Option<rv_core::workspace::AppId>,
    handle: SessionHandle,
    title: SharedString,
    status: SharedString,
    phase: Phase,
    scale: ScaleMode,
    pin_toolbar: bool,
    toolbar_open: bool,
    menu_key: String,
    hide_shots: bool,
    thumb_path: Option<PathBuf>,
    show_menu: bool,
    session_menu: Option<Entity<PopupMenu>>,
    window_controls: crate::window_controls::WindowControls,
    info_open: bool,
    buttons: u8,
    wheel_accum: Point<f32>,
    last_generation: u64,
    last_file_revision: u64,
    render_image: Option<Arc<RenderImage>>,
    fb_w: u16,
    fb_h: u16,
    error: Option<SharedString>,
    password_input: Entity<InputState>,
    remember_password: bool,
    retried_password: bool,
    password_validation: Option<SharedString>,
    file_transfer_window: Option<WindowHandle<gpui_component::Root>>,
    file_transfer_opening: bool,
    transfer_folders: Rc<RefCell<TransferFolders>>,
    address_book: Option<WeakEntity<AddressBookApp>>,
    store_paths: Option<rv_core::StorePaths>,
    mailbox: mailbox_ui::MailboxUI,
    _password_subscription: Subscription,
    apps: apps_ui::AppToolbar,
    karing: karing_ui::KaringUI,
    unlock: unlock_ui::UnlockUI,
    notifications: controls_ui::NotificationsUI,
    recovery: reconnect::Recovery,
    screenshot_busy: bool,
    /// Where the remote picture was last painted, in window pixels. Filled
    /// in by a probe element so pointer mapping never guesses chrome sizes.
    image_box: Rc<Cell<Bounds<Pixels>>>,
    keys: Keyboard,
    command_paste_pressed: bool,
    clipboard_transfer: clipboard_ui::ClipboardTransfer,
    focus: FocusHandle,
    canvas_hovered: bool,
    pointer_in_window: bool,
    local_cursor: local_cursor::LocalCursor,
    _cursor_subscriptions: Vec<Subscription>,
}

impl SessionView {
    fn new(
        req: ConnectRequest,
        title: String,
        options: SessionOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let handle = SessionHandle::spawn(req.clone());
        Self::with_handle(req, title, options, handle, window, cx)
    }

    fn with_handle(
        req: ConnectRequest,
        title: String,
        options: SessionOptions,
        handle: SessionHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_transport(req, title, options, handle, false, None, window, cx)
    }

    pub(crate) fn new_control(
        req: ConnectRequest,
        options: SessionOptions,
        environment_apps: Vec<rv_core::AppShortcut>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title = req.display_name().to_string();
        let handle = SessionHandle::spawn_control(req.clone());
        Self::with_transport(
            req,
            title,
            options,
            handle,
            true,
            Some(environment_apps),
            window,
            cx,
        )
    }

    fn with_transport(
        req: ConnectRequest,
        title: String,
        options: SessionOptions,
        handle: SessionHandle,
        control_only: bool,
        environment_apps: Option<Vec<rv_core::AppShortcut>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        if !control_only {
            focus.focus(window, cx);
        }
        let cursor_subscriptions = if control_only {
            vec![]
        } else {
            vec![
                cx.observe_window_activation(window, Self::update_local_cursor),
                cx.on_focus(&focus, window, Self::update_local_cursor),
                cx.on_blur(&focus, window, Self::update_local_cursor),
            ]
        };
        let SessionOptions {
            scale,
            pin_toolbar,
            menu_key,
            hide_shots,
            thumb_path,
            app_icon_cache,
            address_book,
            transfer_folders,
            app_shortcuts,
            remember_password,
        } = options;
        let store_paths = app_icon_cache
            .clone()
            .or_else(|| rv_core::StorePaths::default_dir().ok());
        let password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Enter a new password")
                .masked(true)
        });
        let password_subscription = cx.subscribe_in(
            &password_input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } if this.phase == Phase::Ended => {
                    this.retry_with_password(window, cx);
                }
                InputEvent::Change => {
                    this.password_validation = None;
                    cx.notify();
                }
                _ => {}
            },
        );
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                if this.update(cx, |this, cx| this.pump(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        let mut apps = apps_ui::AppToolbar::new(app_shortcuts, window, cx);
        apps.cache_paths = app_icon_cache;
        let unlock = unlock_ui::UnlockUI::new(window, cx);
        let environment_apps = environment_apps.unwrap_or_else(|| {
            req.connection_id
                .and_then(|id| {
                    address_book
                        .as_ref()
                        .and_then(WeakEntity::upgrade)
                        .map(|book| book.read(cx).environment_apps(id))
                })
                .unwrap_or_default()
        });
        let functions = functions_ui::FunctionUi::new(environment_apps, window, cx);
        let mailbox = mailbox_ui::MailboxUI::new(window, cx);
        Self {
            functions,
            assistant: None,
            assistant_open: false,
            unlock_dialog_open: false,
            assistant_subscription: None,
            assistant_batch: None,
            assistant_clipboard: None,
            assistant_progress: String::new(),
            apps,
            karing: karing_ui::KaringUI::default(),
            unlock,
            notifications: controls_ui::NotificationsUI::default(),
            recovery: reconnect::Recovery::default(),
            screenshot_busy: false,
            req,
            requested_environment: None,
            home_window: None,
            control_only,
            pending_home_delete: None,
            handle,
            title: title.into(),
            status: "Connecting…".into(),
            phase: Phase::Connecting,
            scale,
            pin_toolbar,
            toolbar_open: pin_toolbar,
            menu_key,
            hide_shots,
            thumb_path,
            show_menu: false,
            session_menu: None,
            window_controls: Default::default(),
            info_open: false,
            buttons: 0,
            wheel_accum: point(0.0, 0.0),
            last_generation: 0,
            last_file_revision: 0,
            render_image: None,
            fb_w: 0,
            fb_h: 0,
            error: None,
            password_input,
            remember_password,
            retried_password: false,
            password_validation: None,
            file_transfer_window: None,
            file_transfer_opening: false,
            transfer_folders: Rc::new(RefCell::new(transfer_folders)),
            address_book,
            store_paths,
            mailbox,
            _password_subscription: password_subscription,
            image_box: Rc::new(Cell::new(Bounds::default())),
            keys: Keyboard::new(),
            command_paste_pressed: false,
            clipboard_transfer: clipboard_ui::ClipboardTransfer::default(),
            focus,
            canvas_hovered: false,
            pointer_in_window: true,
            local_cursor: local_cursor::LocalCursor::default(),
            _cursor_subscriptions: cursor_subscriptions,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_control_test_handle(
        req: ConnectRequest,
        options: SessionOptions,
        handle: SessionHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_transport(
            req,
            "test control".into(),
            options,
            handle,
            true,
            Some(vec![]),
            window,
            cx,
        )
    }
    #[cfg(test)]
    pub(crate) fn pump_control_test(&mut self, cx: &mut Context<Self>) {
        self.pump(cx);
    }

    fn update_local_cursor(&mut self, window: &mut Window, _: &mut Context<Self>) {
        if !window.is_window_active() || !self.focus.is_focused(window) {
            // The V key-up may go to another control or application.
            self.command_paste_pressed = false;
            self.clipboard_transfer.cancel_paste();
        }
        // RFB does not reliably say whether the server paints a pointer into
        // the framebuffer. Automatic therefore favors a visible local cursor;
        // Hide is an explicit opt-in for servers that paint their own pointer.
        let hidden = self.req.local_cursor == LocalCursorMode::Hide
            && self.phase == Phase::Connected
            && self.render_image.is_some()
            && !self.show_menu
            && self.canvas_hovered
            && self.pointer_in_window
            && window.is_window_active()
            && self.focus.is_focused(window)
            && self.map_pointer(window.mouse_position()).is_some();
        self.local_cursor.set_hidden(hidden);
    }

    fn view_only(&self) -> bool {
        self.req.view_only
    }

    fn host(&self) -> String {
        format!("{}:{}", self.req.host, self.req.port)
    }

    fn open_file_transfer(&mut self, cx: &mut Context<Self>) {
        // Claim the request before scheduling it, so rapid clicks from either
        // the toolbar or menu cannot queue multiple window creations.
        if self.file_transfer_opening {
            return;
        }
        self.file_transfer_opening = true;
        let existing = self.file_transfer_window;
        let host = self.host();
        let client = self.handle.file_client();
        let allow_upload = !self.view_only();
        let memory = file_transfer_window::TransferMemory {
            folders: self.transfer_folders.clone(),
            address_book: self.address_book.clone(),
            connection_id: self.req.connection_id,
        };
        cx.spawn(async move |this, cx| {
            if this.upgrade().is_none() {
                return;
            }
            if let Some(handle) = existing
                && handle
                    .update(cx, |_, window, _| window.activate_window())
                    .is_ok()
            {
                let _ = this.update(cx, |view, _| view.file_transfer_opening = false);
                return;
            }
            let result = file_transfer_window::open(host, client, allow_upload, memory, cx);
            let _ = this.update(cx, |view, cx| {
                view.file_transfer_opening = false;
                match result {
                    Ok(handle) => view.file_transfer_window = Some(handle),
                    Err(error) => {
                        view.file_transfer_window = None;
                        view.status = format!("Cannot open file transfer: {error}").into();
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn quick_upload(&mut self, cx: &mut Context<Self>) {
        let client = self.handle.file_client();
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Upload".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = selected.await
                && let Some(source) = paths.into_iter().next()
                && let Some(name) = source.file_name().and_then(|name| name.to_str())
            {
                let path = client.snapshot().remote_path;
                let remote = if path == "/" {
                    format!("/{name}")
                } else {
                    format!("{path}/{name}")
                };
                let _ = this.update(cx, |this, cx| {
                    this.status = "Uploading file…".into();
                    cx.notify();
                });
                client.send(FileCommand::Upload { source, remote });
            }
        })
        .detach();
    }

    fn pump(&mut self, cx: &mut Context<Self>) {
        // Rebuild the GPU image at most once per tick: each rebuild copies the
        // whole framebuffer on the UI thread, and this thread also delivers key
        // events. Stalling it delays key-ups, which the remote turns into
        // auto-repeat.
        let mut frame = None;
        let mut changed = self
            .clipboard_transfer
            .expire_feedback(std::time::Instant::now());
        for ev in self.handle.drain() {
            if self.phase == Phase::Disconnecting
                && !matches!(&ev, SessionEvent::Error(_) | SessionEvent::Disconnected)
            {
                continue;
            }
            changed = true;
            match ev {
                SessionEvent::Apps(event) => self.apply_app_event(event, cx),
                SessionEvent::Functions(event) => self.apply_function_event(event, cx),
                SessionEvent::Status(s) => self.status = s.into(),
                SessionEvent::ControlState(online) => {
                    if !online {
                        self.functions.cancel_cloud_preparations();
                        self.clipboard_transfer.disconnected();
                    }
                    self.phase = if online {
                        Phase::Connected
                    } else {
                        Phase::Connecting
                    };
                    self.status = if online {
                        "ZXTouch 已连接"
                    } else {
                        "ZXTouch 正在重连…"
                    }
                    .into();
                    self.notify_home(cx);
                }
                SessionEvent::Connected { width, height } => {
                    self.fb_w = width;
                    self.fb_h = height;
                    self.phase = Phase::Connected;
                    self.notify_home(cx);
                    self.recovery.connected();
                    self.status = format!("{width}×{height}").into();
                    self.error = None;
                    if self.retried_password {
                        self.retried_password = false;
                        if let Some(book) = self.address_book.as_ref().and_then(WeakEntity::upgrade)
                        {
                            match book.update(cx, |book, cx| {
                                book.save_retried_password(&self.req, self.remember_password, cx)
                            }) {
                                Ok(Some(id)) => {
                                    self.req.connection_id = Some(id);
                                    if let Some(handle) = self.home_window {
                                        let view = cx.entity().downgrade();
                                        let book = book.downgrade();
                                        cx.defer(move |cx| {
                                            if let Some(book) = book.upgrade() {
                                                book.update(cx, |book, cx| {
                                                    book.register_phone_window(id, view, handle, cx)
                                                });
                                            }
                                        });
                                    }
                                }
                                Ok(None) => {}
                                Err(error) => {
                                    self.status =
                                        format!("Connected, but password was not saved: {error}")
                                            .into();
                                }
                            }
                        } else if self.remember_password {
                            self.status =
                                "Connected, but the address book is closed; password was not saved"
                                    .into();
                        }
                    }
                }
                SessionEvent::FrameReady { generation } => frame = Some(generation),
                SessionEvent::Clipboard(text) => {
                    if !self.clipboard_transfer.is_outbound_echo(&text) {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                        self.status = "Clipboard received".into();
                    }
                }
                SessionEvent::Bell => self.status = "Bell".into(),
                SessionEvent::Error(e) => {
                    self.error = Some(e.clone().into());
                    self.status = e.into();
                }
                SessionEvent::Disconnected => {
                    let deliberate = self.phase == Phase::Disconnecting;
                    if !deliberate {
                        self.recovery
                            .schedule(self.error.as_deref(), std::time::Instant::now());
                    }
                    self.unlock.disconnected();
                    self.functions.disconnected();
                    self.assistant_clipboard = None;
                    self.clipboard_transfer.disconnected();
                    self.assistant_batch = None;
                    self.assistant_feedback(
                        "手机已断线，草稿仍保存在本地；待确认导入不会自动重试".into(),
                        cx,
                    );
                    self.local_cursor.set_hidden(false);
                    if self.error.is_none() {
                        self.status = "Disconnected".into();
                    }
                    self.apps.disconnected(cx);
                    self.karing.disconnected();
                    self.command_paste_pressed = false;
                    self.phase = Phase::Ended;
                    self.notify_home(cx);
                    self.buttons = 0;
                    self.save_thumb();
                }
            }
        }
        if self.phase == Phase::Ended && self.recovery.deadline.is_some() {
            changed = true;
            if self
                .recovery
                .deadline
                .is_some_and(|deadline| deadline <= std::time::Instant::now())
            {
                self.start_reconnect(cx);
            }
        }
        self.tick_home_environment(cx);
        self.tick_unlock(cx);
        self.sync_assistant(cx);
        let file_revision = self.handle.file_client().revision();
        if file_revision != self.last_file_revision {
            self.last_file_revision = file_revision;
            if self.status.as_ref().starts_with("Uploading file") {
                let files = self.handle.file_client().snapshot();
                if let Some(error) = files.error {
                    self.status = format!("Upload failed: {error}").into();
                } else if let Some(upload) = files.transfers.iter().rev().find(|transfer| {
                    matches!(transfer.direction, rv_session::TransferDirection::Upload)
                }) {
                    self.status = match &upload.status {
                        TransferStatus::Running => {
                            format!("Uploading file… {} bytes", upload.bytes)
                        }
                        TransferStatus::Sent => "File sent; verify it in the remote folder".into(),
                        TransferStatus::Complete => "Upload complete".into(),
                        TransferStatus::Verified => "Upload complete · SHA-256 verified".into(),
                        TransferStatus::Cancelled => "Upload cancelled".into(),
                        TransferStatus::Failed(reason) => format!("Upload failed: {reason}"),
                    }
                    .into();
                }
            }
            changed = true;
        }
        if let Some(generation) = frame
            && generation != self.last_generation
        {
            self.last_generation = generation;
            self.rebuild_image(cx);
        }
        if changed {
            cx.notify();
        }
    }

    fn rebuild_image(&mut self, cx: &mut App) {
        let snapshot = {
            let Ok(fb) = self.handle.framebuffer.lock() else {
                return;
            };
            self.fb_w = fb.width;
            self.fb_h = fb.height;
            if fb.width == 0 || fb.height == 0 {
                return;
            }
            // The compositor already stores BGRA, which is what RenderImage
            // uploads, so this is a plain copy.
            (fb.width, fb.height, fb.pixels.clone())
        };
        let (width, height, pixels) = snapshot;
        let Some(buf) = ImageBuffer::<Rgba<u8>, _>::from_raw(width as u32, height as u32, pixels)
        else {
            return;
        };
        let image = Arc::new(RenderImage::new(SmallVec::from_elem(
            image::Frame::new(buf),
            1,
        )));
        if let Some(old) = self.render_image.replace(image) {
            cx.drop_image(old, None);
        }
    }

    fn save_thumb(&self) {
        if self.hide_shots {
            return;
        }
        let Some(path) = &self.thumb_path else {
            return;
        };
        if let Ok(fb) = self.handle.framebuffer.lock()
            && let Some(png) = fb.thumbnail_png(320)
        {
            let _ = std::fs::write(path, png);
        }
    }

    fn effective_scale(&self) -> ScaleMode {
        if self.assistant_open {
            ScaleMode::Fit
        } else {
            self.scale
        }
    }
    fn map_pointer(&self, position: Point<Pixels>) -> Option<(u16, u16)> {
        let bounds = self.image_box.get();
        let box_w = f32::from(bounds.size.width);
        let box_h = f32::from(bounds.size.height);
        let local_x = f32::from(position.x - bounds.origin.x);
        let local_y = f32::from(position.y - bounds.origin.y);
        map_to_fb(
            local_x,
            local_y,
            box_w,
            box_h,
            self.fb_w,
            self.fb_h,
            self.effective_scale(),
        )
    }

    fn send_pointer(&mut self, x: u16, y: u16) {
        if self.view_only() || self.unlock.blocks_input() || self.phase != Phase::Connected {
            return;
        }
        self.handle.pointer(x, y, self.buttons);
    }

    fn pointer_at(&mut self, position: Point<Pixels>) {
        if let Some((x, y)) = self.map_pointer(position) {
            self.send_pointer(x, y);
        }
    }

    fn button(&mut self, bit: u8, down: bool, position: Point<Pixels>) {
        if down {
            self.buttons |= bit;
        } else {
            self.buttons &= !bit;
        }
        self.pointer_at(position);
    }

    fn wheel(&mut self, ev: &ScrollWheelEvent) {
        let delta = ev.delta.pixel_delta(px(WHEEL_STEP));
        self.wheel_accum.x += f32::from(delta.x);
        self.wheel_accum.y += f32::from(delta.y);
        let Some((x, y)) = self.map_pointer(ev.position) else {
            self.wheel_accum = point(0.0, 0.0);
            return;
        };
        let mut clicks: Vec<u8> = Vec::new();
        while self.wheel_accum.y <= -WHEEL_STEP {
            self.wheel_accum.y += WHEEL_STEP;
            clicks.push(BTN_WHEEL_DOWN);
        }
        while self.wheel_accum.y >= WHEEL_STEP {
            self.wheel_accum.y -= WHEEL_STEP;
            clicks.push(BTN_WHEEL_UP);
        }
        while self.wheel_accum.x <= -WHEEL_STEP {
            self.wheel_accum.x += WHEEL_STEP;
            clicks.push(BTN_WHEEL_RIGHT);
        }
        while self.wheel_accum.x >= WHEEL_STEP {
            self.wheel_accum.x -= WHEEL_STEP;
            clicks.push(BTN_WHEEL_LEFT);
        }
        for bit in clicks {
            self.buttons |= bit;
            self.send_pointer(x, y);
            self.buttons &= !bit;
            self.send_pointer(x, y);
        }
    }

    fn send_keys(&mut self, events: impl IntoIterator<Item = (u32, bool)>) {
        if self.view_only() || self.unlock.blocks_input() || self.phase != Phase::Connected {
            return;
        }
        for (keysym, down) in events {
            self.handle.key(keysym, down);
        }
    }

    fn tap_key(&mut self, keysym: u32) {
        self.send_keys([(keysym, true), (keysym, false)]);
    }

    fn send_cad(&mut self) {
        let down = CAD_KEYSYMS.iter().map(|k| (*k, true));
        let up = CAD_KEYSYMS.iter().rev().map(|k| (*k, false));
        self.send_keys(down.chain(up));
    }

    fn send_clipboard(&mut self, cx: &App) {
        self.begin_clipboard_transfer(false, cx);
    }

    fn toggle_fullscreen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.toggle_fullscreen();
        cx.notify();
    }

    fn set_scale(&mut self, scale: ScaleMode, cx: &mut Context<Self>) {
        self.scale = scale;
        cx.notify();
    }

    fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.recovery.stop();
        if matches!(self.phase, Phase::Disconnecting | Phase::Ended) {
            return;
        }
        self.save_thumb();
        let released = self.keys.release_all();
        self.send_keys(released);
        self.handle.close();
        self.phase = Phase::Disconnecting;
        self.local_cursor.set_hidden(false);
        self.status = "Disconnecting…".into();
        cx.notify();
    }

    fn reconnect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.apps.disconnected(cx);
        self.karing.disconnected();
        self.local_cursor.set_hidden(false);
        self.focus.focus(window, cx);
        self.recovery.stopped = false;
        self.recovery.attempt = 0;
        self.start_reconnect(cx);
    }

    fn start_reconnect(&mut self, cx: &mut Context<Self>) {
        self.recovery.deadline = None;
        self.apps.disconnected(cx);
        self.karing.disconnected();
        self.unlock.disconnected();
        self.notifications.error = None;
        if let Some(handle) = self.file_transfer_window.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
        self.handle = SessionHandle::spawn(self.req.clone());
        self.phase = Phase::Connecting;
        self.error = None;
        self.status = "Connecting…".into();
        self.buttons = 0;
        self.last_generation = 0;
        self.keys = Keyboard::new();
        self.command_paste_pressed = false;
        // Preserve the frozen frame until the new connection produces a frame.
        cx.notify();
    }

    fn retry_with_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.phase != Phase::Ended || !self.error.as_deref().is_some_and(password_error) {
            return;
        }
        let password = self.password_input.read(cx).unmask_value().to_string();
        if password.is_empty() {
            self.password_validation = Some("Enter a password to reconnect.".into());
            cx.notify();
            return;
        }
        self.req.password = Some(password);
        self.retried_password = true;
        self.password_validation = None;
        self.reconnect(window, cx);
    }

    fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.local_cursor.set_hidden(false);
        self.disconnect(cx);
        window.remove_window();
    }

    fn consume_key(&self, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        window.prevent_default();
    }

    fn handle_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.apps.picker_open {
            if event.keystroke.key == "escape" {
                self.apps.picker_open = false;
                cx.notify();
            }
            self.consume_key(window, cx);
            return;
        }
        if self.functions.open {
            if event.keystroke.key == "escape" {
                if self.function_editor_visible() {
                    self.close_function_editor(window, cx);
                } else {
                    self.functions.open = false;
                    if !self.functions.selected.is_some_and(|spec| spec.task == 31) {
                        self.clear_function_text(window, cx);
                    }
                    self.focus.focus(window, cx);
                    cx.notify();
                }
            }
            return;
        }
        if self.mailbox.open {
            if event.keystroke.key == "escape" {
                self.mailbox.open = false;
                self.focus.focus(window, cx);
                cx.notify();
            }
            return;
        }
        if self.notifications.open {
            if event.keystroke.key == "escape" {
                self.notifications.open = false;
                cx.notify();
            }
            self.consume_key(window, cx);
            return;
        }
        if self.phase != Phase::Connected
            || self.unlock.blocks_input()
            || self.notifications.open
            || self.apps.picker_open
            || self.functions.open
            || self.mailbox.open
            || !self.focus.is_focused(window)
        {
            return;
        }
        let key = event.keystroke.key.as_str();
        tracing::debug!(
            key,
            key_char = ?event.keystroke.key_char,
            is_held = event.is_held,
            "key down"
        );
        if key.eq_ignore_ascii_case(&self.menu_key) {
            self.set_session_menu(!self.show_menu, window, cx);
            self.consume_key(window, cx);
            return;
        }
        if self.show_menu && key == "escape" {
            self.show_menu = false;
            cx.notify();
            self.consume_key(window, cx);
            return;
        }
        let mods = event.keystroke.modifiers;
        if key.eq_ignore_ascii_case("v")
            && self.apps.caps.is_some()
            && (self.command_paste_pressed
                || ((mods.control ^ mods.platform) && !mods.alt && !mods.shift))
        {
            if !self.command_paste_pressed && !event.is_held {
                self.command_paste_pressed = true;
                self.begin_clipboard_transfer(true, cx);
                cx.notify();
            }
            self.consume_key(window, cx);
            return;
        }
        let mut events = self
            .keys
            .set_modifiers(mods.control, mods.alt, mods.platform);
        events.extend(self.keys.key_down(
            key,
            event.keystroke.key_char.as_deref(),
            mods.shift,
            event.is_held,
        ));
        self.send_keys(events);
        if Keyboard::recognizes(key, event.keystroke.key_char.as_deref()) {
            self.consume_key(window, cx);
        }
    }

    fn handle_key_up(&mut self, event: &KeyUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.phase != Phase::Connected
            || self.unlock.blocks_input()
            || self.notifications.open
            || self.apps.picker_open
            || self.functions.open
            || self.mailbox.open
            || !self.focus.is_focused(window)
        {
            return;
        }
        let key = event.keystroke.key.as_str();
        let mods = event.keystroke.modifiers;
        if key.eq_ignore_ascii_case("v") && self.command_paste_pressed {
            self.command_paste_pressed = false;
            let events = self
                .keys
                .set_modifiers(mods.control, mods.alt, mods.platform);
            self.send_keys(events);
            self.consume_key(window, cx);
            return;
        }
        let mut events = self.keys.key_up(key);
        events.extend(
            self.keys
                .set_modifiers(mods.control, mods.alt, mods.platform),
        );
        self.send_keys(events);
        if Keyboard::recognizes(key, event.keystroke.key_char.as_deref()) {
            self.consume_key(window, cx);
        }
    }

    fn handle_modifiers(&mut self, event: &ModifiersChangedEvent, window: &Window) {
        if self.phase != Phase::Connected
            || self.unlock.blocks_input()
            || self.notifications.open
            || self.apps.picker_open
            || self.functions.open
            || self.mailbox.open
            || !self.focus.is_focused(window)
        {
            return;
        }
        let events = self.keys.set_modifiers(
            event.modifiers.control,
            event.modifiers.alt,
            event.modifiers.platform,
        );
        self.send_keys(events);
    }

    fn render_toolbar(
        &self,
        _fullscreen: bool,
        _width: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let live = self.phase == Phase::Connected && !self.view_only();
        h_flex()
            .debug_selector(|| "session-toolbar".into())
            .h(theme::toolbar_height())
            .min_w_0()
            .w_full()
            .px_2()
            .gap_1()
            .items_center()
            .bg(theme::card(cx))
            .child(self.render_home(cx))
            .child(self.render_unlock_button(cx))
            .child(self.render_screenshot(cx))
            .child(self.render_app_toolbar(cx))
            .child(
                crate::ui::icon_button("clip", IconName::Copy, "发送剪贴板")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.send_clipboard(cx);
                        cx.notify();
                    }))
                    .text_color(self.clipboard_transfer.color(cx))
                    .disabled(!live || self.clipboard_transfer.busy()),
            )
            .child(div().flex_1())
            .child(self.render_function_button(true, cx))
    }

    fn render_session_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Popover::new("session-settings-popover")
            .open(self.show_menu)
            .appearance(false)
            .overlay_closable(false)
            .anchor(Anchor::TopRight)
            .trigger(
                Button::new("session-settings")
                    .ghost()
                    .icon(IconName::Settings)
                    .debug_selector(|| "session-settings".into())
                    .accessibility_label("设置"),
            )
            .on_open_change(cx.listener(|this, open: &bool, window, cx| {
                this.set_session_menu(*open, window, cx);
            }))
            .content({
                let menu = self.session_menu.clone();
                move |_, _, _| {
                    div()
                        .debug_selector(|| "session-menu".into())
                        .children(menu.clone())
                }
            })
    }

    /// Auto-hide toolbar: a small handle at the top edge that grows into the
    /// full toolbar while hovered.
    fn render_floating_toolbar(
        &self,
        fullscreen: bool,
        width: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .id("float-tb")
                    .occlude()
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                        this.toolbar_open = *hovered || this.apps.picker_open;
                        cx.notify();
                    }))
                    .child(if self.toolbar_open {
                        div()
                            .rounded_b_lg()
                            .shadow_lg()
                            .overflow_hidden()
                            .h(theme::toolbar_height())
                            .child(self.render_toolbar(fullscreen, width, cx))
                            .into_any_element()
                    } else {
                        div()
                            .px_6()
                            .py_1()
                            .rounded_b_md()
                            .bg(theme::card(cx))
                            .text_color(theme::muted(cx))
                            .cursor_pointer()
                            .child(Icon::new(IconName::ChevronDown).text_color(theme::muted(cx)))
                            .into_any_element()
                    }),
            )
    }

    fn session_menu_item(
        &self,
        label: impl Into<SharedString>,
        icon: IconName,
        handler: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &Context<Self>,
    ) -> PopupMenuItem {
        let entity = cx.entity().downgrade();
        PopupMenuItem::new(label)
            .icon(icon)
            .on_click(move |_, window, cx| {
                let _ = entity.update(cx, |this, cx| {
                    this.show_menu = false;
                    this.session_menu = None;
                    handler(this, window, cx);
                    cx.notify();
                });
            })
    }

    fn set_session_menu(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.show_menu = open;
        if !open {
            self.session_menu = None;
            cx.notify();
            return;
        }
        self.mailbox.open = false;
        self.functions.open = false;
        self.notifications.open = false;
        self.info_open = false;
        let live = self.phase == Phase::Connected && !self.view_only();
        let fullscreen = window.is_fullscreen();
        let mut items = vec![
            self.session_menu_item(
                if fullscreen { "退出全屏" } else { "全屏" },
                if fullscreen {
                    IconName::Minimize
                } else {
                    IconName::Maximize
                },
                |this, window, cx| this.toggle_fullscreen(window, cx),
                cx,
            ),
            PopupMenuItem::separator(),
        ];
        for (mode, label) in [
            (ScaleMode::Fit, "适应窗口"),
            (ScaleMode::Actual, "100%"),
            (ScaleMode::Stretch, "拉伸"),
        ] {
            items.push(
                self.session_menu_item(
                    label,
                    IconName::Maximize,
                    move |this, _, cx| this.set_scale(mode, cx),
                    cx,
                )
                .checked(self.scale == mode),
            );
        }
        items.push(PopupMenuItem::separator());
        items.push(
            self.session_menu_item(
                "Ctrl + Alt + Del",
                IconName::SquareTerminal,
                |this, _, _| this.send_cad(),
                cx,
            )
            .disabled(!live),
        );
        items.push(
            self.session_menu_item(
                "发送剪贴板",
                IconName::Copy,
                |this, _, cx| this.send_clipboard(cx),
                cx,
            )
            .disabled(!live),
        );
        items.push(
            self.session_menu_item(
                "文件传输",
                IconName::Folder,
                |this, _, cx| this.open_file_transfer(cx),
                cx,
            )
            .disabled(self.phase != Phase::Connected),
        );
        let files = self.handle.file_client().snapshot();
        let can_upload = live
            && files.caps.is_some_and(|caps| caps.upload)
            && files.listed_path.as_deref() == Some(files.remote_path.as_str());
        items.push(
            self.session_menu_item(
                "上传文件",
                IconName::ArrowUp,
                |this, _, cx| this.quick_upload(cx),
                cx,
            )
            .disabled(!can_upload),
        );
        items.push(self.session_menu_item(
            "助手",
            IconName::SquareTerminal,
            |this, window, cx| this.toggle_assistant(window, cx),
            cx,
        ));
        items.push(self.session_menu_item(
            "连接信息",
            IconName::Info,
            |this, _, _| this.info_open = !this.info_open,
            cx,
        ));
        items.push(
            self.session_menu_item(
                "固定工具栏",
                IconName::Star,
                |this, _, _| {
                    this.pin_toolbar = !this.pin_toolbar;
                    this.toolbar_open = this.pin_toolbar;
                },
                cx,
            )
            .checked(self.pin_toolbar),
        );
        items.push(PopupMenuItem::separator());
        items.push(
            if matches!(self.phase, Phase::Disconnecting | Phase::Ended) {
                self.session_menu_item(
                    "关闭窗口",
                    IconName::WindowClose,
                    |this, window, cx| this.close_window(window, cx),
                    cx,
                )
            } else {
                self.session_menu_item(
                    "断开连接",
                    IconName::WindowClose,
                    |this, _, cx| this.disconnect(cx),
                    cx,
                )
            },
        );
        let menu = PopupMenu::build(window, cx, |menu, _, _| {
            items.into_iter().fold(
                menu.action_context(self.focus.clone())
                    .check_side(gpui_component::Side::Right),
                |menu, item| menu.item(item),
            )
        });
        cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, _, cx| {
            this.show_menu = false;
            this.session_menu = None;
            cx.notify();
        })
        .detach();
        menu.focus_handle(cx).focus(window, cx);
        self.session_menu = Some(menu);
        cx.notify();
    }

    fn render_info(&self, cx: &App) -> impl IntoElement {
        let rows: Vec<(&str, String)> = vec![
            ("设备", self.title.to_string()),
            ("Server", self.host()),
            (
                "Desktop",
                if self.fb_w > 0 {
                    format!("{}×{}", self.fb_w, self.fb_h)
                } else {
                    "—".into()
                },
            ),
            ("Scaling", self.scale.label().into()),
            ("Encryption", self.req.encryption.label().into()),
            ("Quality", self.req.quality.label().into()),
            (
                "Access",
                if self.view_only() {
                    "View only".into()
                } else {
                    "Full control".into()
                },
            ),
        ];
        crate::ui::tool_panel(cx)
            .id("session-info")
            .occlude()
            .absolute()
            .top_3()
            .right_3()
            .children(rows.into_iter().map(|(k, v)| {
                h_flex()
                    .gap_3()
                    .text_xs()
                    .child(div().w(px(70.)).text_color(theme::muted(cx)).child(k))
                    .child(div().child(v))
            }))
            .child(
                div()
                    .text_xs()
                    .text_color(theme::muted(cx))
                    .mt_1()
                    .child(format!(
                        "{} for the session menu",
                        self.menu_key.to_ascii_uppercase()
                    )),
            )
    }

    fn render_picture(&self, cx: &Context<Self>) -> AnyElement {
        let Some(image) = self.render_image.clone() else {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    Icon::new(IconName::Loader)
                        .size_10()
                        .text_color(theme::muted(cx)),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(theme::muted(cx))
                        .child(self.status.clone()),
                )
                .into_any_element();
        };
        let probe = {
            let cell = self.image_box.clone();
            let update_cursor = cx.listener(|this, hovered: &bool, window, cx| {
                this.canvas_hovered = *hovered;
                this.update_local_cursor(window, cx);
            });
            let on_move = cx.listener(|this, hovered: &bool, window, cx| {
                this.pointer_in_window = true;
                this.canvas_hovered = *hovered;
                this.update_local_cursor(window, cx);
            });
            let on_exit = cx.listener(|this, _: &MouseExitEvent, window, cx| {
                // MouseExited can retain the last in-window coordinates. Keep
                // subsequent video frames from hiding the cursor again.
                this.pointer_in_window = false;
                this.update_local_cursor(window, cx);
            });
            canvas(
                move |bounds, window, _| {
                    cell.set(bounds);
                    window.insert_hitbox(bounds, HitboxBehavior::Normal)
                },
                move |_, hitbox, window, cx| {
                    // Track the actual picture hitbox, including occluding UI.
                    // Div::on_hover suppresses hover during dragging/typing,
                    // which would bring the duplicate cursor back mid-session.
                    let move_hitbox = hitbox.clone();
                    window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture {
                            on_move(&move_hitbox.should_handle_scroll(window), window, cx);
                        }
                    });
                    window.on_mouse_event(move |event: &MouseExitEvent, phase, window, cx| {
                        if phase == DispatchPhase::Capture {
                            on_exit(event, window, cx);
                        }
                    });
                    let hitbox = hitbox.clone();
                    window.defer(cx, move |window, cx| {
                        update_cursor(&hitbox.should_handle_scroll(window), window, cx);
                    });
                },
            )
            .absolute()
            .inset_0()
        };
        match self.effective_scale() {
            ScaleMode::Fit | ScaleMode::Stretch => div()
                .relative()
                .size_full()
                .child(
                    img(image)
                        .id("fb")
                        .absolute()
                        .inset_0()
                        .size_full()
                        .object_fit(if self.effective_scale() == ScaleMode::Fit {
                            ObjectFit::Contain
                        } else {
                            ObjectFit::Fill
                        }),
                )
                .child(probe)
                .into_any_element(),
            ScaleMode::Actual => div()
                .id("fb-scroll")
                .size_full()
                .overflow_scroll()
                .child(
                    // Centered when smaller than the viewport, scrollable when larger.
                    div()
                        .min_w_full()
                        .min_h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .relative()
                                .flex_shrink_0()
                                .w(px(f32::from(self.fb_w)))
                                .h(px(f32::from(self.fb_h)))
                                .child(
                                    img(image)
                                        .id("fb")
                                        .absolute()
                                        .inset_0()
                                        .size_full()
                                        .object_fit(ObjectFit::Fill),
                                )
                                .child(probe),
                        ),
                )
                .into_any_element(),
        }
    }
}

impl Focusable for SessionView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Drop for SessionView {
    fn drop(&mut self) {
        // Window closed mid-session: keep the last picture as the preview.
        if self.phase == Phase::Connected {
            self.save_thumb();
        }
    }
}

impl Render for SessionView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_unlock_dialog(window, cx);
        self.fill_received_verification_code(window, cx);
        self.sync_function_notifications(window, cx);
        self.sync_function_result_editor(window, cx);
        if self.control_only {
            let size = window.viewport_size();
            return div()
                .size_full()
                .relative()
                .when(self.functions.open, |el| {
                    el.child(self.render_functions(
                        f32::from(size.width) - 32.,
                        f32::from(size.height) - 80.,
                        cx,
                    ))
                })
                .into_any_element();
        }
        self.update_local_cursor(window, cx);
        self.finish_keyboard_paste(window, cx);
        let width = window.viewport_size().width;
        let height = window.viewport_size().height;
        let fullscreen = window.is_fullscreen();
        self.window_controls.sync(window, theme::is_dark(cx));
        let show_pinned = self.pin_toolbar;
        let remote_keyboard_active = self.phase == Phase::Connected
            && !self.unlock.blocks_input()
            && !self.notifications.open
            && !self.apps.picker_open
            && !self.functions.open
            && !self.mailbox.open
            && self.focus.is_focused(window);

        v_flex()
            .size_full()
            .bg(theme::desktop())
            .key_context("Session")
            .track_focus(&self.focus)
            .child({
                let focus = self.focus.clone();
                canvas(
                    |_, _, _| (),
                    move |_, _, window, cx| {
                        set_platform_ime_disabled(window, remote_keyboard_active);
                        if remote_keyboard_active {
                            window.handle_input(&focus, DisabledIme, cx);
                        }
                    },
                )
                .w(px(0.))
                .h(px(0.))
            })
            .on_key_down(cx.listener(|this, ev, window, cx| this.handle_key(ev, window, cx)))
            .on_key_up(cx.listener(|this, ev, window, cx| this.handle_key_up(ev, window, cx)))
            .on_modifiers_changed(
                cx.listener(|this, ev, window, _| this.handle_modifiers(ev, window)),
            )
            .on_action(cx.listener(|this, _: &SessionFullscreen, window, cx| {
                this.toggle_fullscreen(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SessionDisconnect, _, cx| this.disconnect(cx)))
            .on_action(cx.listener(|this, _: &SessionClose, window, cx| {
                this.close_window(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SessionCad, _, cx| {
                this.send_cad();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SessionScaleCycle, _, cx| {
                let next = this.scale.cycle();
                this.set_scale(next, cx);
            }))
            .on_action(cx.listener(|this, _: &SessionToggleToolbar, _, cx| {
                this.pin_toolbar = !this.pin_toolbar;
                this.toolbar_open = this.pin_toolbar;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SessionMenu, window, cx| {
                this.set_session_menu(!this.show_menu, window, cx);
            }))
            .child(
                TitleBar::new()
                    .h(theme::toolbar_height())
                    .bg(theme::card(cx))
                    .border_color(theme::line(cx))
                    .child(
                        h_flex()
                            .h_full()
                            .flex_1()
                            .min_w_0()
                            .px_2()
                            .items_center()
                            .child(div().text_sm().text_ellipsis().child(self.title.clone()))
                            .child(div().flex_1())
                            .child(
                                h_flex()
                                    .id("session-titlebar-actions")
                                    .items_center()
                                    .when(cfg!(target_os = "windows"), |bar| {
                                        bar.occlude()
                                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                cx.stop_propagation()
                                            })
                                    })
                                    .child(self.render_keyboard_shortcuts(cx))
                                    .when(cfg!(target_os = "windows"), |bar| {
                                        bar.child(self.render_mailbox_launcher(cx))
                                    })
                                    .child(
                                        crate::ui::icon_button(
                                            "files",
                                            IconName::Folder,
                                            "文件传输",
                                        )
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.open_file_transfer(cx)
                                            }),
                                        )
                                        .disabled(self.phase != Phase::Connected),
                                    )
                                    .child(self.render_notification_button(cx))
                                    .child(self.render_session_settings(cx)),
                            ),
                    ),
            )
            .when(show_pinned, |this| {
                this.child(self.render_toolbar(fullscreen, width, cx))
            })
            .child(
                div()
                    .id("stage")
                    .debug_selector(|| "session-stage".into())
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .overflow_hidden()
                    .flex()
                    .child(
                        div()
                            .id("vnc-canvas")
                            .when(
                                self.functions.open
                                    && width >= px(1100.)
                                    && self.scale == ScaleMode::Fit,
                                |canvas| {
                                    canvas.mr(px(if self.function_uses_two_panels() {
                                        772.
                                    } else {
                                        408.
                                    }))
                                },
                            )
                            .relative()
                            .overflow_hidden()
                            .min_h_0()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .bg(theme::desktop())
                            .cursor(CursorStyle::Arrow)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                                    this.focus.focus(window, cx);
                                    if this.show_menu {
                                        this.show_menu = false;
                                        cx.notify();
                                    }
                                    this.button(BTN_LEFT, true, ev.position);
                                }),
                            )
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, ev: &MouseUpEvent, _, _| {
                                    this.button(BTN_LEFT, false, ev.position);
                                }),
                            )
                            .on_mouse_down(
                                MouseButton::Middle,
                                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                                    this.focus.focus(window, cx);
                                    this.button(BTN_MIDDLE, true, ev.position);
                                }),
                            )
                            .on_mouse_up(
                                MouseButton::Middle,
                                cx.listener(|this, ev: &MouseUpEvent, _, _| {
                                    this.button(BTN_MIDDLE, false, ev.position);
                                }),
                            )
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                                    this.focus.focus(window, cx);
                                    this.button(BTN_RIGHT, true, ev.position);
                                }),
                            )
                            .on_mouse_up(
                                MouseButton::Right,
                                cx.listener(|this, ev: &MouseUpEvent, _, _| {
                                    this.button(BTN_RIGHT, false, ev.position);
                                }),
                            )
                            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, _| {
                                this.pointer_at(ev.position);
                            }))
                            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, _| {
                                this.wheel(ev);
                            }))
                            .child(self.render_picture(cx)),
                    )
                    .when(self.assistant_open, |el| {
                        if let Some(assistant) = &self.assistant {
                            el.child(
                                div()
                                    .w(px((f32::from(width) * 0.34).clamp(280., 380.)))
                                    .max_w(relative(0.6))
                                    .h_full()
                                    .flex_shrink_0()
                                    .border_l_1()
                                    .border_color(theme::line(cx))
                                    .child(assistant.clone()),
                            )
                        } else {
                            el
                        }
                    })
                    .when(!show_pinned && self.phase != Phase::Ended, |this| {
                        this.child(self.render_floating_toolbar(fullscreen, width, cx))
                    })
                    .when(self.info_open, |this| this.child(self.render_info(cx)))
                    .when(self.functions.open, |this| {
                        this.child(self.render_functions(
                            f32::from(width),
                            f32::from(height)
                                - f32::from(theme::toolbar_height())
                                - if show_pinned {
                                    f32::from(theme::toolbar_height())
                                } else {
                                    0.
                                },
                            cx,
                        ))
                    })
                    .when(self.notifications.open, |this| {
                        this.child(self.render_notifications(cx))
                    })
                    .when(self.mailbox.open, |this| {
                        this.child(
                            div()
                                .id("mailbox-overlay")
                                .absolute()
                                .top(px(8.))
                                .right(px(8.))
                                .w(px(440f32.min((f32::from(width) - 16.).max(100.))))
                                .max_h(px((f32::from(height) - 100.).max(100.)))
                                .overflow_y_scroll()
                                .child(self.render_mailbox_panel(cx)),
                        )
                    })
                    .when(
                        self.phase == Phase::Ended
                            || (self.recovery.visible
                                && matches!(self.phase, Phase::Connecting | Phase::Disconnecting)),
                        |this| this.child(self.render_recovery(cx)),
                    ),
            )
            .into_any_element()
    }
}

/// Map a point inside the picture box to framebuffer coordinates.
///
/// `box_w`/`box_h` is the element the picture is drawn in; for Fit the image
/// is letterboxed inside it, for Stretch and 1:1 it fills it.
fn map_to_fb(
    local_x: f32,
    local_y: f32,
    box_w: f32,
    box_h: f32,
    fb_w: u16,
    fb_h: u16,
    mode: ScaleMode,
) -> Option<(u16, u16)> {
    if fb_w == 0 || fb_h == 0 || box_w < 1.0 || box_h < 1.0 {
        return None;
    }
    let (dx, dy, dw, dh) = dest_rect(box_w, box_h, fb_w, fb_h, mode);
    if local_x < dx || local_y < dy || local_x >= dx + dw || local_y >= dy + dh {
        return None;
    }
    let fx = (local_x - dx) / dw * fb_w as f32;
    let fy = (local_y - dy) / dh * fb_h as f32;
    Some((
        fx.clamp(0.0, (fb_w as f32 - 1.0).max(0.0)) as u16,
        fy.clamp(0.0, (fb_h as f32 - 1.0).max(0.0)) as u16,
    ))
}

fn dest_rect(
    box_w: f32,
    box_h: f32,
    fb_w: u16,
    fb_h: u16,
    mode: ScaleMode,
) -> (f32, f32, f32, f32) {
    match mode {
        // Stretch fills the box; in 1:1 mode the box *is* the picture.
        ScaleMode::Stretch | ScaleMode::Actual => (0.0, 0.0, box_w, box_h),
        ScaleMode::Fit => {
            let scale = (box_w / fb_w as f32).min(box_h / fb_h as f32);
            let dw = fb_w as f32 * scale;
            let dh = fb_h as f32 * scale;
            ((box_w - dw) * 0.5, (box_h - dh) * 0.5, dw, dh)
        }
    }
}

/// Platform IME client that refuses composition and committed text.
///
/// VNC needs raw key down/up, not composed characters from an input method.
struct DisabledIme;

impl InputHandler for DisabledIme {
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut App,
    ) -> Option<UTF16Selection> {
        None
    }

    fn marked_text_range(&mut self, _: &mut Window, _: &mut App) -> Option<Range<usize>> {
        None
    }

    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<String> {
        None
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        _: &str,
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        _: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut App) {}

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<Bounds<Pixels>> {
        None
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<usize> {
        None
    }

    fn accepts_text_input(&mut self, _: &mut Window, _: &mut App) -> bool {
        false
    }

    fn prefers_ime_for_printable_keys(&mut self, _: &mut Window, _: &mut App) -> bool {
        false
    }

    fn apple_press_and_hold_enabled(&mut self) -> bool {
        false
    }
}

fn set_platform_ime_disabled(window: &Window, disabled: bool) {
    macos_ime::set_disabled(window, disabled);
}

mod local_cursor {
    /// Owns exactly one balanced hide/unhide pair, even if the window is closed
    /// while the pointer is hidden. Repaints and mouse moves must not stack hides.
    #[derive(Default)]
    pub struct LocalCursor {
        hidden: bool,
    }

    impl LocalCursor {
        #[cfg(test)]
        pub(super) fn is_hidden(&self) -> bool {
            self.hidden
        }

        pub fn set_hidden(&mut self, hidden: bool) {
            if self.hidden != hidden {
                set_platform_hidden(hidden);
                self.hidden = hidden;
            }
        }
    }

    impl Drop for LocalCursor {
        fn drop(&mut self) {
            self.set_hidden(false);
        }
    }

    #[cfg(all(target_os = "macos", not(test)))]
    fn set_platform_hidden(hidden: bool) {
        use objc2_app_kit::NSCursor;

        if hidden {
            NSCursor::hide();
        } else {
            NSCursor::unhide();
        }
    }

    #[cfg(all(not(target_os = "macos"), not(test)))]
    fn set_platform_hidden(_: bool) {}

    #[cfg(test)]
    std::thread_local! {
        static TRANSITIONS: std::cell::RefCell<Vec<bool>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    #[cfg(test)]
    fn set_platform_hidden(hidden: bool) {
        TRANSITIONS.with_borrow_mut(|transitions| transitions.push(hidden));
    }

    #[test]
    fn repeated_updates_and_window_close_balance_cursor_hiding() {
        TRANSITIONS.with_borrow_mut(Vec::clear);
        {
            let mut cursor = LocalCursor::default();
            cursor.set_hidden(false);
            cursor.set_hidden(true);
            cursor.set_hidden(true); // Mouse moves and frame updates.
            cursor.set_hidden(false); // Leave the picture or lose focus.
            cursor.set_hidden(false);
            cursor.set_hidden(true); // Return to the remote desktop.
        } // Closing the window must restore the cursor.
        TRANSITIONS.with_borrow(|transitions| {
            assert_eq!(transitions, &[true, false, true, false]);
        });
        drop(LocalCursor::default());
        TRANSITIONS.with_borrow(|transitions| assert_eq!(transitions.len(), 4));
    }
}

#[cfg(test)]
mod ui_tests;

#[cfg(target_os = "macos")]
mod macos_ime {
    use gpui::Window;
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSArray, NSString};
    use raw_window_handle::RawWindowHandle;

    pub fn set_disabled(window: &Window, disabled: bool) {
        let Ok(handle) = raw_window_handle::HasWindowHandle::window_handle(window) else {
            return;
        };
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return;
        };
        let view = appkit.ns_view.as_ptr().cast::<AnyObject>();
        if view.is_null() {
            return;
        }
        unsafe {
            let view = &*view;
            let ctx: *mut AnyObject = msg_send![view, inputContext];
            if ctx.is_null() {
                return;
            }
            let ctx = &*ctx;
            if disabled {
                // Empty locale list prevents CJK/dead-key IMEs from intercepting VNC keys.
                let empty = NSArray::<NSString>::new();
                let _: () = msg_send![ctx, setAllowedInputSourceLocales: &*empty];
                let _: () = msg_send![ctx, discardMarkedText];
            } else {
                // A text field needs the window's normal input sources again.
                let _: () =
                    msg_send![ctx, setAllowedInputSourceLocales: std::ptr::null::<AnyObject>()];
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod macos_ime {
    pub fn set_disabled(_window: &gpui::Window, _disabled: bool) {}
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the `gpui::*` glob would shadow `#[test]`.
    use super::{ScaleMode, map_to_fb};

    #[test]
    fn fit_letterboxes_and_maps_corners() {
        // 200×100 picture in a 400×400 box scales ×2: drawn at y 100..300, x 0..400.
        assert_eq!(
            map_to_fb(0.0, 100.0, 400.0, 400.0, 200, 100, ScaleMode::Fit),
            Some((0, 0))
        );
        assert_eq!(
            map_to_fb(399.0, 299.0, 400.0, 400.0, 200, 100, ScaleMode::Fit),
            Some((199, 99))
        );
        assert_eq!(
            map_to_fb(200.0, 200.0, 400.0, 400.0, 200, 100, ScaleMode::Fit),
            Some((100, 50))
        );
        assert_eq!(
            map_to_fb(10.0, 10.0, 400.0, 400.0, 200, 100, ScaleMode::Fit),
            None
        );
    }

    #[test]
    fn actual_maps_one_to_one() {
        assert_eq!(
            map_to_fb(17.0, 23.0, 200.0, 100.0, 200, 100, ScaleMode::Actual),
            Some((17, 23))
        );
        assert_eq!(
            map_to_fb(200.0, 0.0, 200.0, 100.0, 200, 100, ScaleMode::Actual),
            None
        );
    }

    #[test]
    fn stretch_scales_each_axis() {
        assert_eq!(
            map_to_fb(200.0, 200.0, 400.0, 400.0, 200, 100, ScaleMode::Stretch),
            Some((100, 50))
        );
    }

    #[test]
    fn empty_framebuffer_maps_nothing() {
        assert_eq!(
            map_to_fb(1.0, 1.0, 100.0, 100.0, 0, 0, ScaleMode::Fit),
            None
        );
    }
}

mod apps_ui;
mod assistant_bridge;
mod clipboard_ui;
mod controls_ui;
mod functions_ui;
mod karing_ui;
mod mailbox_service;
mod mailbox_ui;
mod reconnect;
mod unlock_ui;

#[cfg(test)]
mod assistant_visual_tests;

#[cfg(test)]
mod functions_visual_tests;
