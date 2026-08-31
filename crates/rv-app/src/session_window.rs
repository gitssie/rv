use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    Icon, IconName, Selectable, Sizable, StyledExt, TitleBar,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::DropdownMenu,
    v_flex,
};
use image::{ImageBuffer, Rgba};
use smallvec::SmallVec;

use rv_core::{CAD_KEYSYMS, ConnectRequest, Keyboard, ScaleMode, XK_ALT_L, XK_CONTROL_L};
use rv_session::{SessionEvent, SessionHandle};

use crate::actions::*;
use crate::theme;

pub struct SessionOptions {
    pub scale: ScaleMode,
    pub pin_toolbar: bool,
    pub menu_key: String,
    pub hide_shots: bool,
    pub thumb_path: Option<PathBuf>,
}

pub fn open(
    req: ConnectRequest,
    title: String,
    session: SessionOptions,
    _window: &mut Window,
    cx: &mut App,
) {
    // Open after the current window update finishes. Nesting `open_window`
    // inside another window's constructor tears the session window down.
    cx.spawn(async move |cx| {
        let mut window_options = TitleBar::window_options();
        window_options.window_bounds = Some(WindowBounds::Windowed(Bounds {
            origin: point(px(72.), px(64.)),
            size: size(px(1280.), px(800.)),
        }));
        window_options.window_min_size = Some(size(px(640.), px(400.)));
        window_options.titlebar = Some(TitlebarOptions {
            title: Some(title.clone().into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(9.), px(9.))),
        });
        let _ = cx.open_window(window_options, move |window, cx| {
            let view = cx.new(|cx| SessionView::new(req, title, session, window, cx));
            cx.new(|cx| gpui_component::Root::new(view, window, cx))
        });
    })
    .detach();
}

pub struct SessionView {
    handle: SessionHandle,
    title: SharedString,
    host: SharedString,
    status: SharedString,
    scale: ScaleMode,
    pin_toolbar: bool,
    toolbar_open: bool,
    menu_key: String,
    hide_shots: bool,
    thumb_path: Option<PathBuf>,
    view_only: bool,
    fullscreen: bool,
    show_menu: bool,
    info_open: bool,
    buttons: u8,
    last_generation: u64,
    render_image: Option<Arc<RenderImage>>,
    fb_w: u16,
    fb_h: u16,
    error: Option<SharedString>,
    keys: Keyboard,
    focus: FocusHandle,
}

impl SessionView {
    fn new(
        req: ConnectRequest,
        title: String,
        options: SessionOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let host = format!("{}:{}", req.host, req.port);
        let view_only = req.view_only;
        let handle = SessionHandle::spawn(req);
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let SessionOptions {
            scale,
            pin_toolbar,
            menu_key,
            hide_shots,
            thumb_path,
        } = options;
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

        Self {
            handle,
            title: title.into(),
            host: host.into(),
            status: "Connecting…".into(),
            scale,
            pin_toolbar,
            toolbar_open: true,
            menu_key,
            hide_shots,
            thumb_path,
            view_only,
            fullscreen: false,
            show_menu: false,
            info_open: false,
            buttons: 0,
            last_generation: 0,
            render_image: None,
            fb_w: 0,
            fb_h: 0,
            error: None,
            keys: Keyboard::new(),
            focus,
        }
    }

    fn pump(&mut self, cx: &mut Context<Self>) {
        // Rebuild the GPU image at most once per tick: each rebuild copies the
        // whole framebuffer on the UI thread, and this thread also delivers key
        // events. Stalling it delays key-ups, which the remote turns into
        // auto-repeat.
        let mut frame = None;
        for ev in self.handle.drain() {
            match ev {
                SessionEvent::Status(s) => self.status = s.into(),
                SessionEvent::Connected {
                    width,
                    height,
                    name,
                } => {
                    self.fb_w = width;
                    self.fb_h = height;
                    if !name.is_empty() {
                        self.title = name.into();
                    }
                    self.status = format!("{width}×{height}").into();
                    self.error = None;
                }
                SessionEvent::FrameReady { generation } => frame = Some(generation),
                SessionEvent::Clipboard(text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    self.status = "Clipboard received".into();
                }
                SessionEvent::Bell => self.status = "Bell".into(),
                SessionEvent::Error(e) => {
                    self.error = Some(e.clone().into());
                    self.status = e.into();
                }
                SessionEvent::Disconnected => {
                    self.status = "Disconnected".into();
                    self.save_thumb();
                }
            }
        }
        if let Some(generation) = frame
            && generation != self.last_generation
        {
            self.last_generation = generation;
            self.rebuild_image(cx);
        }
        cx.notify();
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
            (fb.width, fb.height, fb.pixels.clone())
        };
        let (width, height, mut pixels) = snapshot;
        // RenderImage uploads as BGRA; the compositor stores RGBA.
        for px in pixels.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
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

    fn map_pointer(&self, event: &dyn MouseEventLike, window: &Window) -> Option<(u16, u16)> {
        let pos = event.position();
        let win = window.bounds().size;
        let chrome = f32::from(theme::titlebar_height())
            + if self.pin_toolbar && self.toolbar_open {
                f32::from(theme::toolbar_height()) * 2.0
            } else {
                0.0
            };
        let view_w = f32::from(win.width);
        let view_h = (f32::from(win.height) - chrome).max(1.0);
        let local_x = f32::from(pos.x);
        let local_y = f32::from(pos.y) - chrome;
        map_to_fb(
            local_x, local_y, view_w, view_h, self.fb_w, self.fb_h, self.scale,
        )
    }

    fn send_pointer(&mut self, x: u16, y: u16) {
        if self.view_only {
            return;
        }
        self.handle.pointer(x, y, self.buttons);
    }

    fn send_keys(&mut self, events: impl IntoIterator<Item = (u32, bool)>) {
        if self.view_only {
            return;
        }
        for (keysym, down) in events {
            self.handle.key(keysym, down);
        }
    }

    fn extra_key(&mut self, keysym: u32) {
        self.send_keys([(keysym, true), (keysym, false)]);
    }

    fn send_cad(&mut self) {
        if self.view_only {
            return;
        }
        for k in CAD_KEYSYMS {
            self.handle.key(k, true);
        }
        for k in CAD_KEYSYMS.iter().rev() {
            self.handle.key(*k, false);
        }
    }

    fn send_clipboard(&mut self, cx: &App) {
        if self.view_only {
            return;
        }
        if let Some(item) = cx.read_from_clipboard()
            && let Some(text) = item.text()
        {
            if text.chars().any(|c| c as u32 > 255) {
                self.status = "Clipboard paste skipped: RFB is Latin-1 only".into();
                return;
            }
            self.handle.copy_text(text);
            self.status = "Clipboard sent".into();
        }
    }

    fn toggle_fullscreen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.fullscreen = !self.fullscreen;
        window.toggle_fullscreen();
        cx.notify();
    }

    fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.save_thumb();
        let released = self.keys.release_all();
        self.send_keys(released);
        self.handle.close();
        cx.notify();
    }

    fn consume_key(&self, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        window.prevent_default();
    }

    fn handle_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.as_str();
        tracing::debug!(
            key,
            key_char = ?event.keystroke.key_char,
            is_held = event.is_held,
            prefer_character_input = event.prefer_character_input,
            "key down"
        );
        if key.eq_ignore_ascii_case(&self.menu_key) {
            self.show_menu = !self.show_menu;
            cx.notify();
            self.consume_key(window, cx);
            return true;
        }
        let mods = event.keystroke.modifiers;
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
            true
        } else {
            false
        }
    }

    fn handle_key_up(&mut self, event: &KeyUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        tracing::debug!(key, key_char = ?event.keystroke.key_char, "key up");
        let mods = event.keystroke.modifiers;
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

    fn handle_modifiers(&mut self, event: &ModifiersChangedEvent) {
        let events = self.keys.set_modifiers(
            event.modifiers.control,
            event.modifiers.alt,
            event.modifiers.platform,
        );
        self.send_keys(events);
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .h(theme::toolbar_height())
            .w_full()
            .px_2()
            .gap_1()
            .items_center()
            .bg(theme::toolbar())
            .text_color(theme::toolbar_fg())
            .child(
                Button::new("pin")
                    .ghost()
                    .icon(if self.pin_toolbar {
                        IconName::Star
                    } else {
                        IconName::Ellipsis
                    })
                    .tooltip("Pin / unpin toolbar")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.pin_toolbar = !this.pin_toolbar;
                        this.toolbar_open = true;
                        cx.notify();
                    })),
            )
            .child(toolbar_sep())
            .child(tool_btn(
                "full",
                IconName::Maximize,
                "Full screen",
                cx.listener(|this, _, window, cx| this.toggle_fullscreen(window, cx)),
            ))
            .child(tool_btn(
                "scale",
                IconName::Maximize,
                self.scale.label(),
                cx.listener(|this, _, _, cx| {
                    this.scale = this.scale.cycle();
                    cx.notify();
                }),
            ))
            .child(toolbar_sep())
            .child(tool_btn(
                "cad",
                IconName::SquareTerminal,
                "Ctrl+Alt+Del",
                cx.listener(|this, _, _, cx| {
                    this.send_cad();
                    cx.notify();
                }),
            ))
            .child(
                Button::new("extra")
                    .ghost()
                    .icon(IconName::Asterisk)
                    .tooltip("Extra keys")
                    .dropdown_menu(move |menu, _, _| menu.menu("F8 menu", Box::new(SessionMenu))),
            )
            .child(tool_btn(
                "clip",
                IconName::Copy,
                "Send clipboard",
                cx.listener(|this, _, _, cx| {
                    this.send_clipboard(cx);
                    cx.notify();
                }),
            ))
            .child(div().flex_1())
            .child(
                Button::new("info")
                    .ghost()
                    .icon(IconName::Info)
                    .selected(self.info_open)
                    .tooltip("Connection info")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.info_open = !this.info_open;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("disc")
                    .ghost()
                    .icon(IconName::WindowClose)
                    .text_color(theme::danger())
                    .tooltip("Disconnect")
                    .on_click(cx.listener(|this, _, _, cx| this.disconnect(cx))),
            )
    }

    fn extra_keys_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_1()
            .px_2()
            .py_1()
            .bg(theme::toolbar())
            .child(chip(
                "Ctrl",
                cx.listener(|this, _, _, _| this.extra_key(XK_CONTROL_L)),
            ))
            .child(chip(
                "Alt",
                cx.listener(|this, _, _, _| this.extra_key(XK_ALT_L)),
            ))
            .child(chip(
                "Win",
                cx.listener(|this, _, _, _| this.extra_key(rv_core::XK_SUPER_L)),
            ))
            .child(chip(
                "Tab",
                cx.listener(|this, _, _, _| this.extra_key(rv_core::XK_TAB)),
            ))
            .child(chip(
                "Esc",
                cx.listener(|this, _, _, _| this.extra_key(rv_core::XK_ESCAPE)),
            ))
            .child(chip(
                "CAD",
                cx.listener(|this, _, _, cx| {
                    this.send_cad();
                    cx.notify();
                }),
            ))
    }
}

impl Focusable for SessionView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SessionView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let image = self.render_image.clone();
        let object_fit = match self.scale {
            ScaleMode::Fit => ObjectFit::Contain,
            ScaleMode::Stretch => ObjectFit::Fill,
            ScaleMode::Actual => ObjectFit::None,
        };
        let info = format!(
            "{}  {}×{}  {}",
            self.host,
            self.fb_w,
            self.fb_h,
            self.scale.label()
        );

        v_flex()
            .size_full()
            .bg(theme::desktop())
            .text_color(theme::toolbar_fg())
            .key_context("Session")
            .track_focus(&self.focus)
            .child({
                let focus = self.focus.clone();
                canvas(
                    |_, _, _| (),
                    move |_, _, window, cx| {
                        disable_platform_ime(window);
                        window.handle_input(&focus, DisabledIme, cx);
                    },
                )
                .w(px(0.))
                .h(px(0.))
            })
            .on_key_down(cx.listener(|this, ev, window, cx| {
                this.handle_key(ev, window, cx);
            }))
            .on_key_up(cx.listener(|this, ev, window, cx| this.handle_key_up(ev, window, cx)))
            .on_modifiers_changed(cx.listener(|this, ev, _, _| this.handle_modifiers(ev)))
            .on_action(cx.listener(|this, _: &SessionFullscreen, window, cx| {
                this.toggle_fullscreen(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SessionDisconnect, _, cx| {
                this.disconnect(cx);
            }))
            .on_action(cx.listener(|this, _: &SessionCad, _, cx| {
                this.send_cad();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SessionScaleCycle, _, cx| {
                this.scale = this.scale.cycle();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SessionMenu, _, cx| {
                this.show_menu = !this.show_menu;
                cx.notify();
            }))
            .child(
                TitleBar::new().child(
                    h_flex()
                        .w_full()
                        .px_2()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .font_semibold()
                                .text_color(theme::ink())
                                .child(self.title.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::muted())
                                .child(self.status.clone()),
                        ),
                ),
            )
            .when(self.pin_toolbar && self.toolbar_open, |this| {
                this.child(self.toolbar(cx)).child(self.extra_keys_bar(cx))
            })
            .child(
                div()
                    .id("vnc-canvas")
                    .flex_1()
                    .relative()
                    .overflow_hidden()
                    .bg(theme::desktop())
                    .cursor(CursorStyle::Arrow)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                            this.focus.focus(window, cx);
                            this.buttons |= 1;
                            if let Some((x, y)) = this.map_pointer(ev, window) {
                                this.send_pointer(x, y);
                            }
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseUpEvent, window, _| {
                            this.buttons &= !1;
                            if let Some((x, y)) = this.map_pointer(ev, window) {
                                this.send_pointer(x, y);
                            }
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                            this.focus.focus(window, cx);
                            this.buttons |= 4;
                            if let Some((x, y)) = this.map_pointer(ev, window) {
                                this.send_pointer(x, y);
                            }
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Right,
                        cx.listener(|this, ev: &MouseUpEvent, window, _| {
                            this.buttons &= !4;
                            if let Some((x, y)) = this.map_pointer(ev, window) {
                                this.send_pointer(x, y);
                            }
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, window, _| {
                        if let Some((x, y)) = this.map_pointer(ev, window) {
                            this.send_pointer(x, y);
                        }
                    }))
                    .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, window, _| {
                        let delta = ev.delta.pixel_delta(px(16.));
                        let bit = if f32::from(delta.y) < 0.0 { 16u8 } else { 8u8 };
                        this.buttons |= bit;
                        if let Some((x, y)) = this.map_pointer(ev, window) {
                            this.send_pointer(x, y);
                            this.buttons &= !bit;
                            this.send_pointer(x, y);
                        } else {
                            this.buttons &= !bit;
                        }
                    }))
                    .child(
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(if let Some(img_src) = image {
                                img(img_src)
                                    .id("fb")
                                    .object_fit(object_fit)
                                    .when(self.scale == ScaleMode::Actual, |this| this)
                                    .size_full()
                                    .into_any_element()
                            } else {
                                v_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        Icon::new(IconName::Frame)
                                            .size_10()
                                            .text_color(theme::muted()),
                                    )
                                    .child(div().text_sm().child(self.status.clone()))
                                    .into_any_element()
                            }),
                    )
                    .when(self.error.is_some(), |this| {
                        this.child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(hsla(0., 0., 0., 0.55))
                                .child(
                                    v_flex()
                                        .gap_2()
                                        .p_4()
                                        .rounded_lg()
                                        .bg(theme::card())
                                        .text_color(theme::ink())
                                        .child(
                                            div()
                                                .font_semibold()
                                                .text_color(theme::danger())
                                                .child("Connection failed"),
                                        )
                                        .child(div().child(self.error.clone().unwrap_or_default())),
                                ),
                        )
                    })
                    .when(self.info_open, |this| {
                        this.child(
                            div()
                                .absolute()
                                .top_2()
                                .right_2()
                                .p_3()
                                .rounded_md()
                                .bg(theme::toolbar())
                                .child(div().text_xs().child(info.clone())),
                        )
                    })
                    .when(!self.pin_toolbar, |this| {
                        this.child(div().absolute().top_2().left_1_2().child(
                            Button::new("float-tb").primary().label("Toolbar").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.pin_toolbar = true;
                                    this.toolbar_open = true;
                                    cx.notify();
                                }),
                            ),
                        ))
                    }),
            )
            .when(self.show_menu, |this| {
                this.child(
                    h_flex()
                        .px_3()
                        .py_2()
                        .gap_2()
                        .bg(theme::toolbar())
                        .child(div().text_xs().child("F8 menu"))
                        .child(
                            Button::new("m-full")
                                .xsmall()
                                .label("Full screen")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.toggle_fullscreen(window, cx)
                                })),
                        )
                        .child(
                            Button::new("m-scale")
                                .xsmall()
                                .label(self.scale.label())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.scale = this.scale.cycle();
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("m-cad")
                                .xsmall()
                                .label("Ctrl+Alt+Del")
                                .on_click(cx.listener(|this, _, _, _cx| this.send_cad())),
                        )
                        .child(
                            Button::new("m-disc")
                                .xsmall()
                                .danger()
                                .label("Disconnect")
                                .on_click(cx.listener(|this, _, _, cx| this.disconnect(cx))),
                        ),
                )
            })
    }
}

trait MouseEventLike {
    fn position(&self) -> Point<Pixels>;
}

impl MouseEventLike for MouseDownEvent {
    fn position(&self) -> Point<Pixels> {
        self.position
    }
}
impl MouseEventLike for MouseUpEvent {
    fn position(&self) -> Point<Pixels> {
        self.position
    }
}
impl MouseEventLike for MouseMoveEvent {
    fn position(&self) -> Point<Pixels> {
        self.position
    }
}
impl MouseEventLike for ScrollWheelEvent {
    fn position(&self) -> Point<Pixels> {
        self.position
    }
}

fn map_to_fb(
    local_x: f32,
    local_y: f32,
    view_w: f32,
    view_h: f32,
    fb_w: u16,
    fb_h: u16,
    mode: ScaleMode,
) -> Option<(u16, u16)> {
    if fb_w == 0 || fb_h == 0 || view_w <= 1.0 || view_h <= 1.0 {
        return None;
    }
    let (dx, dy, dw, dh) = dest_rect(view_w, view_h, fb_w, fb_h, mode);
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
    view_w: f32,
    view_h: f32,
    fb_w: u16,
    fb_h: u16,
    mode: ScaleMode,
) -> (f32, f32, f32, f32) {
    match mode {
        ScaleMode::Stretch => (0.0, 0.0, view_w, view_h),
        ScaleMode::Actual => {
            let dw = fb_w as f32;
            let dh = fb_h as f32;
            ((view_w - dw) * 0.5, (view_h - dh) * 0.5, dw, dh)
        }
        ScaleMode::Fit => {
            let scale = (view_w / fb_w as f32).min(view_h / fb_h as f32);
            let dw = fb_w as f32 * scale;
            let dh = fb_h as f32 * scale;
            ((view_w - dw) * 0.5, (view_h - dh) * 0.5, dw, dh)
        }
    }
}

fn toolbar_sep() -> impl IntoElement {
    div().w(px(1.)).h(px(18.)).bg(hsla(0., 0., 1., 0.12))
}

fn tool_btn(
    id: &'static str,
    icon: IconName,
    tip: impl Into<SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    Button::new(id)
        .ghost()
        .icon(icon)
        .tooltip(tip)
        .on_click(on_click)
}

fn chip(
    label: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    Button::new(label)
        .xsmall()
        .ghost()
        .label(label)
        .on_click(on_click)
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

fn disable_platform_ime(window: &Window) {
    macos_ime::disable(window);
}

#[cfg(target_os = "macos")]
mod macos_ime {
    use gpui::Window;
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSArray, NSString};
    use raw_window_handle::RawWindowHandle;

    pub fn disable(window: &Window) {
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
            // Empty locale list: no input sources, so CJK/dead-key IMEs cannot attach.
            let empty = NSArray::<NSString>::new();
            let _: () = msg_send![ctx, setAllowedInputSourceLocales: &*empty];
            let _: () = msg_send![ctx, discardMarkedText];
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod macos_ime {
    pub fn disable(_window: &gpui::Window) {}
}
