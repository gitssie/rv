use std::time::{SystemTime, UNIX_EPOCH};

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    Disableable as _, Icon, IconName, Root, Selectable, Sizable, StyledExt, TitleBar,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    menu::{ContextMenuExt, PopupMenuItem},
    sidebar::{
        Sidebar, SidebarCollapsible, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem,
        SidebarToggleButton,
    },
    v_flex,
};

use rv_core::{
    AddressBook, ConnectRequest, Connection, ConnectionId, EncryptionMode, QualityPreset,
    load_password, parse_server, save_password,
};

use crate::actions::*;
use crate::session_window;
use crate::theme;

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

#[derive(Clone)]
enum Modal {
    None,
    Connection,
    DeleteConfirm { id: ConnectionId, name: String },
    Preferences,
}

pub struct AddressBookApp {
    book: AddressBook,
    search: Entity<InputState>,
    name_input: Entity<InputState>,
    server_input: Entity<InputState>,
    password_input: Entity<InputState>,
    collapsed: bool,
    view_mode: ViewMode,
    filter: SidebarFilter,
    selected: Option<ConnectionId>,
    remember_password: bool,
    encryption: EncryptionMode,
    quality: QualityPreset,
    view_only: bool,
    shared: bool,
    editing: Option<ConnectionId>,
    modal: Modal,
    status: SharedString,
    focus: FocusHandle,
}

impl AddressBookApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let book = AddressBook::load(rv_core::StorePaths::default_dir().unwrap_or_else(|_| {
            rv_core::StorePaths::in_dir(
                std::env::temp_dir().join(format!("rv-{}", std::process::id())),
            )
        }))
        .unwrap_or_else(|e| {
            tracing::warn!("address book load failed: {e}");
            AddressBook::load(rv_core::StorePaths::in_dir(
                std::env::temp_dir().join(format!("rv-{}", std::process::id())),
            ))
            .expect("temp address book")
        });

        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search connections…")
                .clean_on_escape()
        });
        let name_input = cx.new(|cx| InputState::new(window, cx).placeholder("Office Mac"));
        let server_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("host.example.com:5900"));
        let password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("VNC password")
                .masked(true)
        });

        Self {
            book,
            search,
            name_input,
            server_input,
            password_input,
            collapsed: false,
            view_mode: ViewMode::Grid,
            filter: SidebarFilter::All,
            selected: None,
            remember_password: true,
            encryption: EncryptionMode::LetServerChoose,
            quality: QualityPreset::Auto,
            view_only: false,
            shared: true,
            editing: None,
            modal: Modal::None,
            status: "Ready".into(),
            focus: cx.focus_handle(),
        }
    }

    fn persist(&mut self) {
        if let Err(e) = self.book.save() {
            self.status = format!("Save failed: {e}").into();
        }
    }

    fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().to_string()
    }

    fn visible(&self, cx: &App) -> Vec<Connection> {
        let q = self.query(cx);
        match &self.filter {
            SidebarFilter::All => self.book.filtered(&q, None),
            SidebarFilter::Recents => self
                .book
                .recents(20)
                .into_iter()
                .filter(|c| {
                    q.is_empty()
                        || c.name
                            .to_ascii_lowercase()
                            .contains(&q.to_ascii_lowercase())
                        || c.host
                            .to_ascii_lowercase()
                            .contains(&q.to_ascii_lowercase())
                })
                .collect(),
            SidebarFilter::Label(label) => self.book.filtered(&q, Some(label)),
        }
    }

    fn on_new(&mut self, _: &NewConnection, window: &mut Window, cx: &mut Context<Self>) {
        tracing::info!("opening new connection dialog");
        self.status = "New connection".into();
        self.editing = None;
        self.remember_password = true;
        self.encryption = EncryptionMode::LetServerChoose;
        self.quality = QualityPreset::Auto;
        self.view_only = false;
        self.shared = true;
        self.name_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.server_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.password_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.modal = Modal::Connection;
        self.name_input.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    fn on_properties(&mut self, _: &OpenProperties, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected else {
            self.status = "Select a connection first".into();
            cx.notify();
            return;
        };
        let Some(conn) = self.book.get(id).cloned() else {
            return;
        };
        self.editing = Some(id);
        self.encryption = conn.encryption;
        self.quality = conn.quality;
        self.view_only = conn.view_only;
        self.shared = conn.shared;
        self.remember_password = conn.remember_password;
        self.name_input
            .update(cx, |s, cx| s.set_value(conn.name.clone(), window, cx));
        self.server_input
            .update(cx, |s, cx| s.set_value(conn.server_display(), window, cx));
        let pw = load_password(id).ok().flatten().unwrap_or_default();
        self.password_input
            .update(cx, |s, cx| s.set_value(pw, window, cx));
        self.modal = Modal::Connection;
        self.name_input.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    fn on_duplicate(&mut self, _: &DuplicateSelected, _: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected else {
            return;
        };
        if let Some(conn) = self.book.get(id).cloned() {
            let mut copy = conn;
            copy.id = ConnectionId::new();
            copy.name = format!("{} copy", copy.name);
            copy.last_connected = None;
            self.book.upsert(copy);
            self.persist();
            cx.notify();
        }
    }

    fn submit_connect(&mut self, connect: bool, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name_input.read(cx).value().to_string();
        let server = self.server_input.read(cx).value().to_string();
        let password = self.password_input.read(cx).unmask_value().to_string();
        let (host, port) = match parse_server(&server) {
            Ok(v) => v,
            Err(e) => {
                self.status = e.into();
                cx.notify();
                return;
            }
        };
        let mut conn = if let Some(id) = self.editing {
            self.book
                .get(id)
                .cloned()
                .unwrap_or_else(|| Connection::new(&name, &host, port))
        } else {
            Connection::new(&name, &host, port)
        };
        conn.name = if name.trim().is_empty() {
            format!("{host}:{port}")
        } else {
            name
        };
        conn.host = host;
        conn.port = port;
        conn.encryption = self.encryption;
        conn.quality = self.quality;
        conn.view_only = self.view_only;
        conn.shared = self.shared;
        conn.remember_password = self.remember_password;
        if self.remember_password && !password.is_empty() {
            let _ = save_password(conn.id, &password);
        }
        let req = ConnectRequest::from_connection(
            &conn,
            if password.is_empty() {
                load_password(conn.id).ok().flatten()
            } else {
                Some(password)
            },
        );
        self.book.upsert(conn.clone());
        self.selected = Some(conn.id);
        self.persist();
        self.modal = Modal::None;
        if connect {
            self.launch(req, window, cx);
        } else {
            self.status = format!("Saved {}", conn.name).into();
            cx.notify();
        }
    }

    fn on_connect(&mut self, _: &ConnectSelected, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected else {
            self.status = "Select a connection to connect".into();
            cx.notify();
            return;
        };
        self.connect_id(id, window, cx);
    }

    fn connect_id(&mut self, id: ConnectionId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut conn) = self.book.get(id).cloned() else {
            return;
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        conn.last_connected = Some(now);
        let password = load_password(id).ok().flatten();
        let req = ConnectRequest::from_connection(&conn, password);
        self.book.upsert(conn);
        self.persist();
        self.launch(req, window, cx);
    }

    fn launch(&mut self, req: ConnectRequest, window: &mut Window, cx: &mut Context<Self>) {
        self.status = format!("Connecting to {}…", req.display_name()).into();
        let title = req.display_name().to_string();
        let scale = self.book.prefs().default_scale;
        let pin = self.book.prefs().pin_toolbar;
        let menu_key = self.book.prefs().menu_key.clone();
        let hide_shots = self.book.prefs().hide_screenshots;
        let thumb_path = req.connection_id.map(|id| self.book.paths().thumb_path(id));
        session_window::open(
            req,
            title,
            session_window::SessionOptions {
                scale,
                pin_toolbar: pin,
                menu_key,
                hide_shots,
                thumb_path,
            },
            window,
            cx,
        );
        cx.notify();
    }

    pub fn connect_target(&mut self, target: &str, window: &mut Window, cx: &mut Context<Self>) {
        match parse_server(target) {
            Ok((host, port)) => {
                let conn = Connection::new(format!("CLI {host}"), &host, port);
                let req = ConnectRequest::from_connection(&conn, None);
                self.book.upsert(conn);
                self.persist();
                self.launch(req, window, cx);
            }
            Err(e) => {
                self.status = e.into();
                cx.notify();
            }
        }
    }

    fn ask_delete(&mut self, id: ConnectionId, cx: &mut Context<Self>) {
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
            }
            Err(e) => {
                self.status = format!("Delete failed: {e}").into();
            }
        }
        self.modal = Modal::None;
        cx.notify();
    }

    fn close_modal(&mut self, cx: &mut Context<Self>) {
        self.modal = Modal::None;
        cx.notify();
    }

    fn on_delete(&mut self, _: &DeleteSelected, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.modal, Modal::DeleteConfirm { .. }) {
            if let Modal::DeleteConfirm { id, .. } = self.modal.clone() {
                self.confirm_delete(id, cx);
            }
            return;
        }
        if !matches!(self.modal, Modal::None) {
            return;
        }
        if self.search.focus_handle(cx).is_focused(window) {
            return;
        }
        let Some(id) = self.selected else {
            self.status = "Select a connection to delete".into();
            cx.notify();
            return;
        };
        self.ask_delete(id, cx);
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

    fn on_focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |s, cx| s.focus(window, cx));
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let labels = self.book.labels();
        let filter = self.filter.clone();
        Sidebar::new("rv-sidebar")
            .collapsible(SidebarCollapsible::Icon)
            .collapsed(self.collapsed)
            .w(theme::sidebar_width())
            .bg(theme::sidebar())
            .header(
                SidebarHeader::new().child(
                    h_flex().gap_2().items_center().child(logo_mark()).when(
                        !self.collapsed,
                        |this| {
                            this.child(
                                v_flex()
                                    .child(
                                        div().text_color(theme::ink()).font_semibold().child("RV"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme::muted())
                                            .child("Address Book"),
                                    ),
                            )
                        },
                    ),
                ),
            )
            .child(
                SidebarGroup::new("Browse").child(
                    SidebarMenu::new().children([
                        SidebarMenuItem::new("All connections")
                            .icon(IconName::LayoutDashboard)
                            .active(filter == SidebarFilter::All)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.filter = SidebarFilter::All;
                                cx.notify();
                            })),
                        SidebarMenuItem::new("Recents")
                            .icon(IconName::BookOpen)
                            .active(filter == SidebarFilter::Recents)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.filter = SidebarFilter::Recents;
                                cx.notify();
                            })),
                    ]),
                ),
            )
            .when(!labels.is_empty(), |side| {
                side.child(
                    SidebarGroup::new("Labels").child(SidebarMenu::new().children(
                        labels.into_iter().map(|label| {
                            let selected =
                                matches!(&filter, SidebarFilter::Label(l) if l == &label);
                            let label_for_click = label.clone();
                            SidebarMenuItem::new(label)
                                .icon(IconName::Star)
                                .active(selected)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.filter = SidebarFilter::Label(label_for_click.clone());
                                    cx.notify();
                                }))
                        }),
                    )),
                )
            })
    }

    fn render_card(&self, conn: &Connection, cx: &mut Context<Self>) -> impl IntoElement {
        let id = conn.id;
        let selected = self.selected == Some(id);
        let hide = self.book.prefs().hide_screenshots;
        let thumb = self.book.paths().thumb_path(id);
        let name = conn.name.clone();
        let host = conn.server_display();

        v_flex()
            .id(ElementId::from(format!("card-{id}")))
            .w(px(220.))
            .rounded_lg()
            .border_1()
            .border_color(if selected {
                theme::accent()
            } else {
                theme::line()
            })
            .bg(theme::card())
            .shadow_sm()
            .overflow_hidden()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, _, cx| {
                    this.selected = Some(id);
                    cx.notify();
                }),
            )
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                this.selected = Some(id);
                if ev.click_count() >= 2 {
                    this.connect_id(id, window, cx);
                }
                cx.notify();
            }))
            .context_menu(connection_menu(cx.entity(), id))
            .child(
                div()
                    .relative()
                    .h(px(132.))
                    .w_full()
                    .bg(theme::thumb_bg())
                    .items_center()
                    .justify_center()
                    .flex()
                    .child(if !hide && thumb.exists() {
                        img(thumb)
                            .size_full()
                            .object_fit(ObjectFit::Cover)
                            .into_any_element()
                    } else {
                        Icon::new(IconName::Frame)
                            .size_8()
                            .text_color(theme::muted())
                            .into_any_element()
                    })
                    .child(
                        div().absolute().top_1().right_1().child(
                            Button::new(SharedString::from(format!("card-del-{id}")))
                                .ghost()
                                .icon(IconName::Delete)
                                .xsmall()
                                .tooltip("Delete connection")
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.ask_delete(id, cx);
                                })),
                        ),
                    ),
            )
            .child(
                v_flex()
                    .p_2()
                    .gap_0p5()
                    .child(
                        div()
                            .font_semibold()
                            .text_color(theme::ink())
                            .text_ellipsis()
                            .child(name),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::muted())
                            .text_ellipsis()
                            .child(host),
                    ),
            )
    }

    fn render_row(&self, conn: &Connection, cx: &mut Context<Self>) -> impl IntoElement {
        let id = conn.id;
        let selected = self.selected == Some(id);
        h_flex()
            .id(ElementId::from(format!("row-{id}")))
            .w_full()
            .px_3()
            .py_2()
            .gap_3()
            .rounded_md()
            .bg(if selected {
                theme::selected()
            } else {
                theme::card()
            })
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, _, cx| {
                    this.selected = Some(id);
                    cx.notify();
                }),
            )
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                this.selected = Some(id);
                if ev.click_count() >= 2 {
                    this.connect_id(id, window, cx);
                }
                cx.notify();
            }))
            .context_menu(connection_menu(cx.entity(), id))
            .child(Icon::new(IconName::Frame).text_color(theme::accent()))
            .child(
                v_flex()
                    .flex_1()
                    .child(div().font_semibold().child(conn.name.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::muted())
                            .child(conn.server_display()),
                    ),
            )
            .when(!conn.labels.is_empty(), |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(theme::accent())
                        .child(conn.labels.join(", ")),
                )
            })
            .child(
                Button::new(SharedString::from(format!("row-del-{id}")))
                    .ghost()
                    .icon(IconName::Delete)
                    .xsmall()
                    .tooltip("Delete connection")
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ask_delete(id, cx);
                    })),
            )
    }

    fn render_new_card(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("card-new")
            .w(px(220.))
            .rounded_lg()
            .border_1()
            .border_color(theme::line())
            .bg(theme::card())
            .overflow_hidden()
            .cursor_pointer()
            .on_click(cx.listener(|this, _, window, cx| {
                this.on_new(&NewConnection, window, cx);
            }))
            .child(
                div()
                    .h(px(132.))
                    .w_full()
                    .bg(theme::thumb_bg())
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        Icon::new(IconName::Plus)
                            .size_8()
                            .text_color(theme::accent()),
                    ),
            )
            .child(
                v_flex().p_2().gap_0p5().child(
                    div()
                        .font_semibold()
                        .text_color(theme::ink())
                        .child("New connection"),
                ),
            )
    }

    fn render_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let title = match &self.modal {
            Modal::Connection => {
                if self.editing.is_some() {
                    "Connection properties"
                } else {
                    "New connection"
                }
            }
            Modal::DeleteConfirm { .. } => "Delete connection",
            Modal::Preferences => "Preferences",
            Modal::None => return div().into_any_element(),
        };

        let body: AnyElement = match &self.modal {
            Modal::Connection => v_flex()
                .gap_3()
                .child(field("Name", Input::new(&self.name_input)))
                .child(field("VNC server", Input::new(&self.server_input)))
                .child(field("Password", Input::new(&self.password_input)))
                .child(options_row(self, cx))
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("modal-cancel")
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| this.close_modal(cx))),
                        )
                        .child(
                            Button::new("modal-save")
                                .label("Save")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.submit_connect(false, window, cx)
                                })),
                        )
                        .child(
                            Button::new("modal-connect")
                                .primary()
                                .label("Connect")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.submit_connect(true, window, cx)
                                })),
                        ),
                )
                .into_any_element(),
            Modal::DeleteConfirm { id, name } => {
                let id = *id;
                let name = name.clone();
                v_flex()
                    .gap_3()
                    .child(
                        div()
                            .text_color(theme::ink())
                            .child(format!("Remove “{name}” from the address book?")),
                    )
                    .child(div().text_xs().text_color(theme::muted()).child(
                        "The saved password and screenshot for this connection are also removed.",
                    ))
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("del-cancel")
                                    .label("Cancel")
                                    .on_click(cx.listener(|this, _, _, cx| this.close_modal(cx))),
                            )
                            .child(Button::new("del-ok").danger().label("Delete").on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.confirm_delete(id, cx);
                                }),
                            )),
                    )
                    .into_any_element()
            }
            Modal::Preferences => v_flex()
                .gap_3()
                .child(prefs_body(self, cx))
                .child(
                    h_flex().justify_end().child(
                        Button::new("prefs-done")
                            .primary()
                            .label("Done")
                            .on_click(cx.listener(|this, _, _, cx| this.close_modal(cx))),
                    ),
                )
                .into_any_element(),
            Modal::None => div().into_any_element(),
        };

        div()
            .id("modal-overlay")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(hsla(0., 0., 0., 0.45))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, _, cx| this.close_modal(cx)))
            .child(
                v_flex()
                    .id("modal-card")
                    .w(px(460.))
                    .rounded_lg()
                    .bg(theme::card())
                    .border_1()
                    .border_color(theme::line())
                    .shadow_lg()
                    .p_5()
                    .gap_4()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .text_lg()
                            .font_semibold()
                            .text_color(theme::ink())
                            .child(title),
                    )
                    .child(body),
            )
            .into_any_element()
    }
}

impl Focusable for AddressBookApp {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for AddressBookApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let items = self.visible(cx);
        let mut cards: Vec<AnyElement> = items
            .iter()
            .map(|c| self.render_card(c, cx).into_any_element())
            .collect();
        cards.push(self.render_new_card(cx).into_any_element());
        let mut rows: Vec<AnyElement> = items
            .iter()
            .map(|c| self.render_row(c, cx).into_any_element())
            .collect();
        rows.insert(
            0,
            h_flex()
                .id("row-new")
                .w_full()
                .px_3()
                .py_2()
                .gap_3()
                .rounded_md()
                .bg(theme::card())
                .cursor_pointer()
                .on_click(cx.listener(|this, _, window, cx| {
                    this.on_new(&NewConnection, window, cx);
                }))
                .child(Icon::new(IconName::Plus).text_color(theme::accent()))
                .child(div().font_semibold().child("New connection"))
                .into_any_element(),
        );
        v_flex()
            .id("address-book")
            .relative()
            .size_full()
            .bg(theme::surface())
            .text_color(theme::ink())
            .key_context("AddressBook")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_new))
            .on_action(cx.listener(Self::on_connect))
            .on_action(cx.listener(Self::on_delete))
            .on_action(cx.listener(Self::on_prefs))
            .on_action(cx.listener(Self::on_toggle_view))
            .on_action(cx.listener(Self::on_focus_search))
            .on_action(cx.listener(Self::on_properties))
            .on_action(cx.listener(Self::on_duplicate))
            .on_action(|_: &QuitApp, _, cx| cx.quit())
            .child(
                TitleBar::new().child(
                    h_flex()
                        .w_full()
                        .pr_3()
                        .items_center()
                        .gap_2()
                        .child(
                            SidebarToggleButton::new()
                                .collapsed(self.collapsed)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.collapsed = !this.collapsed;
                                    cx.notify();
                                })),
                        )
                        .child(
                            div()
                                .font_semibold()
                                .text_color(theme::ink())
                                .child("RV Viewer"),
                        ),
                ),
            )
            .child(
                h_flex()
                    .h(px(48.))
                    .px_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(theme::line())
                    .bg(theme::card())
                    .child(
                        div().w(px(280.)).child(
                            Input::new(&self.search)
                                .prefix(Icon::new(IconName::Search).text_color(theme::muted())),
                        ),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("view-mode")
                            .ghost()
                            .icon(if self.view_mode == ViewMode::Grid {
                                IconName::LayoutDashboard
                            } else {
                                IconName::Menu
                            })
                            .tooltip("Toggle list / grid")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.view_mode = match this.view_mode {
                                    ViewMode::Grid => ViewMode::List,
                                    ViewMode::List => ViewMode::Grid,
                                };
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("prefs")
                            .ghost()
                            .icon(IconName::Settings)
                            .tooltip("Preferences")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_prefs(&OpenPreferences, window, cx);
                            })),
                    )
                    .child(
                        Button::new("delete")
                            .ghost()
                            .icon(IconName::Delete)
                            .label("Delete")
                            .tooltip("Delete selected connection")
                            .disabled(self.selected.is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(id) = this.selected {
                                    this.ask_delete(id, cx);
                                } else {
                                    this.status = "Select a connection to delete".into();
                                    cx.notify();
                                }
                            })),
                    )
                    .child(
                        Button::new("new")
                            .primary()
                            .icon(IconName::Plus)
                            .label("New connection")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_new(&NewConnection, window, cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(cx))
                    .child(v_flex().flex_1().min_w_0().h_full().p_4().gap_3().child(
                        if items.is_empty() {
                            empty_state(cx).into_any_element()
                        } else if self.view_mode == ViewMode::Grid {
                            div()
                                .id("grid")
                                .flex()
                                .flex_row()
                                .flex_wrap()
                                .gap_4()
                                .overflow_y_scroll()
                                .children(cards)
                                .into_any_element()
                        } else {
                            v_flex()
                                .id("list")
                                .gap_1()
                                .overflow_y_scroll()
                                .children(rows)
                                .into_any_element()
                        },
                    )),
            )
            .child(
                h_flex()
                    .h(px(28.))
                    .px_3()
                    .items_center()
                    .border_t_1()
                    .border_color(theme::line())
                    .bg(theme::card())
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme::muted())
                            .child(self.status.clone()),
                    ),
            )
            .when(!matches!(self.modal, Modal::None), |this| {
                this.child(self.render_modal(cx))
            })
    }
}

fn logo_mark() -> impl IntoElement {
    div()
        .size_8()
        .rounded_md()
        .bg(theme::accent())
        .text_color(rgb(0xFFFFFF))
        .flex()
        .items_center()
        .justify_center()
        .font_bold()
        .child("RV")
}

fn field(label: &'static str, input: impl IntoElement) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(div().text_xs().text_color(theme::muted()).child(label))
        .child(input)
}

fn connection_menu(
    book: Entity<AddressBookApp>,
    id: ConnectionId,
) -> impl Fn(
    gpui_component::menu::PopupMenu,
    &mut Window,
    &mut Context<gpui_component::menu::PopupMenu>,
) -> gpui_component::menu::PopupMenu {
    move |menu, _, _| {
        let connect = book.clone();
        let props = book.clone();
        let dup = book.clone();
        let del = book.clone();
        menu.item(
            PopupMenuItem::new("Connect").on_click(move |_, window, cx| {
                connect.update(cx, |this, cx| {
                    this.selected = Some(id);
                    this.connect_id(id, window, cx);
                });
            }),
        )
        .item(
            PopupMenuItem::new("Properties").on_click(move |_, window, cx| {
                props.update(cx, |this, cx| {
                    this.selected = Some(id);
                    this.on_properties(&OpenProperties, window, cx);
                });
            }),
        )
        .item(
            PopupMenuItem::new("Duplicate").on_click(move |_, window, cx| {
                dup.update(cx, |this, cx| {
                    this.selected = Some(id);
                    this.on_duplicate(&DuplicateSelected, window, cx);
                });
            }),
        )
        .separator()
        .item(PopupMenuItem::new("Delete").on_click(move |_, _, cx| {
            del.update(cx, |this, cx| {
                this.ask_delete(id, cx);
            });
        }))
    }
}

fn options_row(app: &AddressBookApp, cx: &mut Context<AddressBookApp>) -> impl IntoElement {
    let enc = app.encryption;
    let quality = app.quality;
    let remember = app.remember_password;
    let view_only = app.view_only;
    let shared = app.shared;
    v_flex()
        .gap_2()
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("enc")
                        .outline()
                        .label(format!("Encryption: {}", enc.label()))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.encryption = this.encryption.cycle();
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("qual")
                        .outline()
                        .label(format!("Quality: {}", quality.label()))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.quality = this.quality.cycle();
                            cx.notify();
                        })),
                ),
        )
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("remember")
                        .ghost()
                        .selected(remember)
                        .label("Remember password")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.remember_password = !this.remember_password;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("viewonly")
                        .ghost()
                        .selected(view_only)
                        .label("View only")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.view_only = !this.view_only;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("shared")
                        .ghost()
                        .selected(shared)
                        .label("Shared")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.shared = !this.shared;
                            cx.notify();
                        })),
                ),
        )
}

fn prefs_body(app: &AddressBookApp, cx: &mut Context<AddressBookApp>) -> impl IntoElement {
    let theme_pref = app.book.prefs().theme;
    let hide = app.book.prefs().hide_screenshots;
    let scale = app.book.prefs().default_scale;
    let pin = app.book.prefs().pin_toolbar;
    v_flex()
        .gap_2()
        .child(
            Button::new("theme")
                .outline()
                .label(format!("Theme: {}", theme_pref.label()))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.book.prefs_mut().theme = this.book.prefs().theme.cycle();
                    this.persist();
                    cx.notify();
                })),
        )
        .child(
            Button::new("scale")
                .outline()
                .label(format!("Default scaling: {}", scale.label()))
                .on_click(cx.listener(|this, _, _, cx| {
                    let next = this.book.prefs().default_scale.cycle();
                    this.book.prefs_mut().default_scale = next;
                    this.persist();
                    cx.notify();
                })),
        )
        .child(
            Button::new("pin")
                .ghost()
                .selected(pin)
                .label("Pin session toolbar")
                .on_click(cx.listener(|this, _, _, cx| {
                    let v = !this.book.prefs().pin_toolbar;
                    this.book.prefs_mut().pin_toolbar = v;
                    this.persist();
                    cx.notify();
                })),
        )
        .child(
            Button::new("hide")
                .ghost()
                .selected(hide)
                .label("Hide desktop previews")
                .on_click(cx.listener(|this, _, _, cx| {
                    let v = !this.book.prefs().hide_screenshots;
                    this.book.prefs_mut().hide_screenshots = v;
                    this.persist();
                    cx.notify();
                })),
        )
        .child(
            Button::new("forget")
                .danger()
                .label("Forget passwords and screenshots")
                .on_click(cx.listener(|this, _, _, cx| {
                    let _ = this.book.forget_sensitive();
                    this.status = "Sensitive data removed".into();
                    cx.notify();
                })),
        )
}

fn empty_state(cx: &mut Context<AddressBookApp>) -> impl IntoElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .child(logo_mark())
        .child(div().text_lg().font_semibold().child("No connections yet"))
        .child(
            div()
                .text_color(theme::muted())
                .child("Create a connection to a VNC server on your network."),
        )
        .child(
            Button::new("empty-new")
                .primary()
                .label("New connection")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.on_new(&NewConnection, window, cx);
                })),
        )
}
