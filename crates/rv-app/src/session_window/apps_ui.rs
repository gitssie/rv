use super::*;
use crate::assets::AppActionIcon;
use gpui_component::{
    button::ButtonCustomVariant,
    menu::{ContextMenuExt, PopupMenu, PopupMenuItem},
    popover::Popover,
};
use rv_core::{AppShortcut, move_app_shortcut};
use rv_session::{AppCapabilities, AppCommand, AppEvent, RemoteApp};
use std::collections::{HashMap, HashSet};

struct AppIconImage {
    original: Arc<RenderImage>,
    sizes: std::sync::Mutex<HashMap<u32, Arc<RenderImage>>>,
}

impl AppIconImage {
    fn new(original: Arc<RenderImage>) -> Arc<Self> {
        Arc::new(Self {
            original,
            sizes: std::sync::Mutex::new(HashMap::new()),
        })
    }

    fn for_size(&self, logical_size: f32, scale_factor: f32) -> Arc<RenderImage> {
        let pixels = (logical_size * scale_factor).ceil().max(1.) as u32;
        let mut sizes = self.sizes.lock().unwrap();
        sizes
            .entry(pixels)
            .or_insert_with(|| resample_app_icon(&self.original, pixels))
            .clone()
    }

    fn drop_images(&self, cx: &mut App) {
        cx.drop_image(self.original.clone(), None);
        for image in self.sizes.lock().unwrap().values() {
            cx.drop_image(image.clone(), None);
        }
    }
}

fn resample_app_icon(original: &Arc<RenderImage>, pixels: u32) -> Arc<RenderImage> {
    let size = original.size(0);
    let width = size.width.0 as u32;
    let height = size.height.0 as u32;
    if width == pixels && height == pixels {
        return original.clone();
    }
    // GPUI's atlas uses bilinear filtering without mipmaps. Prefilter to the
    // actual device-pixel size so small compass ticks/gears don't alias.
    // Filter premultiplied alpha to keep transparent edges free of dark halos.
    let bytes = original.as_bytes(0).unwrap();
    let source = ImageBuffer::<Rgba<f32>, Vec<f32>>::from_fn(width, height, |x, y| {
        let offset = ((y * width + x) * 4) as usize;
        let bgra = &bytes[offset..offset + 4];
        let alpha = bgra[3] as f32 / 255.;
        Rgba([
            bgra[0] as f32 / 255. * alpha,
            bgra[1] as f32 / 255. * alpha,
            bgra[2] as f32 / 255. * alpha,
            alpha,
        ])
    });
    let filtered = image::imageops::resize(
        &source,
        pixels,
        pixels,
        image::imageops::FilterType::Lanczos3,
    );
    let output = ImageBuffer::from_fn(pixels, pixels, |x, y| {
        let pixel = filtered.get_pixel(x, y);
        let alpha = pixel[3].clamp(0., 1.);
        if alpha <= f32::EPSILON {
            return Rgba([0, 0, 0, 0]);
        }
        Rgba([
            ((pixel[0] / alpha).clamp(0., 1.) * 255.).round() as u8,
            ((pixel[1] / alpha).clamp(0., 1.) * 255.).round() as u8,
            ((pixel[2] / alpha).clamp(0., 1.) * 255.).round() as u8,
            (alpha * 255.).round() as u8,
        ])
    });
    Arc::new(RenderImage::new(SmallVec::from_elem(
        image::Frame::new(output),
        1,
    )))
}

pub(super) struct AppToolbar {
    pub(super) caps: Option<AppCapabilities>,
    installed: Vec<RemoteApp>,
    shortcuts: Vec<AppShortcut>,
    icons: HashMap<String, Arc<AppIconImage>>,
    fallback_icons: HashMap<String, Arc<AppIconImage>>,
    requested: HashSet<String>,
    busy: HashSet<String>,
    foreground: Option<String>,
    search: Entity<InputState>,
    _search_subscription: Subscription,
    pub picker_open: bool,
    loading: bool,
    page: usize,
    error: Option<String>,
}

impl AppToolbar {
    pub fn new(
        shortcuts: Vec<AppShortcut>,
        window: &mut Window,
        cx: &mut Context<SessionView>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索 App"));
        let subscription = cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.apps.page = 0;
                this.request_app_icons(cx);
                cx.notify();
            }
        });
        let mut seen = HashSet::new();
        let shortcuts = shortcuts
            .into_iter()
            .filter(|s| {
                rv_session::valid_bundle_id(&s.bundle_id) && seen.insert(s.bundle_id.clone())
            })
            .take(32)
            .collect();
        Self {
            caps: None,
            installed: vec![],
            shortcuts,
            icons: HashMap::new(),
            fallback_icons: bundled_app_icons(),
            requested: HashSet::new(),
            busy: HashSet::new(),
            foreground: None,
            search,
            _search_subscription: subscription,
            picker_open: false,
            loading: false,
            page: 0,
            error: None,
        }
    }
    fn icon_for(&self, bundle_id: &str) -> Option<Arc<AppIconImage>> {
        self.icons
            .get(bundle_id)
            .or_else(|| self.fallback_icons.get(bundle_id))
            .cloned()
    }
    pub fn disconnected(&mut self, cx: &mut Context<SessionView>) {
        self.caps = None;
        self.installed.clear();
        self.requested.clear();
        self.busy.clear();
        self.foreground = None;
        self.picker_open = false;
        self.loading = false;
        self.page = 0;
        self.error = None;
        for (_, image) in self.icons.drain() {
            image.drop_images(cx);
        }
    }
}

fn bundled_app_icons() -> HashMap<String, Arc<AppIconImage>> {
    static ICONS: std::sync::OnceLock<HashMap<String, Arc<AppIconImage>>> =
        std::sync::OnceLock::new();
    ICONS
        .get_or_init(|| {
            let assets: [(&str, &[u8]); 6] = [
                (
                    "com.apple.mobilesafari",
                    include_bytes!("../../../../assets/app-shortcuts/safari.png"),
                ),
                (
                    "com.tencent.xin",
                    include_bytes!("../../../../assets/app-shortcuts/wechat.png"),
                ),
                (
                    "com.apple.mobileslideshow",
                    include_bytes!("../../../../assets/app-shortcuts/photos.png"),
                ),
                (
                    "com.apple.Preferences",
                    include_bytes!("../../../../assets/app-shortcuts/settings.png"),
                ),
                (
                    "com.apple.AppStore",
                    include_bytes!("../../../../assets/app-shortcuts/app-store.png"),
                ),
                (
                    "com.hydra.projectx",
                    include_bytes!("../../../../assets/app-shortcuts/projectx.png"),
                ),
            ];
            assets
                .into_iter()
                .map(|(id, bytes)| {
                    // Decode once for the process. Share small BGRA textures across session windows.
                    let mut icon = image::load_from_memory(bytes)
                        .expect("bundled App icon must decode")
                        .into_rgba8();
                    if id == "com.apple.AppStore" {
                        // The macOS ICNS canvas includes 50px of padding/shadow
                        // around its 412px tile. Crop only this bundled fallback
                        // so its artwork fills the same box as the iOS icons.
                        icon = image::imageops::crop_imm(&icon, 50, 50, 412, 412).to_image();
                    }
                    let (width, height) = icon.dimensions();
                    let mut pixels = icon.into_raw();
                    for pixel in pixels.chunks_exact_mut(4) {
                        pixel.swap(0, 2);
                    }
                    let image =
                        ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, pixels).unwrap();
                    (
                        id.to_owned(),
                        AppIconImage::new(Arc::new(RenderImage::new(SmallVec::from_elem(
                            image::Frame::new(image),
                            1,
                        )))),
                    )
                })
                .collect()
        })
        .clone()
}

#[derive(Clone)]
struct ShortcutDrag {
    session: EntityId,
    shortcut: AppShortcut,
    image: Option<Arc<AppIconImage>>,
}
impl Render for ShortcutDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        app_icon(self.image.clone(), 32.).opacity(0.85)
    }
}

fn app_icon(image: Option<Arc<AppIconImage>>, size: f32) -> Div {
    div()
        .size(px(size))
        .rounded(px(size * 0.22))
        .overflow_hidden()
        .when_some(image.clone(), |el, image| {
            el.child(
                img(move |window: &mut Window, _: &mut App| {
                    Some(Ok(image.for_size(size, window.scale_factor())))
                })
                .size_full(),
            )
        })
        .when(image.is_none(), |el| {
            // Unknown Apps use a graphic placeholder, never a letter tile.
            el.bg(rgb(0x42536c))
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(rgb(0xe4edf7))
                .child(Icon::new(IconName::LayoutDashboard).small())
        })
}

fn app_action_button(
    id: &'static str,
    icon: AppActionIcon,
    label: &'static str,
    enabled: bool,
    menu: WeakEntity<PopupMenu>,
    previous_focus: Option<FocusHandle>,
    handler: impl Fn(&mut App) + 'static,
) -> Button {
    let handler = Rc::new(handler);
    let keyboard_handler = handler.clone();
    Button::new(id)
        .ghost()
        .size(px(28.))
        .icon(Icon::new(icon))
        .accessibility_label(label)
        .debug_selector(move || id.into())
        .disabled(!enabled)
        .key_context("AppActionIcon")
        .on_action(move |_: &AppActionActivate, window, cx| {
            cx.stop_propagation();
            if !enabled {
                return;
            }
            keyboard_handler(cx);
            if let Some(menu) = menu.upgrade() {
                menu.update(cx, |_, cx| cx.emit(DismissEvent));
            }
            if let Some(focus) = &previous_focus {
                focus.focus(window, cx);
            }
        })
        .on_click(move |_, _, cx| handler(cx))
}

impl SessionView {
    pub(super) fn apply_app_event(&mut self, event: AppEvent, cx: &mut Context<Self>) {
        match event {
            AppEvent::LockState(locked) => self.unlock_lock_state(locked, cx),
            AppEvent::ScreenState(attempt, state) => self.unlock_screen_state(attempt, state, cx),
            AppEvent::UnlockArmed(attempt, armed) => self.unlock_armed(attempt, armed, cx),
            AppEvent::Capabilities(caps) => {
                self.apps.loading = caps.list;
                self.apps.caps = Some(caps);
            }
            AppEvent::List(apps) => {
                self.apps.installed = apps;
                self.apps.page = 0;
                self.apps.loading = false;
                self.apps.error = None;
            }
            AppEvent::Foreground(id) => self.apps.foreground = id,
            AppEvent::Icon {
                bundle_id,
                width,
                height,
                bgra,
            } => {
                if let Some(buf) = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, bgra) {
                    let icon = AppIconImage::new(Arc::new(RenderImage::new(SmallVec::from_elem(
                        image::Frame::new(buf),
                        1,
                    ))));
                    let wanted = self.wanted_app_icon_ids(cx);
                    if self.apps.icons.len() >= 160
                        && let Some(evict) = self
                            .apps
                            .icons
                            .keys()
                            .find(|id| !wanted.contains(id))
                            .cloned()
                    {
                        if let Some(old) = self.apps.icons.remove(&evict) {
                            old.drop_images(cx);
                        }
                        self.apps.requested.remove(&evict);
                    }
                    if let Some(old) = self.apps.icons.insert(bundle_id, icon) {
                        old.drop_images(cx);
                    }
                }
            }
            AppEvent::Finished(command) => {
                if let Some(id) = command.bundle_id() {
                    self.apps.busy.remove(id);
                }
                if !matches!(command, AppCommand::Icon(_)) {
                    self.apps.error = None;
                }
            }
            AppEvent::Failed { command, message } => {
                if matches!(
                    command,
                    AppCommand::ScreenState(_)
                        | AppCommand::PrepareUnlock(_)
                        | AppCommand::ArmUnlock(_, _)
                        | AppCommand::LockScreen(_)
                ) {
                    self.unlock_failed(&command, message, cx);
                    return;
                }
                if let Some(id) = command.bundle_id() {
                    self.apps.busy.remove(id);
                }
                if matches!(command, AppCommand::List) {
                    self.apps.loading = false;
                }
                // Missing icons and unavailable foreground queries should not obscure user actions.
                if !matches!(command, AppCommand::Icon(_) | AppCommand::Foreground) {
                    self.status = message.clone().into();
                    self.apps.error = Some(message);
                }
            }
        }
        self.request_app_icons(cx);
        cx.notify();
    }

    fn app_allowed(&self, id: &str, action: u8) -> bool {
        if self.phase != Phase::Connected
            || self.view_only()
            || self.unlock.blocks_input()
            || self.apps.busy.contains(id)
        {
            return false;
        }
        let Some(caps) = &self.apps.caps else {
            return false;
        };
        let Some(app) = self.apps.installed.iter().find(|app| app.bundle_id == id) else {
            return false;
        };
        caps.control
            && match action {
                2 => caps.launch && app.can_launch,
                3 => caps.terminate && app.can_terminate,
                4 => caps.restart && app.can_launch && app.can_terminate,
                _ => false,
            }
    }
    fn dispatch_app(&mut self, command: AppCommand, cx: &mut Context<Self>) {
        let action = match command {
            AppCommand::Launch(_) => 2,
            AppCommand::Terminate(_) => 3,
            AppCommand::Restart(_) => 4,
            _ => return,
        };
        let id = command.bundle_id().unwrap().to_owned();
        if !self.app_allowed(&id, action) {
            return;
        }
        self.apps.error = None;
        self.apps.busy.insert(id);
        self.handle.app(command);
        cx.notify();
    }
    fn wanted_app_icon_ids(&self, cx: &Context<Self>) -> Vec<String> {
        let query = self.apps.search.read(cx).value().to_lowercase();
        let mut ids: Vec<_> = self
            .apps
            .shortcuts
            .iter()
            .map(|a| a.bundle_id.clone())
            .collect();
        if self.apps.picker_open {
            ids.extend(
                self.apps
                    .installed
                    .iter()
                    .filter(|a| {
                        a.name.to_lowercase().contains(&query)
                            || a.bundle_id.to_lowercase().contains(&query)
                    })
                    .skip(self.apps.page * 60)
                    .take(60)
                    .map(|a| a.bundle_id.clone()),
            );
        }
        ids
    }
    fn refresh_apps(&mut self, cx: &mut Context<Self>) {
        if self.apps.loading || !self.apps.caps.as_ref().is_some_and(|c| c.list) {
            return;
        }
        self.apps
            .requested
            .retain(|id| self.apps.icons.contains_key(id));
        self.apps.loading = true;
        self.apps.error = None;
        self.handle.app(AppCommand::List);
        cx.notify();
    }
    fn request_app_icons(&mut self, cx: &Context<Self>) {
        if !self.apps.caps.as_ref().is_some_and(|c| c.icons) {
            return;
        }
        let ids = self.wanted_app_icon_ids(cx);
        for id in ids {
            if self.apps.installed.iter().any(|a| a.bundle_id == id)
                && self.apps.requested.insert(id.clone())
            {
                self.handle.app(AppCommand::Icon(id));
            }
        }
    }
    fn persist_app_shortcuts(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.req.connection_id
            && let Some(book) = self.address_book.as_ref().and_then(|b| b.upgrade())
            && let Err(error) = book.update(cx, |book, cx| {
                book.remember_app_shortcuts(id, self.apps.shortcuts.clone(), cx)
            })
        {
            self.status = error.clone().into();
            self.apps.error = Some(error);
        }
    }
    fn remove_app_shortcut(&mut self, id: &str, cx: &mut Context<Self>) {
        self.apps.shortcuts.retain(|a| a.bundle_id != id);
        self.persist_app_shortcuts(cx);
        cx.notify();
    }
    fn toggle_app_shortcut(&mut self, app: RemoteApp, cx: &mut Context<Self>) {
        if self
            .apps
            .shortcuts
            .iter()
            .any(|a| a.bundle_id == app.bundle_id)
        {
            self.remove_app_shortcut(&app.bundle_id, cx);
            return;
        }
        if self.apps.shortcuts.len() >= 32 {
            self.apps.error = Some("最多添加 32 个快捷 App".into());
            cx.notify();
            return;
        }
        self.apps.shortcuts.push(AppShortcut {
            bundle_id: app.bundle_id,
            name: app.name,
        });
        self.persist_app_shortcuts(cx);
        self.request_app_icons(cx);
        cx.notify();
    }
    fn drop_app_shortcut(
        &mut self,
        drag: &ShortcutDrag,
        before: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        if drag.session == cx.entity_id()
            && move_app_shortcut(&mut self.apps.shortcuts, &drag.shortcut.bundle_id, before)
        {
            self.persist_app_shortcuts(cx);
            cx.notify();
        }
    }
    pub(super) fn render_keyboard_shortcuts(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        Popover::new("keyboard-shortcuts")
            .anchor(Anchor::TopLeft)
            .trigger(
                Button::new("keyboard-menu")
                    .ghost()
                    .icon(IconName::SquareTerminal)
                    .tooltip("键盘快捷操作")
                    .text_color(theme::toolbar_fg()),
            )
            .content(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    v_flex()
                        .w(px(170.))
                        .gap_1()
                        .child(menu_item(
                            "compact-cad",
                            IconName::SquareTerminal,
                            "Ctrl + Alt + Del",
                            cx.listener(|this, _, _, _| this.send_cad()),
                        ))
                        .children(
                            [
                                ("Ctrl", XK_CONTROL_L),
                                ("Alt", XK_ALT_L),
                                ("Win", rv_core::XK_SUPER_L),
                                ("Tab", rv_core::XK_TAB),
                                ("Esc", rv_core::XK_ESCAPE),
                                ("Caps", rv_core::XK_CAPS_LOCK),
                            ]
                            .into_iter()
                            .map(|(label, key)| {
                                Button::new(label)
                                    .ghost()
                                    .label(label)
                                    .disabled(this.phase != Phase::Connected || this.view_only())
                                    .on_click(cx.listener(move |this, _, _, _| this.tap_key(key)))
                            }),
                        )
                        .into_any_element()
                })
            })
    }
    pub(super) fn render_app_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let content_entity = entity.clone();
        let open_entity = entity.clone();
        h_flex()
            .min_w_0()
            .flex_shrink(1.)
            .items_center()
            .gap_1()
            .child(toolbar_sep())
            .child(
                h_flex()
                    .id("app-shortcuts-strip")
                    .min_w_0()
                    .max_w(px(360.))
                    .overflow_x_scroll()
                    .gap_1()
                    .children(self.apps.shortcuts.iter().map(|shortcut| {
                        let id = shortcut.bundle_id.clone();
                        let launch_id = id.clone();
                        let drop_id = id.clone();
                        let menu_id = id.clone();
                        let menu_entity = entity.clone();
                        let can_restart = self.app_allowed(&id, 4);
                        let can_terminate = self.app_allowed(&id, 3);
                        let busy = self.apps.busy.contains(&id);
                        let drag = ShortcutDrag {
                            session: cx.entity_id(),
                            shortcut: shortcut.clone(),
                            image: self.apps.icon_for(&id),
                        };
                        div()
                            .id(SharedString::from(format!("app-shortcut-{id}")))
                            .relative()
                            .flex_shrink_0()
                            .debug_selector(move || format!("shortcut-{}", shortcut.bundle_id))
                            .w(px(34.))
                            .h(px(36.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0x35404e)))
                            .child(app_icon(self.apps.icon_for(&id), 28.).opacity(if busy {
                                0.6
                            } else {
                                1.
                            }))
                            .when(self.apps.foreground.as_deref() == Some(&id), |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .bottom_0()
                                        .size(px(3.))
                                        .rounded_full()
                                        .bg(rgb(0x8abfff)),
                                )
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.dispatch_app(AppCommand::Launch(launch_id.clone()), cx)
                            }))
                            .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
                            .on_drop(cx.listener(move |this, drag: &ShortcutDrag, _, cx| {
                                this.drop_app_shortcut(drag, Some(&drop_id), cx)
                            }))
                            .context_menu(move |menu, window, cx| {
                                let restart = menu_entity.clone();
                                let terminate = menu_entity.clone();
                                let remove = menu_entity.clone();
                                let restart_id = menu_id.clone();
                                let terminate_id = menu_id.clone();
                                let remove_id = menu_id.clone();
                                let menu_view = cx.entity().downgrade();
                                let previous_focus = window.focused(cx);
                                // A compact row of icon buttons. Clicks bubble to the
                                // containing menu item, which dismisses and restores focus.
                                menu.min_w(px(120.))
                                    .max_w(px(120.))
                                    .item(PopupMenuItem::element(move |_, _| {
                                        let restart = restart.clone();
                                        let terminate = terminate.clone();
                                        let remove = remove.clone();
                                        let restart_id = restart_id.clone();
                                        let terminate_id = terminate_id.clone();
                                        let remove_id = remove_id.clone();
                                        h_flex()
                                            .id("app-actions")
                                            .debug_selector(|| "app-actions".into())
                                            .gap_1()
                                            .child(app_action_button(
                                                "restart-app",
                                                AppActionIcon::Restart,
                                                "重启 App",
                                                can_restart,
                                                menu_view.clone(),
                                                previous_focus.clone(),
                                                move |cx| {
                                                    restart.update(cx, |this, cx| {
                                                        this.dispatch_app(
                                                            AppCommand::Restart(restart_id.clone()),
                                                            cx,
                                                        )
                                                    });
                                                },
                                            ))
                                            .child(app_action_button(
                                                "terminate-app",
                                                AppActionIcon::Terminate,
                                                "关闭 App",
                                                can_terminate,
                                                menu_view.clone(),
                                                previous_focus.clone(),
                                                move |cx| {
                                                    terminate.update(cx, |this, cx| {
                                                        this.dispatch_app(
                                                            AppCommand::Terminate(
                                                                terminate_id.clone(),
                                                            ),
                                                            cx,
                                                        )
                                                    });
                                                },
                                            ))
                                            .child(app_action_button(
                                                "remove-app-shortcut",
                                                AppActionIcon::RemoveShortcut,
                                                "移出快捷栏",
                                                true,
                                                menu_view.clone(),
                                                previous_focus.clone(),
                                                move |cx| {
                                                    remove.update(cx, |this, cx| {
                                                        this.remove_app_shortcut(&remove_id, cx)
                                                    });
                                                },
                                            ))
                                    }))
                            })
                    })),
            )
            .child(
                div()
                    .id("append-app-shortcut")
                    .debug_selector(|| "app-plus".into())
                    .flex_shrink_0()
                    .on_drop(cx.listener(|this, drag: &ShortcutDrag, _, cx| {
                        this.drop_app_shortcut(drag, None, cx)
                    }))
                    .child(
                        Popover::new("app-picker-popover")
                            .anchor(Anchor::TopRight)
                            .appearance(false)
                            .trigger(
                                Button::new("add-app-shortcut")
                                    .custom(
                                        ButtonCustomVariant::new(cx)
                                            .foreground(theme::toolbar_muted())
                                            .hover(rgb(0x2a343f).into())
                                            .active(rgb(0x35404e).into()),
                                    )
                                    .icon(IconName::Plus)
                                    .text_color(theme::toolbar_muted()),
                            )
                            .open(self.apps.picker_open)
                            .on_open_change(move |open, window, cx| {
                                open_entity.update(cx, |this, cx| {
                                    this.apps.picker_open = *open;
                                    this.toolbar_open = *open || this.pin_toolbar;
                                    if *open {
                                        let keys = this.keys.release_all();
                                        this.send_keys(keys);
                                        this.apps
                                            .search
                                            .update(cx, |search, cx| search.focus(window, cx));
                                        if this.apps.caps.as_ref().is_some_and(|c| c.list) {
                                            this.apps.loading = true;
                                            this.handle.app(AppCommand::List);
                                        }
                                        this.request_app_icons(cx);
                                    } else {
                                        this.focus.focus(window, cx);
                                    }
                                    cx.notify();
                                });
                            })
                            .content(move |_, _, cx| {
                                content_entity.update(cx, |this, cx| {
                                    this.render_app_picker(cx).into_any_element()
                                })
                            }),
                    ),
            )
    }
    fn render_app_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.apps.search.read(cx).value().to_lowercase();
        let apps: Vec<_> = self
            .apps
            .installed
            .iter()
            .filter(|a| {
                a.name.to_lowercase().contains(&query)
                    || a.bundle_id.to_lowercase().contains(&query)
            })
            .collect();
        let message = if self.phase != Phase::Connected {
            Some("未连接")
        } else if self.apps.caps.is_none() {
            None
        } else if !self.apps.caps.as_ref().is_some_and(|c| c.list) {
            Some("App 列表不可用")
        } else if self.apps.loading && apps.is_empty() {
            Some("加载中…")
        } else if apps.is_empty() {
            Some("没有找到 App")
        } else {
            None
        };
        v_flex()
            .id("app-picker")
            .debug_selector(|| "app-picker".into())
            .occlude()
            .w(px(360.))
            .p_3()
            .gap_2()
            .rounded_lg()
            .bg(theme::toolbar())
            .border_1()
            .border_color(theme::toolbar_line())
            .shadow_lg()
            .text_color(theme::toolbar_fg())
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.apps.search)
                                .small()
                                .appearance(false)
                                .bg(rgb(0x29323e))
                                .text_color(theme::toolbar_fg())
                                .rounded_md()
                                .border_1()
                                .border_color(theme::toolbar_line())
                                .prefix(
                                    Icon::new(IconName::Search)
                                        .small()
                                        .text_color(theme::toolbar_muted()),
                                ),
                        ),
                    )
                    .when(self.apps.caps.as_ref().is_some_and(|c| c.list), |row| {
                        row.child(
                            Button::new("refresh-apps")
                                .ghost()
                                .small()
                                .icon(IconName::Redo)
                                .text_color(theme::toolbar_muted())
                                .disabled(self.apps.loading)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.refresh_apps(cx);
                                })),
                        )
                    }),
            )
            .when_some(self.apps.error.clone(), |el, error| {
                el.child(div().text_xs().text_color(rgb(0xff9c9c)).child(error))
            })
            .when_some(message, |el, message| {
                el.child(
                    div()
                        .py_4()
                        .text_center()
                        .text_xs()
                        .text_color(theme::toolbar_muted())
                        .child(message),
                )
            })
            .child(
                h_flex()
                    .id("installed-app-grid")
                    .flex_wrap()
                    .items_start()
                    .max_h(px(300.))
                    .overflow_y_scroll()
                    .children(apps.iter().skip(self.apps.page * 60).take(60).map(|app| {
                        let app = (*app).clone();
                        let selected = self
                            .apps
                            .shortcuts
                            .iter()
                            .any(|s| s.bundle_id == app.bundle_id);
                        let click_app = app.clone();
                        v_flex()
                            .id(SharedString::from(format!("picker-app-{}", app.bundle_id)))
                            .relative()
                            .w(px(80.))
                            .h(px(80.))
                            .gap_2()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0x35404e)))
                            .child(app_icon(self.apps.icon_for(&app.bundle_id), 38.))
                            .child(
                                div()
                                    .w(px(70.))
                                    .text_center()
                                    .text_xs()
                                    .text_ellipsis()
                                    .child(app.name),
                            )
                            .when(selected, |el| {
                                el.child(
                                    div()
                                        .absolute()
                                        .top_1()
                                        .right_1()
                                        .text_color(rgb(0x8abfff))
                                        .child(Icon::new(IconName::Check).small()),
                                )
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_app_shortcut(click_app.clone(), cx)
                            }))
                    })),
            )
            .when(apps.len() > 60, |el| {
                el.child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .child(
                            Button::new("apps-previous")
                                .ghost()
                                .label("上一页")
                                .disabled(self.apps.page == 0)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.apps.page = this.apps.page.saturating_sub(1);
                                    this.request_app_icons(cx);
                                    cx.notify();
                                })),
                        )
                        .child(div().text_xs().child(format!(
                            "{} / {}",
                            self.apps.page + 1,
                            apps.len().div_ceil(60)
                        )))
                        .child(
                            Button::new("apps-next")
                                .ghost()
                                .label("下一页")
                                .disabled((self.apps.page + 1) * 60 >= apps.len())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.apps.page += 1;
                                    this.request_app_icons(cx);
                                    cx.notify();
                                })),
                        ),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::super::ui_tests::setup;
    use super::{
        AppCapabilities, AppCommand, AppEvent, AppIconImage, AppShortcut, RemoteApp, SessionView,
        ShortcutDrag, resample_app_icon,
    };
    use gpui::{
        Entity, Modifiers, MouseButton, MouseDownEvent, RenderImage, TestAppContext,
        VisualTestContext, px, size,
    };
    use image::{ImageBuffer, Rgba};
    use rv_session::{SessionCommand, SessionTestPeer};
    use smallvec::SmallVec;
    use std::sync::Arc;

    fn enable(view: &Entity<SessionView>, cx: &mut VisualTestContext) {
        view.update(cx, |view, cx| {
            // A user-added third-party shortcut, independent of the system defaults.
            if !view
                .apps
                .shortcuts
                .iter()
                .any(|app| app.bundle_id == "com.tencent.xin")
            {
                view.apps.shortcuts.insert(
                    1,
                    AppShortcut {
                        bundle_id: "com.tencent.xin".into(),
                        name: "微信".into(),
                    },
                );
            }
            view.apply_app_event(
                AppEvent::Capabilities(AppCapabilities {
                    list: true,
                    launch: true,
                    terminate: true,
                    restart: true,
                    control: true,
                    ..Default::default()
                }),
                cx,
            );
            view.apply_app_event(
                AppEvent::List(vec![RemoteApp {
                    bundle_id: "com.tencent.xin".into(),
                    name: "微信".into(),
                    can_launch: true,
                    can_terminate: true,
                }]),
                cx,
            );
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
    fn commands(peer: &mut SessionTestPeer) -> Vec<SessionCommand> {
        std::iter::from_fn(|| peer.commands.try_recv().ok()).collect()
    }
    fn open_app_actions(cx: &mut VisualTestContext) {
        let bounds = cx.debug_bounds("shortcut-com.tencent.xin").unwrap();
        cx.simulate_event(MouseDownEvent {
            button: MouseButton::Right,
            position: bounds.center(),
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
    #[gpui::test]
    fn offscreen_app_action_icons_dispatch_once_and_dismiss(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        for (selector, command) in [
            ("restart-app", AppCommand::Restart("com.tencent.xin".into())),
            (
                "terminate-app",
                AppCommand::Terminate("com.tencent.xin".into()),
            ),
        ] {
            open_app_actions(cx);
            let row = cx.debug_bounds("app-actions").unwrap();
            assert!(row.size.width <= px(100.) && row.size.height <= px(32.));
            let bounds = cx.debug_bounds(selector).unwrap();
            cx.simulate_click(bounds.center(), Modifiers::default());
            let sent = commands(&mut peer);
            assert_eq!(sent.len(), 1, "icon click must send one command");
            assert!(matches!(&sent[0], SessionCommand::App(sent) if sent == &command));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(
                cx.debug_bounds("app-actions").is_none(),
                "action menu must close"
            );
            view.update(cx, |view, cx| {
                view.apply_app_event(AppEvent::Finished(command), cx)
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }
        open_app_actions(cx);
        let remove = cx.debug_bounds("remove-app-shortcut").unwrap();
        cx.simulate_click(remove.center(), Modifiers::default());
        assert!(
            commands(&mut peer).is_empty(),
            "removal must not close/uninstall App"
        );
        view.read_with(cx, |view, _| {
            assert!(
                !view
                    .apps
                    .shortcuts
                    .iter()
                    .any(|s| s.bundle_id == "com.tencent.xin")
            );
        });
    }
    #[gpui::test]
    fn offscreen_app_action_icons_respect_read_only(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, true);
        enable(&view, cx);
        open_app_actions(cx);
        for selector in ["restart-app", "terminate-app"] {
            let bounds = cx.debug_bounds(selector).unwrap();
            cx.simulate_click(bounds.center(), Modifiers::default());
            assert!(commands(&mut peer).is_empty());
        }
        let remove = cx.debug_bounds("remove-app-shortcut").unwrap();
        cx.simulate_click(remove.center(), Modifiers::default());
        view.read_with(cx, |view, _| {
            assert!(
                !view
                    .apps
                    .shortcuts
                    .iter()
                    .any(|s| s.bundle_id == "com.tencent.xin")
            );
        });
    }
    #[gpui::test]
    fn offscreen_app_action_icons_support_keyboard_activation(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        open_app_actions(cx);
        cx.simulate_keystrokes("tab enter");
        let sent = commands(&mut peer);
        assert_eq!(sent.len(), 1);
        assert!(
            matches!(&sent[0], SessionCommand::App(AppCommand::Restart(id)) if id == "com.tencent.xin")
        );
    }
    #[test]
    fn app_icon_minification_filters_fine_lines_and_preserves_alpha_edges() {
        let stripes = ImageBuffer::from_fn(512, 512, |x, _| {
            let value = if x % 2 == 0 { 0 } else { 255 };
            Rgba([value, value, value, 255])
        });
        let icon = AppIconImage::new(Arc::new(RenderImage::new(SmallVec::from_elem(
            image::Frame::new(stripes),
            1,
        ))));
        for scale in [1., 2.] {
            let filtered = icon.for_size(28., scale);
            assert_eq!(filtered.size(0).width.0, (28. * scale) as i32);
            assert!(Arc::ptr_eq(&filtered, &icon.for_size(28., scale)));
            let side = filtered.size(0).width.0 as usize;
            // Dense alternating lines must average to gray, not turn into moire.
            for y in 2..side - 2 {
                for x in 2..side - 2 {
                    let pixel = &filtered.as_bytes(0).unwrap()[(y * side + x) * 4..];
                    assert!(
                        (120..=135).contains(&pixel[0]),
                        "aliased pixel: {}",
                        pixel[0]
                    );
                    assert_eq!(pixel[3], 255);
                }
            }
        }
        let alpha_edge = ImageBuffer::from_fn(128, 128, |x, _| {
            if x < 64 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 0, 0])
            }
        });
        let source = Arc::new(RenderImage::new(SmallVec::from_elem(
            image::Frame::new(alpha_edge),
            1,
        )));
        let filtered = resample_app_icon(&source, 28);
        let mut partial_alpha = false;
        for pixel in filtered.as_bytes(0).unwrap().chunks_exact(4) {
            if (1..255).contains(&pixel[3]) {
                partial_alpha = true;
                assert_eq!(pixel[0], 255, "transparent edge darkened");
            }
        }
        assert!(partial_alpha);
    }
    #[gpui::test]
    fn offscreen_app_icons_exist_before_capabilities_and_remote_icons_take_priority(
        cx: &mut TestAppContext,
    ) {
        let (view, cx, _) = setup(cx, false);
        view.update(cx, |view, cx| {
            assert!(view.apps.caps.is_none());
            for shortcut in &view.apps.shortcuts {
                assert!(
                    view.apps.icon_for(&shortcut.bundle_id).is_some(),
                    "{} must have a bundled icon",
                    shortcut.bundle_id
                );
            }
            let store_icon = view
                .apps
                .icon_for("com.apple.AppStore")
                .unwrap()
                .for_size(28., 1.);
            let bytes = store_icon.as_bytes(0).unwrap();
            let visible_width = (0..28)
                .filter(|x| bytes[(14 * 28 + x) * 4 + 3] >= 128)
                .count();
            assert!(
                visible_width >= 26,
                "App Store artwork must fill the toolbar icon: {visible_width}px"
            );
            let fallback = view.apps.icon_for("com.tencent.xin").unwrap();
            view.apply_app_event(
                AppEvent::Icon {
                    bundle_id: "com.tencent.xin".into(),
                    width: 1,
                    height: 1,
                    bgra: vec![0, 255, 0, 255],
                },
                cx,
            );
            let remote = view.apps.icon_for("com.tencent.xin").unwrap();
            assert!(!std::sync::Arc::ptr_eq(&fallback, &remote));
            view.apps.disconnected(cx);
            assert!(std::sync::Arc::ptr_eq(
                &fallback,
                &view.apps.icon_for("com.tencent.xin").unwrap()
            ));
        });
    }
    #[gpui::test]
    fn offscreen_app_launch_duplicate_guard_and_read_only(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        let bounds = cx
            .debug_bounds("shortcut-com.tencent.xin")
            .expect("App icon must be visible");
        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.simulate_click(bounds.center(), Modifiers::default());
        let commands = commands(&mut peer);
        assert_eq!(commands.len(), 1);
        assert!(
            matches!(&commands[0], SessionCommand::App(AppCommand::Launch(id)) if id == "com.tencent.xin")
        );
        view.update(cx, |view, cx| {
            view.apply_app_event(
                AppEvent::Finished(AppCommand::Launch("com.tencent.xin".into())),
                cx,
            );
            view.req.view_only = true;
            view.dispatch_app(AppCommand::Terminate("com.tencent.xin".into()), cx);
        });
        assert!(peer.commands.try_recv().is_err());
    }
    #[gpui::test]
    fn offscreen_app_picker_search_does_not_send_remote_keys(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        let bounds = cx.debug_bounds("app-plus").unwrap();
        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("app-picker").is_some());
        cx.simulate_input("微信");
        cx.simulate_keystrokes("backspace");
        assert!(
            commands(&mut peer)
                .iter()
                .all(|c| !matches!(c, SessionCommand::Input(_)))
        );
        view.read_with(cx, |view, cx| {
            assert!(!view.apps.search.read(cx).value().is_empty())
        });
    }
    #[gpui::test]
    fn offscreen_app_refresh_retries_failed_icons(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        view.update(cx, |view, cx| {
            view.apps.caps.as_mut().unwrap().icons = true;
            view.request_app_icons(cx);
        });
        assert!(commands(&mut peer).iter().any(
            |c| matches!(c, SessionCommand::App(AppCommand::Icon(id)) if id == "com.tencent.xin")
        ));
        view.update(cx, |view, cx| {
            view.apply_app_event(
                AppEvent::Failed {
                    command: AppCommand::Icon("com.tencent.xin".into()),
                    message: "busy".into(),
                },
                cx,
            );
        });
        assert!(
            commands(&mut peer).is_empty(),
            "must not loop on a failed icon automatically"
        );
        view.update(cx, |view, cx| {
            view.refresh_apps(cx);
            let apps = view.apps.installed.clone();
            view.apply_app_event(AppEvent::List(apps), cx);
        });
        assert!(commands(&mut peer).iter().any(
            |c| matches!(c, SessionCommand::App(AppCommand::Icon(id)) if id == "com.tencent.xin")
        ));
    }
    #[gpui::test]
    fn offscreen_app_favorites_reorder_and_disconnect(cx: &mut TestAppContext) {
        let (view, cx, mut peer) = setup(cx, false);
        enable(&view, cx);
        view.update(cx, |view, cx| {
            let drag = ShortcutDrag {
                session: cx.entity_id(),
                shortcut: view.apps.shortcuts[1].clone(),
                image: None,
            };
            let before = view.apps.shortcuts[0].bundle_id.clone();
            view.drop_app_shortcut(&drag, Some(&before), cx);
            assert_eq!(view.apps.shortcuts[0].bundle_id, "com.tencent.xin");
            view.remove_app_shortcut("com.tencent.xin", cx);
            assert!(
                !view
                    .apps
                    .shortcuts
                    .iter()
                    .any(|s| s.bundle_id == "com.tencent.xin")
            );
            let app = view.apps.installed[0].clone();
            view.toggle_app_shortcut(app, cx);
            assert_eq!(
                view.apps.shortcuts.last().unwrap().bundle_id,
                "com.tencent.xin"
            );
            view.apps.disconnected(cx);
            view.dispatch_app(AppCommand::Launch("com.tencent.xin".into()), cx);
            assert!(view.apps.caps.is_none());
        });
        assert!(commands(&mut peer).is_empty());
        cx.simulate_resize(size(px(640.), px(400.)));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        for selector in ["app-plus", "disconnect-control"] {
            let bounds = cx.debug_bounds(selector).unwrap();
            assert!(
                bounds.right() <= px(640.),
                "{selector} must remain inside narrow window: {bounds:?}"
            );
            assert!(bounds.left() >= px(0.));
        }
    }
}
