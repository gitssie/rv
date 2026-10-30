use super::{AddressBookApp, Modal, ViewMode, WindowRoot};
use crate::actions::*;
use gpui::{AppContext, Entity, Modifiers, TestAppContext, VisualTestContext, point, px, size};
use gpui_component::Root;
use rv_core::{
    AddressBook, ClipboardMode, ConnectRequest, Connection, LocalCursorMode, TransferFolders,
};
use tempfile::TempDir;

pub(super) fn setup(
    cx: &mut TestAppContext,
    connections: Vec<Connection>,
) -> (Entity<AddressBookApp>, &mut VisualTestContext, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut book = AddressBook::load(rv_core::StorePaths::in_dir(dir.path().into())).unwrap();
    for connection in connections {
        book.upsert(connection);
    }
    book.save().unwrap();
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.set_reduce_motion(true);
        crate::bind_keys(cx);
    });
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| AddressBookApp::with_book(book, false, "Ready".into(), window, cx));
        entity = Some(app.clone());
        let shell = cx.new(|cx| WindowRoot::new(app, cx));
        Root::new(shell, window, cx)
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    (entity.unwrap(), cx, dir)
}

pub(super) fn click(cx: &mut VisualTestContext, selector: &str) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let bounds = cx
        .debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
        .unwrap_or_else(|| panic!("button must be rendered: {selector}"));
    assert!(bounds.size.width > px(0.) && bounds.size.height > px(0.));
    cx.simulate_mouse_move(bounds.center(), None, Modifiers::default());
    cx.simulate_click(bounds.center(), Modifiers::default());
    cx.run_until_parked();
}

#[gpui::test]
fn home_actions_share_window_titlebar_without_a_second_toolbar(cx: &mut TestAppContext) {
    let (app, cx, _dir) = setup(cx, vec![]);
    for width in [1100., 800.] {
        cx.simulate_resize(size(px(width), px(720.)));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let toolbar = cx.debug_bounds("management-toolbar").unwrap();
        let content = cx.debug_bounds("home-content").unwrap();
        assert_eq!(toolbar.top(), px(0.));
        // TitleBar reserves its bottom pixel for the window separator.
        assert_eq!(toolbar.size.height, crate::theme::toolbar_height() - px(1.));
        assert_eq!(content.top(), crate::theme::toolbar_height());
        for selector in [
            "sidebar-toggle",
            "chat-open",
            "view-grid",
            "management-settings",
        ] {
            let button = cx.debug_bounds(selector).unwrap();
            assert!(button.top() >= toolbar.top() && button.bottom() <= toolbar.bottom());
            assert!(button.left() >= toolbar.left() && button.right() <= toolbar.right());
        }
    }
    click(cx, "chat-open");
    cx.run_until_parked();
    let chat=app.read_with(cx,|app,_|app.chat.window.expect("chat window"));
    let main=cx.update(|window,_|window.window_handle().window_id());
    assert_ne!(chat.window_id(),main);
    chat.update(cx,|_,window,_|window.remove_window()).unwrap();
    cx.run_until_parked();
    click(cx, "sidebar-toggle");
    app.read_with(cx, |app, _| assert!(app.collapsed));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(cx.debug_bounds("home-content").unwrap().left(), px(0.));
    assert!(cx.debug_bounds("management-new").is_none());
    click(cx, "sidebar-toggle");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(
        cx.debug_bounds("home-content").unwrap().left(),
        crate::theme::sidebar_width()
    );
    assert!(cx.debug_bounds("management-new").is_some());
}

#[gpui::test]
fn transfer_folders_are_saved_with_the_connection(cx: &mut TestAppContext) {
    let connection = Connection::new("Office", "example.test", 5900);
    let id = connection.id;
    let (app, cx, dir) = setup(cx, vec![connection]);
    let folders = TransferFolders {
        local: Some("/tmp/rv-last-folder".into()),
        remote: Some("/remote/work".into()),
        photo_mode: true,
    };
    app.update(cx, |app, cx| {
        app.remember_transfer_folders(id, folders.clone(), cx)
    });
    let reloaded = AddressBook::load(rv_core::StorePaths::in_dir(dir.path().into())).unwrap();
    assert_eq!(reloaded.get(id).unwrap().transfer_folders, folders);
}

#[gpui::test]
fn offscreen_app_shortcuts_save_per_connection_and_preserve_empty(cx: &mut TestAppContext) {
    let first = Connection::new("First", "first.test", 5900);
    let second = Connection::new("Second", "second.test", 5900);
    let first_id = first.id;
    let second_id = second.id;
    let (app, cx, dir) = setup(cx, vec![first, second]);
    let mut favorites = rv_core::default_app_shortcuts();
    favorites.reverse();
    app.update(cx, |app, cx| {
        app.remember_app_shortcuts(first_id, favorites.clone(), cx)
            .unwrap()
    });
    let reload = || AddressBook::load(rv_core::StorePaths::in_dir(dir.path().into())).unwrap();
    assert_eq!(reload().get(first_id).unwrap().app_shortcuts, favorites);
    assert_eq!(
        reload().get(second_id).unwrap().app_shortcuts,
        rv_core::default_app_shortcuts()
    );
    app.update(cx, |app, cx| {
        app.remember_app_shortcuts(first_id, vec![], cx).unwrap()
    });
    assert!(reload().get(first_id).unwrap().app_shortcuts.is_empty());
    app.update(cx, |app, cx| {
        app.read_only = true;
        assert!(
            app.remember_app_shortcuts(first_id, favorites.clone(), cx)
                .is_err()
        );
    });
    assert!(reload().get(first_id).unwrap().app_shortcuts.is_empty());
}

#[gpui::test]
fn environment_apps_persist_separately_from_launch_shortcuts(cx: &mut TestAppContext) {
    let first = Connection::new("First", "first.test", 5900);
    let second = Connection::new("Second", "second.test", 5900);
    let first_id = first.id;
    let second_id = second.id;
    let (app, cx, dir) = setup(cx, vec![first, second]);
    let environment = vec![rv_core::AppShortcut {
        bundle_id: "fr.vinted".into(),
        name: "Vinted".into(),
    }];
    app.update(cx, |app, cx| {
        app.remember_environment_apps(first_id, environment.clone(), cx)
            .unwrap()
    });
    let reload = || AddressBook::load(rv_core::StorePaths::in_dir(dir.path().into())).unwrap();
    assert_eq!(
        reload().get(first_id).unwrap().environment_apps,
        environment
    );
    assert!(reload().get(second_id).unwrap().environment_apps.is_empty());
    assert_eq!(
        reload().get(first_id).unwrap().app_shortcuts,
        rv_core::default_app_shortcuts()
    );
    app.update(cx, |app, cx| {
        app.remember_app_shortcuts(first_id, vec![], cx).unwrap()
    });
    assert_eq!(
        reload().get(first_id).unwrap().environment_apps,
        environment
    );
}

#[gpui::test]
fn offscreen_save_connection_persists_form_and_clears_password(cx: &mut TestAppContext) {
    let (app, cx, dir) = setup(cx, vec![]);
    cx.simulate_keystrokes("ctrl-n");
    app.update(cx, |app, cx| {
        assert!(matches!(app.modal, Modal::Connection));
        app.remember_password = false;
        cx.notify();
    });
    cx.simulate_input("example.test:2");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.zxtouch_port_input
                .update(cx, |input, cx| input.set_value("6008", window, cx));
            app.name_input
                .update(cx, |input, cx| input.set_value("Office", window, cx));
            app.username_input
                .update(cx, |input, cx| input.set_value("  mac-user  ", window, cx));
            app.labels_input.update(cx, |input, cx| {
                input.set_value("work, lab, work", window, cx)
            });
            app.password_input.update(cx, |input, cx| {
                input.set_value("temporary test password", window, cx)
            });
        });
    });
    click(cx, "modal-save");
    app.read_with(cx, |app, cx| {
        assert!(matches!(app.modal, Modal::None));
        assert!(!app.credential_busy);
        assert!(app.password_input.read(cx).unmask_value().is_empty());
    });
    let book = AddressBook::load(rv_core::StorePaths::in_dir(dir.path().into())).unwrap();
    let saved = &book.connections()[0];
    assert_eq!(
        (saved.name.as_str(), saved.host.as_str(), saved.port),
        ("Office", "example.test", 5902)
    );
    assert_eq!(saved.zxtouch_port, Some(6008));
    assert_eq!(
        ConnectRequest::from_connection(saved, None).zxtouch_port,
        Some(6008)
    );
    assert_eq!(saved.username.as_deref(), Some("mac-user"));
    assert_eq!(
        ConnectRequest::from_connection(saved, None).username,
        saved.username
    );
    assert_eq!(saved.labels, ["lab", "work"]);
    assert!(!saved.remember_password);
    assert!(saved.last_connected.is_none());
}

#[gpui::test]
fn offscreen_invalid_server_keeps_editor_open(cx: &mut TestAppContext) {
    let (app, cx, _dir) = setup(cx, vec![]);
    cx.simulate_keystrokes("ctrl-n");
    click(cx, "modal-connect");
    app.read_with(cx, |app, _| {
        assert!(matches!(app.modal, Modal::Connection));
        assert!(!app.credential_busy);
        assert!(app.book.connections().is_empty());
        assert!(
            app.connection_error
                .as_ref()
                .is_some_and(|error| !error.is_empty())
        );
    });
}

#[gpui::test]
fn offscreen_local_cursor_mode_cycles_and_persists(cx: &mut TestAppContext) {
    let (app, cx, dir) = setup(cx, vec![]);
    cx.simulate_keystrokes("ctrl-n");
    cx.simulate_input("example.test");
    app.read_with(cx, |app, _| {
        assert_eq!(app.local_cursor_mode, LocalCursorMode::Automatic)
    });
    click(cx, "cursor-mode");
    app.read_with(cx, |app, _| {
        assert_eq!(app.local_cursor_mode, LocalCursorMode::Show)
    });
    click(cx, "modal-save");
    let book = AddressBook::load(rv_core::StorePaths::in_dir(dir.path().into())).unwrap();
    let saved = &book.connections()[0];
    assert_eq!(saved.local_cursor, LocalCursorMode::Show);
    assert_eq!(
        ConnectRequest::from_connection(saved, None).local_cursor,
        LocalCursorMode::Show
    );
}

#[gpui::test]
fn offscreen_clipboard_mode_has_two_choices_and_persists(cx: &mut TestAppContext) {
    let (app, cx, dir) = setup(cx, vec![]);
    cx.simulate_keystrokes("ctrl-n");
    cx.simulate_input("example.test");
    app.read_with(cx, |app, _| {
        assert_eq!(app.clipboard_mode, ClipboardMode::Utf8)
    });
    click(cx, "clipboard-mode");
    app.read_with(cx, |app, _| {
        assert_eq!(app.clipboard_mode, ClipboardMode::Latin1)
    });
    click(cx, "clipboard-mode");
    app.read_with(cx, |app, _| {
        assert_eq!(app.clipboard_mode, ClipboardMode::Utf8)
    });
    click(cx, "clipboard-mode");
    click(cx, "modal-save");
    let book = AddressBook::load(rv_core::StorePaths::in_dir(dir.path().into())).unwrap();
    let saved = &book.connections()[0];
    assert_eq!(saved.clipboard, ClipboardMode::Latin1);
    assert_eq!(
        ConnectRequest::from_connection(saved, None).clipboard,
        ClipboardMode::Latin1
    );
}

#[gpui::test]
fn offscreen_connection_modal_stays_within_minimum_window_height(cx: &mut TestAppContext) {
    let (_app, cx, _dir) = setup(cx, vec![]);
    cx.simulate_resize(size(px(800.), px(520.)));
    cx.simulate_keystrokes("ctrl-n");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let bounds = cx
        .debug_bounds("modal-save")
        .expect("connection modal must be rendered");
    assert!(bounds.origin.y >= px(0.));
    assert!(bounds.bottom() <= px(520.));
}

#[gpui::test]
fn chat_dialog_fits_minimum_window(cx: &mut TestAppContext) {
    let (_app, cx, _dir) = setup(cx, vec![]);
    cx.simulate_resize(size(px(800.), px(520.)));
    click(cx, "chat-open");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let bounds = cx.debug_bounds("chat-panel").unwrap();
    assert!(bounds.left() >= px(0.) && bounds.right() <= px(800.));
    assert!(bounds.top() >= px(0.) && bounds.bottom() <= px(520.));
}

#[gpui::test]
fn offscreen_busy_editor_blocks_buttons_and_duplicate_actions(cx: &mut TestAppContext) {
    let (app, cx, _dir) = setup(cx, vec![]);
    cx.simulate_keystrokes("ctrl-n");
    cx.simulate_input("example.test");
    app.update(cx, |app, cx| {
        app.credential_busy = true;
        app.status = "Loading saved password…".into();
        cx.notify();
    });
    for selector in ["modal-save", "modal-connect", "modal-cancel"] {
        click(cx, selector);
    }
    cx.dispatch_action(NewConnection);
    cx.dispatch_action(CloseModal);
    app.read_with(cx, |app, cx| {
        assert!(app.credential_busy);
        assert!(matches!(app.modal, Modal::Connection));
        assert!(app.book.connections().is_empty());
        assert_eq!(app.server_input.read(cx).value().as_ref(), "example.test");
        assert_eq!(app.status.as_ref(), "Loading saved password…");
    });
    app.update(cx, |app, cx| {
        app.credential_busy = false;
        cx.notify();
    });
    click(cx, "modal-cancel");
    app.read_with(cx, |app, _| assert!(matches!(app.modal, Modal::None)));
}

#[gpui::test]
fn offscreen_properties_save_preserves_remembered_password_setting(cx: &mut TestAppContext) {
    let mut connection = Connection::new("Office", "example.test", 5900);
    connection.remember_password = true;
    connection.username = Some("mac-user".into());
    let id = connection.id;
    let (app, cx, dir) = setup(cx, vec![connection]);
    app.update(cx, |app, _| app.selected = Some(id));
    cx.dispatch_action(OpenProperties);
    app.read_with(cx, |app, cx| {
        assert_eq!(app.editing, Some(id));
        assert_eq!(app.username_input.read(cx).value().as_ref(), "mac-user");
        assert!(app.remember_password);
        assert!(app.password_input.read(cx).unmask_value().is_empty());
    });
    click(cx, "modal-save");
    app.read_with(cx, |app, _| assert!(matches!(app.modal, Modal::None)));
    let book = AddressBook::load(rv_core::StorePaths::in_dir(dir.path().into())).unwrap();
    assert!(book.get(id).unwrap().remember_password);
    assert_eq!(book.get(id).unwrap().username.as_deref(), Some("mac-user"));
}

#[gpui::test]
fn offscreen_successful_password_retry_updates_saved_connection(cx: &mut TestAppContext) {
    let connection = Connection::new("Office", "example.test", 5900);
    let id = connection.id;
    let (app, cx, dir) = setup(cx, vec![connection.clone()]);
    let mut request = ConnectRequest::from_connection(&connection, Some("new password".into()));
    app.update(cx, |app, cx| {
        assert_eq!(
            app.save_retried_password(&request, true, cx).unwrap(),
            Some(id)
        );
    });
    let paths = rv_core::StorePaths::in_dir(dir.path().into());
    assert_eq!(
        paths.load_password(id).unwrap().as_deref(),
        Some("new password")
    );
    assert!(
        AddressBook::load(paths.clone())
            .unwrap()
            .get(id)
            .unwrap()
            .remember_password
    );

    request.password = Some("one-time password".into());
    app.update(cx, |app, cx| {
        assert_eq!(
            app.save_retried_password(&request, false, cx).unwrap(),
            Some(id)
        );
    });
    assert_eq!(paths.load_password(id).unwrap(), None);
    assert!(
        !AddressBook::load(paths)
            .unwrap()
            .get(id)
            .unwrap()
            .remember_password
    );
}

#[gpui::test]
fn offscreen_search_and_view_shortcuts_keep_address_book_intact(cx: &mut TestAppContext) {
    let office = Connection::new("Office", "office.test", 5900);
    let lab = Connection::new("Lab", "lab.test", 5900);
    let (app, cx, _dir) = setup(cx, vec![office, lab]);
    cx.simulate_keystrokes("ctrl-l ctrl-b ctrl-f");
    cx.simulate_input("lab");
    app.read_with(cx, |app, cx| {
        assert!(matches!(app.view_mode, ViewMode::List));
        assert!(app.collapsed);
        assert_eq!(app.query(cx), "lab");
        assert_eq!(app.book.connections().len(), 2);
    });
    cx.simulate_keystrokes("escape");
    app.read_with(cx, |app, cx| assert!(app.query(cx).is_empty()));
}

#[gpui::test]
fn offscreen_username_can_be_cleared_and_does_not_leak_into_new_connection(
    cx: &mut TestAppContext,
) {
    let mut connection = Connection::new("Office", "example.test", 5900);
    connection.username = Some("mac-user".into());
    let id = connection.id;
    let (app, cx, dir) = setup(cx, vec![connection]);
    app.update(cx, |app, _| app.selected = Some(id));
    cx.dispatch_action(OpenProperties);
    click(cx, "modal-cancel");
    cx.update(|window, cx| app.read(cx).focus.clone().focus(window, cx));
    cx.dispatch_action(NewConnection);
    app.read_with(cx, |app, cx| {
        assert!(app.username_input.read(cx).value().is_empty())
    });
    click(cx, "modal-cancel");
    cx.update(|window, cx| app.read(cx).focus.clone().focus(window, cx));
    cx.dispatch_action(OpenProperties);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.username_input
                .update(cx, |input, cx| input.set_value("  ", window, cx));
        })
    });
    click(cx, "modal-save");
    let book = AddressBook::load(rv_core::StorePaths::in_dir(dir.path().into())).unwrap();
    assert!(book.get(id).unwrap().username.is_none());
    assert!(
        ConnectRequest::from_connection(book.get(id).unwrap(), None)
            .username
            .is_none()
    );
}

#[gpui::test]
fn backup_cards_deduplicate_and_edit_persisted_notes(cx: &mut TestAppContext) {
    let device = Connection::new("phone", "example.test", 5900);
    let device_id = device.id;
    let (app, cx, dir) = setup(cx, vec![device]);
    let id = "C9EB7804-2062-4519-AC8D-6F68972574F3";
    let reply = serde_json::json!({"bundle_id":"lt.manodrabuziai.fr","backups":[{"kind":"snapshot","backup_id":"snapshot:1","idfv":id,"display_name":"店铺","note":"来自手机"}]});
    app.update(cx, |app, cx| {
        app.sync_phone_backups(device_id, "lt.manodrabuziai.fr", &reply, cx)
    });
    cx.run_until_parked();
    app.update(cx, |app, cx| {
        app.sync_phone_backups(device_id, "lt.manodrabuziai.fr", &reply, cx)
    });
    cx.run_until_parked();
    app.read_with(cx, |app, _| assert_eq!(app.home.apps.len(), 1));
    let key = rv_core::workspace::AppId::from_idfv(id).unwrap();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_app_card(key.clone(), window, cx);
            app.home
                .note
                .update(cx, |s, cx| s.set_value("本地备注", window, cx));
        })
    });
    app.update(cx, |app, cx| app.save_app_card(cx));
    cx.run_until_parked();
    let store =
        rv_core::workspace::WorkspaceStore::open(&rv_core::StorePaths::in_dir(dir.path().into()))
            .unwrap();
    assert_eq!(store.load(&key).unwrap().note, "本地备注");
}

#[gpui::test]
fn home_filters_preserve_phone_selection_and_all_tag_choices(cx: &mut TestAppContext) {
    let device = Connection::new("phone", "example.test", 5900);
    let device_id = device.id;
    let (app, cx, dir) = setup(cx, vec![device]);
    let id = rv_core::workspace::AppId::from_idfv("C9EB7804-2062-4519-AC8D-6F68972574F3").unwrap();
    let reply = serde_json::json!({"bundle_id":"lt.manodrabuziai.fr","backups":[{"kind":"snapshot","backup_id":"snapshot:1","idfv":id.as_str(),"display_name":"店铺"}]});
    let mut store =
        rv_core::workspace::WorkspaceStore::open(&rv_core::StorePaths::in_dir(dir.path().into()))
            .unwrap();
    store
        .sync_backups(device_id, "lt.manodrabuziai.fr", &reply)
        .unwrap();
    let mut card = store.load(&id).unwrap();
    card.tags = vec!["法国".into(), "工作".into()];
    store.save(&card).unwrap();
    app.update(cx, |app, cx| {
        app.selected = Some(device_id);
        app.reload_home(cx);
    });
    cx.run_until_parked();
    click(cx, "app-filter-1");
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(app.selected, Some(device_id));
        assert!(app.home.apps.is_empty());
        assert_eq!(app.home.available_tags, vec!["工作", "法国"]);
    });
}

#[gpui::test]
fn offscreen_connection_ellipsis_edits_its_target_in_grid_and_list(cx: &mut TestAppContext) {
    let first = Connection::new("First", "first.test", 5900);
    let second = Connection::new("Second", "second.test", 5900);
    let first_id = first.id;
    let second_id = second.id;
    let (app, cx, _dir) = setup(cx, vec![first, second]);
    cx.simulate_resize(size(px(1100.), px(720.)));
    for mode in [ViewMode::Grid, ViewMode::List] {
        app.update(cx, |app, cx| {
            app.view_mode = mode;
            app.selected = Some(first_id);
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let prefix = "device-row";
        let target = cx
            .debug_bounds(Box::leak(format!("{prefix}-{second_id}").into_boxed_str()))
            .unwrap();
        cx.simulate_mouse_move(target.center(), None, Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        click(cx, &format!("connection-menu-{second_id}"));
        // First Down selects Connect, second Down selects Edit.
        cx.simulate_keystrokes("down down enter");
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.editing,
                Some(second_id),
                "editing wrong mode {} first {first_id}",
                mode == ViewMode::Grid
            );
            assert_eq!(
                app.selected,
                Some(second_id),
                "selection wrong mode {} first {first_id}",
                mode == ViewMode::Grid
            );
            assert!(matches!(app.modal, Modal::Connection));
            assert!(!app.credential_busy);
            assert_eq!(app.book.connections().len(), 2);
        });
        app.update(cx, |app, cx| app.close_modal(cx));
    }
}

#[gpui::test]
fn offscreen_validation_feedback_survives_footer_removal(cx: &mut TestAppContext) {
    let (_app, cx, _dir) = setup(cx, vec![]);
    click(cx, "management-new");
    click(cx, "modal-connect");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let feedback = cx
        .debug_bounds("management-feedback")
        .expect("invalid address must stay visible in its editor");
    let modal = cx.debug_bounds("modal-card").unwrap();
    assert!(feedback.origin.y >= modal.origin.y && feedback.bottom() <= modal.bottom());
}

#[gpui::test]
fn offscreen_invalid_zxtouch_port_keeps_editor_open(cx: &mut TestAppContext) {
    let (app, cx, dir) = setup(cx, vec![]);
    cx.simulate_keystrokes("ctrl-n");
    for port in ["0", "65536", "6000x", "-1"] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.server_input.update(cx, |input, cx| {
                    input.set_value("example.test:5901", window, cx)
                });
                app.zxtouch_port_input
                    .update(cx, |input, cx| input.set_value(port, window, cx));
                app.submit_connect(false, window, cx);
                assert!(matches!(app.modal, Modal::Connection));
            });
        });
    }
    assert!(
        AddressBook::load(rv_core::StorePaths::in_dir(dir.path().into()))
            .unwrap()
            .connections()
            .is_empty()
    );
}

#[gpui::test]
fn offscreen_editor_derives_zxtouch_default_and_omits_connection_progress(cx: &mut TestAppContext) {
    let mut phone = rv_core::Connection::new("iPhonX", "172.16.19.47", 5901);
    phone.zxtouch_port = None;
    let (app, cx, dir) = setup(cx, vec![phone.clone()]);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.status = "Connecting to iPhonX…".into();
            app.open_editor(Some(&phone), window, cx);
            assert_eq!(app.zxtouch_port_input.read(cx).value().as_ref(), "6000");
            assert!(app.connection_error.is_none());
        });
        window.draw(cx).clear(cx);
    });
    assert!(cx.debug_bounds("management-feedback").is_none());
    let vnc = cx.debug_bounds("vnc-address-input").unwrap();
    let zx = cx.debug_bounds("zxtouch-port-input").unwrap();
    assert!(zx.top() > vnc.bottom() && zx.top() - vnc.bottom() <= px(20.));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.zxtouch_port_input
                .update(cx, |input, cx| input.set_value("", window, cx));
        })
    });
    click(cx, "modal-save");
    let book = AddressBook::load(rv_core::StorePaths::in_dir(dir.path().into())).unwrap();
    assert_eq!(book.get(phone.id).unwrap().zxtouch_port, Some(6000));
}

#[gpui::test]
fn home_phone_control_reads_metadata_and_opens_environment_without_vnc(cx: &mut TestAppContext) {
    use crate::session_window::{SessionOptions, SessionView};
    use rv_session::{AppEvent, SessionEvent, SessionHandle};
    let device = Connection::new("phone", "example.test", 5901);
    let id = device.id;
    let (app, cx, _dir) = setup(cx, vec![device.clone()]);
    let (handle, peer) = SessionHandle::test_pair();
    let view = cx.update(|window, cx| {
        let options = SessionOptions {
            scale: rv_core::ScaleMode::Fit,
            pin_toolbar: false,
            menu_key: "F8".into(),
            hide_shots: true,
            thumb_path: None,
            app_icon_cache: None,
            address_book: Some(app.downgrade()),
            transfer_folders: Default::default(),
            app_shortcuts: vec![],
            remember_password: false,
        };
        let view = cx.new(|cx| {
            SessionView::with_control_test_handle(
                ConnectRequest::from_connection(&device, None),
                options,
                handle,
                window,
                cx,
            )
        });
        app.update(cx, |app, _| {
            app.home.control_loading.remove(&id);
            app.home.controls.insert(
                id,
                super::home::control::PhoneControl::new(view.clone(), &device),
            );
        });
        peer.events.send(SessionEvent::ControlState(true)).unwrap();
        peer.events
            .send(SessionEvent::Apps(AppEvent::DeviceInfo(
                serde_json::json!({"model":"iPhone Test","os_version":"15.8.8"}),
            )))
            .unwrap();
        view.update(cx, |view, cx| view.pump_control_test(cx));
        app.update(cx, |app, cx| {
            assert!(app.phone_online(id, cx));
            assert!(app.home.sessions.is_empty());
            assert_eq!(
                app.book.get(id).unwrap().model.as_deref(),
                Some("iPhone Test")
            );
            app.show_phone_backups(id, None, window, cx);
            assert!(matches!(app.modal, Modal::PhoneControl(target) if target == id));
            assert!(
                app.home.sessions.is_empty(),
                "home actions must not create a VNC window"
            );
        });
        view
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("function-tab-App 环境").is_some());
    click(cx, "function-close");
    app.read_with(cx, |app, cx| {
        assert!(matches!(app.modal, Modal::None));
        assert!(
            app.phone_online(id, cx),
            "closing the panel must retain phone control"
        );
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("function-close").is_none());
    assert!(
        cx.debug_bounds(Box::leak(
            format!("device-status-online-{id}").into_boxed_str()
        ))
        .is_some()
    );
    assert!(
        cx.debug_bounds(Box::leak(
            format!("device-status-offline-{id}").into_boxed_str()
        ))
        .is_none()
    );
    cx.update(|_, cx| {
        peer.events.send(SessionEvent::ControlState(false)).unwrap();
        view.update(cx, |view, cx| view.pump_control_test(cx));
        app.read_with(cx, |app, cx| assert!(!app.phone_online(id, cx)));
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds(Box::leak(
            format!("device-status-offline-{id}").into_boxed_str()
        ))
        .is_some()
    );
    assert!(
        cx.debug_bounds(Box::leak(
            format!("device-status-online-{id}").into_boxed_str()
        ))
        .is_none()
    );
}

#[gpui::test]
fn saved_password_editor_shows_mask_without_submitting_mask_as_password(cx: &mut TestAppContext) {
    let mut phone = Connection::new("phone", "example.test", 5901);
    phone.remember_password = true;
    let id = phone.id;
    let (app, cx, _dir) = setup(cx, vec![phone.clone()]);
    app.read_with(cx, |app, _| {
        app.book
            .paths()
            .save_password(id, "fixture-secret")
            .unwrap()
    });
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_editor(Some(&phone), window, cx)));
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        let input = app.password_input.read(cx);
        assert_eq!(input.presentation().placeholder().as_ref(), "********");
        assert!(input.unmask_value().is_empty());
    });
    cx.update(|window, cx| app.update(cx, |app, cx| app.submit_connect(false, window, cx)));
    cx.run_until_parked();
    app.read_with(cx, |app, _| {
        assert_eq!(
            app.book.paths().load_password(id).unwrap().as_deref(),
            Some("fixture-secret")
        )
    });
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_editor(None, window, cx)));
    cx.run_until_parked();
    app.read_with(cx, |app, cx| {
        assert_eq!(
            app.password_input
                .read(cx)
                .presentation()
                .placeholder()
                .as_ref(),
            "密码（可选）"
        )
    });
}

#[gpui::test]
fn card_delete_is_local_only_and_keeps_other_idfv(cx: &mut TestAppContext) {
    use rv_core::workspace::{AppId, WorkspaceStore};
    let device = Connection::new("phone", "example.test", 5901);
    let (app, cx, _dir) = setup(cx, vec![device.clone()]);
    let id = AppId::from_idfv("C9EB7804-2062-4519-AC8D-6F68972574F3").unwrap();
    let other = AppId::from_idfv("7A70B21A-2F9D-4F66-81C4-6D21355758D7").unwrap();
    let response = serde_json::json!({"bundle_id":"lt.manodrabuziai.fr","backups":[
        {"backup_id":"snapshot:20261007-01","kind":"snapshot","display_name":"shop","idfv":id.as_str()},
        {"backup_id":"snapshot:20261007-02","kind":"snapshot","display_name":"other","idfv":other.as_str()}
    ]});
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            WorkspaceStore::open(app.book.paths())
                .unwrap()
                .sync_backups(device.id, "lt.manodrabuziai.fr", &response)
                .unwrap();
            app.open_app_delete(id.clone(), cx);
            assert!(!app.home.deletion.as_ref().unwrap().phone);
        })
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("delete-phone-backup").is_none());
    click(cx, "confirm-app-delete");
    app.read_with(cx, |app, _| {
        assert!(matches!(app.modal, Modal::None));
        let store = WorkspaceStore::open(app.book.paths()).unwrap();
        assert!(store.load(&id).is_err());
        assert!(store.load(&other).is_ok());
        assert!(
            store
                .phone_state(device.id, "lt.manodrabuziai.fr")
                .unwrap()
                .is_some()
        );
    });
}

#[gpui::test]
fn shop_tags_edit_and_local_deletion_do_not_call_phone(cx: &mut TestAppContext) {
    use crate::session_window::{SessionOptions, SessionView};
    use rv_core::workspace::{AppId, WorkspaceStore};
    use rv_session::{SessionEvent, SessionHandle};
    let device = Connection::new("phone", "example.test", 5901);
    let (app, cx, _dir) = setup(cx, vec![device.clone()]);
    let id = AppId::from_idfv("C9EB7804-2062-4519-AC8D-6F68972574F3").unwrap();
    let (handle, mut peer) = SessionHandle::test_pair();
    let _view = cx.update(|window, cx| {
        let view = cx.new(|cx| SessionView::with_control_test_handle(
            ConnectRequest::from_connection(&device, None),
            SessionOptions { scale: rv_core::ScaleMode::Fit, pin_toolbar: false,
                menu_key: "F8".into(), hide_shots: true, thumb_path: None,
                app_icon_cache: None,
                address_book: Some(app.downgrade()), transfer_folders: Default::default(),
                app_shortcuts: vec![], remember_password: false }, handle, window, cx));
        app.update(cx, |app, cx| {
            app.home.control_loading.remove(&device.id);
            app.home.controls.insert(device.id, super::home::control::PhoneControl::new(view.clone(), &device));
            let mut store = WorkspaceStore::open(app.book.paths()).unwrap();
            store.sync_backups(device.id, "lt.manodrabuziai.fr", &serde_json::json!({"bundle_id":"lt.manodrabuziai.fr", "backups":[
                {"backup_id":"snapshot:fixture", "kind":"snapshot", "display_name":"shop", "idfv":id.as_str()}
            ]})).unwrap();
            let mut card = store.load(&id).unwrap();
            card.tags = vec!["France".into(), "Fashion".into()];
            store.save(&card).unwrap();
            app.reload_home(cx);
        });
        peer.events.send(SessionEvent::ControlState(true)).unwrap();
        view.update(cx, |view, cx| view.pump_control_test(cx));
        view
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("shop-product-skeleton").is_none());
    assert!(
        cx.debug_bounds(Box::leak(
            format!("app-card-tags-{}", id.as_str()).into_boxed_str()
        ))
        .is_some()
    );
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_app_detail(id.clone(), window, cx)));
    app.read_with(cx, |app, _| assert!(matches!(app.modal, Modal::Shop)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("shop-detail-skeleton").is_none());
    app.update(cx, |app, cx| app.close_modal(cx));
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_app_card(id.clone(), window, cx)));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("app-edit-form").is_some());
    assert!(
        cx.debug_bounds("shop-detail-skeleton").is_none(),
        "editing must not render business data"
    );
    assert!(cx.debug_bounds("shop-operations").is_none());
    app.update(cx, |app, cx| app.close_modal(cx));
    app.update(cx, |app, cx| {
        app.open_app_delete(id.clone(), cx);
        app.confirm_app_delete(cx);
    });
    app.read_with(cx, |app, _| {
        assert!(app.home.deletion.is_none());
        assert!(
            WorkspaceStore::open(app.book.paths())
                .unwrap()
                .load(&id)
                .is_err()
        );
        assert!(matches!(app.modal, Modal::None));
    });
    assert!(std::iter::from_fn(|| peer.commands.try_recv().ok()).all(|command| !matches!(command,
        rv_session::SessionCommand::Function(rv_session::FunctionCommand::Invoke {id,..}) if id=="app-delete")));
}

#[gpui::test]
fn proxy_manager_edits_credentials_and_card_editor_keeps_fixed_binding(cx: &mut TestAppContext) {
    use rv_core::workspace::{AppId, ProxyMode, SocksProxy, WorkspaceStore};
    let device = Connection::new("phone", "example.test", 5900);
    let device_id = device.id;
    let (app, cx, dir) = setup(cx, vec![device]);
    let paths = rv_core::StorePaths::in_dir(dir.path().into());
    let id = AppId::from_idfv("C9EB7804-2062-4519-AC8D-6F68972574F3").unwrap();
    let proxy = SocksProxy {
        protocol: Default::default(),
        front_proxy_id: None,
        id: uuid::Uuid::new_v4().to_string(),
        name: "Shop exit".into(),
        host: "127.0.0.1".into(),
        port: 1080,
        username: "user".into(),
        password: "actual-secret".into(),
        mode: ProxyMode::Exclusive,
        ip: Default::default(),
    };
    let mut store = WorkspaceStore::open(&paths).unwrap();
    store.sync_backups(device_id,"lt.manodrabuziai.fr",&serde_json::json!({"bundle_id":"lt.manodrabuziai.fr","backups":[{"kind":"snapshot","backup_id":"snapshot:1","idfv":id.as_str(),"display_name":"shop"}]})).unwrap();
    store.save_proxy(&proxy).unwrap();
    store.bind_proxy(&id, Some(&proxy.id)).unwrap();
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_proxy_manager(window, cx)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("settings-proxies").is_some());
    click(cx, "proxy-save");
    let saved = store.proxies().unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].password, "actual-secret");
    assert_eq!(saved[0].mode, ProxyMode::Exclusive);
    click(cx, "settings-back");
    app.read_with(cx, |app, _| assert!(matches!(app.modal, Modal::None)));
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_app_card(id.clone(), window, cx)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("app-proxy-binding").is_some());
    app.update(cx, |app, cx| app.save_app_card(cx));
    assert_eq!(store.proxy_binding(&id).unwrap(), Some(proxy.id.clone()));
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_app_card(id.clone(), window, cx)));
    app.read_with(cx, |app, cx| {
        assert_eq!(
            app.home.proxies.binding_select.read(cx).selected_value(),
            Some(&proxy.id)
        );
    });
    app.update(cx, |app, cx| {
        app.home.proxies.binding_select.update(cx, |_, cx| {
            cx.emit(gpui_component::select::SelectEvent::<
                Vec<super::home::ProxyChoice>,
            >::Confirm(Some(String::new())));
        });
    });
    cx.run_until_parked();
    app.update(cx, |app, cx| app.save_app_card(cx));
    assert_eq!(store.proxy_binding(&id).unwrap(), None);
}

#[gpui::test]
fn full_window_settings_keeps_home_state_and_cross_page_focus(cx: &mut TestAppContext) {
    use super::settings::SettingsPage;
    use gpui::Focusable;
    let phone = Connection::new("phone", "example.test", 5900);
    let id = phone.id;
    let (app, cx, _dir) = setup(cx, vec![phone]);
    app.update(cx, |app, _| {
        app.selected = Some(id);
        app.collapsed = true;
    });
    cx.update(|window, cx| app.update(cx, |app, cx| app.on_prefs(&OpenPreferences, window, cx)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("settings-page").is_some());
    assert!(cx.debug_bounds("home-content").is_none());
    assert!(cx.debug_bounds("management-toolbar").is_none());
    cx.simulate_keystrokes("ctrl-n");
    app.read_with(cx, |app, _| assert!(matches!(app.modal, Modal::None)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.settings.theme.focus_handle(cx).focus(window, cx);
            app.select_settings_page(SettingsPage::Proxies, window, cx);
            assert!(app.focus.is_focused(window));
            app.home
                .proxies
                .front_select
                .focus_handle(cx)
                .focus(window, cx);
            app.select_settings_page(SettingsPage::General, window, cx);
            assert!(app.focus.is_focused(window));
            app.on_focus_search(&FocusSearch, window, cx);
            assert!(!app.search.focus_handle(cx).is_focused(window));
        })
    });
    click(cx, "settings-back");
    app.read_with(cx, |app, _| {
        assert!(app.settings.page.is_none());
        assert_eq!(app.selected, Some(id));
        assert!(app.collapsed);
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("home-content").is_some());
}

#[gpui::test]
fn settings_component_switches_pages_without_search(cx: &mut TestAppContext) {
    use gpui::Focusable;
    let (app, cx, _dir) = setup(cx, vec![]);
    cx.simulate_resize(size(px(1100.), px(720.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_settings(window, cx);
        })
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("settings-general").is_some());
    assert!(cx.debug_bounds("settings-proxies").is_none());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.settings.theme.focus_handle(cx).focus(window, cx);
        });
    });
    let root = cx.debug_bounds("settings-page").unwrap();
    let proxy_menu = point(root.left() + px(80.), root.top() + px(74.));
    cx.simulate_mouse_move(proxy_menu, None, Modifiers::default());
    cx.simulate_click(proxy_menu, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("settings-proxies").is_some());
    app.read_with(cx, |app, _| {
        assert_eq!(
            app.settings.page,
            Some(super::settings::SettingsPage::Proxies)
        );
    });
    cx.update(|window, cx| {
        assert!(app.read(cx).focus.is_focused(window));
    });
    cx.update(|window, cx| app.update(cx, |app, cx| app.on_close_modal(&CloseModal, window, cx)));
    app.read_with(cx, |app, _| assert!(app.settings.page.is_none()));
}

#[gpui::test]
fn closing_shop_removes_the_component_dialog_without_an_empty_window(cx: &mut TestAppContext) {
    use gpui_component::WindowExt;
    use rv_core::workspace::{AppId, WorkspaceStore};
    let device = Connection::new("fixture", "127.0.0.1", 9);
    let (app, cx, _dir) = setup(cx, vec![device.clone()]);
    let id = AppId::from_idfv("C9EB7804-2062-4519-AC8D-6F68972574F3").unwrap();
    app.update(cx, |app, cx| {
        WorkspaceStore::open(app.book.paths())
            .unwrap()
            .sync_backups(
                device.id,
                "lt.manodrabuziai.fr",
                &serde_json::json!({"bundle_id":"lt.manodrabuziai.fr", "backups":[{
                    "backup_id":format!("snapshot:{}", id.as_str()), "kind":"snapshot",
                    "display_name":"backup title", "idfv":id.as_str()
                }]}),
            )
            .unwrap();
        app.reload_home(cx);
    });
    cx.run_until_parked();
    for _ in 0..3 {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.open_app_detail(id.clone(), window, cx))
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        app.read_with(cx, |app, _| assert!(matches!(app.modal, Modal::Shop)));
        click(cx, "shop-close");
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            assert!(
                !window.has_active_dialog(cx),
                "closing the shop must leave no empty dialog layer"
            );
        });
        app.read_with(cx, |app, _| assert!(matches!(app.modal, Modal::None)));
        assert!(
            cx.debug_bounds("management-dialog-layer").is_none(),
            "closed dialog must not leave a cached rendered layer"
        );
    }
    // A child dialog shares the shop's stack. Closing the owner must remove both,
    // rather than leaving the owner's builder to render an empty default Dialog.
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_app_detail(id.clone(), window, cx)));
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.open_dialog(cx, |dialog, _, _| dialog.title("child"));
    });
    app.update(cx, |app, cx| app.close_modal(cx));
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        assert!(
            !window.has_active_dialog(cx),
            "closing the shop must also remove its child layers"
        );
    });
    // Match native rendering order: the shell paints overlays before its child
    // AddressBookApp is rendered. A forced second draw used to hide this bug.
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_app_detail(id.clone(), window, cx)));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| app.close_modal(cx));
        let shell = Root::read(window, cx)
            .view()
            .clone()
            .downcast::<WindowRoot>()
            .unwrap();
        shell.update(cx, |_, cx| cx.notify());
        window.draw(cx).clear(cx);
    });
    assert!(
        cx.debug_bounds("management-dialog-layer").is_none(),
        "the first closing frame must not paint a default empty Dialog"
    );
}

#[gpui::test]
fn phone_handoff_cancel_keeps_rv_owner_and_shop_open(cx: &mut TestAppContext) {
    use crate::session_window::{SessionOptions, SessionView};
    use rv_core::workspace::{AppId, ExecutionMode, WorkspaceStore};
    use rv_session::{SessionEvent, SessionHandle};
    let device = Connection::new("fixture", "127.0.0.1", 9);
    let (app, cx, _dir) = setup(cx, vec![device.clone()]);
    let id = AppId::from_idfv("C9EB7804-2062-4519-AC8D-6F68972574F3").unwrap();
    let (handle, peer) = SessionHandle::test_pair();
    let _view = cx.update(|window,cx| {
        let view=cx.new(|cx| SessionView::with_control_test_handle(
            ConnectRequest::from_connection(&device,None),
            SessionOptions { scale:rv_core::ScaleMode::Fit,pin_toolbar:false,menu_key:"F8".into(),hide_shots:true,thumb_path:None,app_icon_cache:None,address_book:Some(app.downgrade()),transfer_folders:Default::default(),app_shortcuts:vec![],remember_password:false },handle,window,cx));
        app.update(cx,|app,cx| {
            app.home.control_loading.remove(&device.id);
            app.home.controls.insert(device.id,super::home::control::PhoneControl::new(view.clone(),&device));
            WorkspaceStore::open(app.book.paths()).unwrap().sync_backups(device.id,"lt.manodrabuziai.fr",&serde_json::json!({"bundle_id":"lt.manodrabuziai.fr","backups":[{"backup_id":format!("snapshot:{}",id.as_str()),"kind":"snapshot","display_name":"fixture backup","idfv":id.as_str()}]})).unwrap();
            app.reload_home(cx);
        });
        peer.events.send(SessionEvent::ControlState(true)).unwrap();
        view.update(cx,|view,cx|view.pump_control_test(cx));
        view
    });
    cx.run_until_parked();
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_app_detail(id.clone(), window, cx)));
    click(cx, "shop-handoff-phone");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    app.read_with(cx, |app, _| {
        assert!(
            matches!(app.modal, Modal::Shop),
            "cancelling handoff must retain the shop"
        );
        let store = WorkspaceStore::open(app.book.paths()).unwrap();
        assert_eq!(store.execution(&id).unwrap().mode, ExecutionMode::Rv);
        assert!(app.home.handoff_running.is_empty());
    });
}

#[gpui::test]
fn rv_takeover_confirmation_requires_acknowledgement_and_cancel_has_no_effect(
    cx: &mut TestAppContext,
) {
    use gpui_component::{WindowExt, dialog::Confirm};
    use std::{cell::Cell, rc::Rc};
    let (_app, cx, _dir) = setup(cx, vec![]);
    let confirmations = Rc::new(Cell::new(0));
    let receiver = confirmations.clone();
    cx.update(|window, cx| {
        crate::ui::confirm_execution_handoff(
            crate::ui::ExecutionHandoffPrompt {
                title: "交给 RV",
                description: crate::ui::RV_HANDOFF_DESCRIPTION,
                icon: crate::assets::FunctionIcon::Monitor,
                device: "iPhone7Plus".into(),
                backup: "fixture backup".into(),
                acknowledgement: Some("已保存最新备份并停止手机 App"),
            },
            window,
            cx,
            move |_, _| receiver.set(receiver.get() + 1),
        )
    });
    click(cx, "handoff-confirm");
    assert_eq!(
        confirmations.get(),
        0,
        "unchecked confirmation button must be disabled"
    );
    cx.dispatch_action(Confirm { secondary: false });
    assert_eq!(
        confirmations.get(),
        0,
        "Enter must not bypass acknowledgement"
    );
    click(cx, "handoff-acknowledgement");
    click(cx, "handoff-confirm");
    assert_eq!(confirmations.get(), 1);
    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
    let receiver = confirmations.clone();
    cx.update(|window, cx| {
        crate::ui::confirm_execution_handoff(
            crate::ui::ExecutionHandoffPrompt {
                title: "交给 RV",
                description: crate::ui::RV_HANDOFF_DESCRIPTION,
                icon: crate::assets::FunctionIcon::Monitor,
                device: "iPhone7Plus".into(),
                backup: "fixture backup".into(),
                acknowledgement: Some("已保存最新备份并停止手机 App"),
            },
            window,
            cx,
            move |_, _| receiver.set(receiver.get() + 1),
        )
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(confirmations.get(), 1, "cancel cannot dispatch takeover");
    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
}

#[gpui::test]
fn shop_rv_takeover_cancel_preserves_phone_source_and_shop(cx: &mut TestAppContext) {
    use crate::session_window::{SessionOptions, SessionView};
    use rv_core::workspace::{AppId, ExecutionMode, WorkspaceStore};
    use rv_session::{SessionEvent, SessionHandle};
    let device = Connection::new("fixture", "127.0.0.1", 9);
    let (app, cx, _dir) = setup(cx, vec![device.clone()]);
    let id = AppId::from_idfv("C9EB7804-2062-4519-AC8D-6F68972574F3").unwrap();
    let (handle, peer) = SessionHandle::test_pair();
    let _view = cx.update(|window,cx| {
        let view=cx.new(|cx| SessionView::with_control_test_handle(
            ConnectRequest::from_connection(&device,None),
            SessionOptions { scale:rv_core::ScaleMode::Fit,pin_toolbar:false,menu_key:"F8".into(),hide_shots:true,thumb_path:None,app_icon_cache:None,address_book:Some(app.downgrade()),transfer_folders:Default::default(),app_shortcuts:vec![],remember_password:false },handle,window,cx));
        app.update(cx,|app,cx| {
            app.home.control_loading.remove(&device.id);
            app.home.controls.insert(device.id,super::home::control::PhoneControl::new(view.clone(),&device));
            WorkspaceStore::open(app.book.paths()).unwrap().sync_backups(device.id,"lt.manodrabuziai.fr",&serde_json::json!({"bundle_id":"lt.manodrabuziai.fr","backups":[{"backup_id":format!("snapshot:{}",id.as_str()),"kind":"snapshot","display_name":"fixture backup","idfv":id.as_str()}]})).unwrap();
            WorkspaceStore::open(app.book.paths()).unwrap().set_token_source(&id,rv_core::workspace::TokenSource::Phone(Some(device.id))).unwrap();
            app.reload_home(cx);
        });
        peer.events.send(SessionEvent::ControlState(true)).unwrap();
        view.update(cx,|view,cx|view.pump_control_test(cx));
        view
    });
    cx.run_until_parked();
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_app_detail(id.clone(), window, cx)));
    click(cx, "shop-handoff-rv");
    click(cx, "handoff-confirm");
    app.read_with(cx,|app,_|assert!(app.home.handoff_running.is_empty(),"unacknowledged takeover must not dispatch"));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    app.read_with(cx, |app, _| {
        assert!(
            matches!(app.modal, Modal::Shop),
            "cancelling handoff must retain the shop"
        );
        let store = WorkspaceStore::open(app.book.paths()).unwrap();
        assert_eq!(store.execution(&id).unwrap().mode, ExecutionMode::Rv);
        assert_eq!(store.token_source(&id).unwrap(),rv_core::workspace::TokenSource::Phone(Some(device.id)));
        assert!(app.home.handoff_running.is_empty());
    });
}
