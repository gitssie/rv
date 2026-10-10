use super::{AddressBookApp, Modal, ViewMode, WindowRoot};
use crate::actions::*;
use gpui::{AppContext, Entity, Modifiers, TestAppContext, VisualTestContext, px, size};
use gpui_component::Root;
use rv_core::{
    AddressBook, ClipboardMode, ConnectRequest, Connection, LocalCursorMode, TransferFolders,
};
use tempfile::TempDir;

fn setup(
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
        crate::bind_keys(cx);
    });
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| AddressBookApp::with_book(book, false, "Ready".into(), window, cx));
        entity = Some(app.clone());
        let shell = cx.new(|_| WindowRoot::new(app));
        Root::new(shell, window, cx)
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    (entity.unwrap(), cx, dir)
}

fn click(cx: &mut VisualTestContext, selector: &str) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let bounds = cx
        .debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
        .expect("button must be rendered");
    assert!(bounds.size.width > px(0.) && bounds.size.height > px(0.));
    cx.simulate_mouse_move(bounds.center(), None, Modifiers::default());
    cx.simulate_click(bounds.center(), Modifiers::default());
    cx.run_until_parked();
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
        .debug_bounds("modal-card")
        .expect("connection modal must be rendered");
    assert!(bounds.origin.y >= px(0.));
    assert!(bounds.bottom() <= px(520.));
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
    let lab_id = lab.id;
    let (app, cx, _dir) = setup(cx, vec![office, lab]);
    cx.simulate_keystrokes("ctrl-l ctrl-b ctrl-f");
    cx.simulate_input("lab");
    app.read_with(cx, |app, cx| {
        assert!(matches!(app.view_mode, ViewMode::List));
        assert!(app.collapsed);
        assert_eq!(
            app.visible(cx).iter().map(|c| c.id).collect::<Vec<_>>(),
            [lab_id]
        );
        assert_eq!(app.book.connections().len(), 2);
    });
    cx.simulate_keystrokes("escape");
    app.read_with(cx, |app, cx| assert_eq!(app.visible(cx).len(), 2));
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
fn offscreen_management_cards_fit_grid_and_keep_actions_inside(cx: &mut TestAppContext) {
    let connections: Vec<_> = (0..6)
        .map(|i| Connection::new(format!("Device {i}"), "example.test", 5900))
        .collect();
    let ids: Vec<_> = connections.iter().map(|conn| conn.id).collect();
    let (app, cx, _dir) = setup(cx, connections);
    for (width, height, columns) in [(1100., 720., 3), (800., 520., 2)] {
        cx.simulate_resize(size(px(width), px(height)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.selected = Some(ids[0]);
                app.credential_busy = true;
                cx.notify();
            });
            window.draw(cx).clear(cx);
        });
        let first = cx
            .debug_bounds(Box::leak(format!("card-{}", ids[0]).into_boxed_str()))
            .unwrap();
        for id in &ids {
            let card = cx
                .debug_bounds(Box::leak(format!("card-{id}").into_boxed_str()))
                .unwrap();
            assert!(card.origin.x >= px(0.) && card.right() <= px(width));
            assert_eq!(card.size.width, first.size.width);
        }
        let next_row = cx
            .debug_bounds(Box::leak(format!("card-{}", ids[columns]).into_boxed_str()))
            .unwrap();
        assert!(next_row.origin.y >= first.bottom());
        let connect = cx
            .debug_bounds(Box::leak(
                format!("connection-connect-{}", ids[0]).into_boxed_str(),
            ))
            .unwrap();
        assert_eq!(connect.size.height, px(28.));
        assert!(connect.origin.x >= first.origin.x && connect.right() <= first.right());
        assert!(connect.bottom() <= first.bottom());
        click(cx, &format!("connection-connect-{}", ids[0]));
        app.read_with(cx, |app, _| {
            assert_eq!(app.selected, Some(ids[0]));
            assert!(app.credential_busy);
            assert!(matches!(app.modal, Modal::None));
        });
    }
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
        let prefix = if mode == ViewMode::Grid {
            "card"
        } else {
            "row"
        };
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
