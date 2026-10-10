//! Optional native Metal previews, isolated from the user's address book.
//! Run: cargo test -p rv-app export_management_previews -- --ignored --test-threads=1
use super::{AddressBookApp, Modal, ViewMode, WindowRoot, unix_now};
use gpui::{AppContext, HeadlessAppContext, px, size};
use gpui_component::Root;
use rv_core::{AddressBook, Connection, StorePaths, ThemePref};
use std::{path::Path, sync::Arc};

#[test]
#[ignore = "manual macOS Metal screenshot export"]
fn export_management_previews() {
    let platform = gpui_platform::current_platform(true);
    let mut cx = HeadlessAppContext::with_platform(
        platform.text_system(),
        Arc::new(crate::assets::Assets),
        gpui_platform::current_headless_renderer,
    );
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::bind_keys(cx);
    });
    let dir = tempfile::tempdir().unwrap();
    let mut book = AddressBook::load(StorePaths::in_dir(dir.path().to_owned())).unwrap();
    let mut first_id = None;
    for (index, (name, label, age)) in [
        ("测试 iPhone", "测试", 0),
        ("办公 Mac", "工作", 600),
        ("家用 Mac mini", "家用", 86400),
        ("备用 iPhone", "测试", 172800),
        ("开发主机", "工作", 259200),
        ("客厅电脑", "家用", 604800),
    ]
    .into_iter()
    .enumerate()
    {
        let mut connection = Connection::new(name, format!("192.168.1.{}", 20 + index), 5900);
        connection.labels = vec![label.into()];
        connection.last_connected = Some(unix_now() - age);
        first_id.get_or_insert(connection.id);
        // Synthetic thumbnail fixtures make portrait containment visible; they are not real devices.
        let (width, height) = if name.contains("iPhone") {
            (160, 320)
        } else {
            (480, 270)
        };
        let preview = image::ImageBuffer::from_fn(width, height, |x, y| {
            let tile = (x / 40 + y / 40) % 2;
            image::Rgba([
                30 + (index * 12) as u8,
                65 + tile as u8 * 28,
                105 + (y * 80 / height) as u8,
                255,
            ])
        });
        preview
            .save(book.paths().thumb_path(connection.id))
            .unwrap();
        book.upsert(connection);
    }
    book.save().unwrap();
    let output = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
    for (name, mode, appearance, width, height, dialog) in [
        (
            "management-ui-implemented-dark.png",
            ViewMode::Grid,
            ThemePref::Dark,
            1100.,
            720.,
            0,
        ),
        (
            "management-ui-implemented-light.png",
            ViewMode::Grid,
            ThemePref::Light,
            1100.,
            720.,
            0,
        ),
        (
            "management-ui-implemented-compact.png",
            ViewMode::List,
            ThemePref::Dark,
            800.,
            520.,
            0,
        ),
        (
            "management-ui-crud-new.png",
            ViewMode::Grid,
            ThemePref::Dark,
            1100.,
            800.,
            1,
        ),
        (
            "management-ui-crud-edit.png",
            ViewMode::Grid,
            ThemePref::Dark,
            1100.,
            800.,
            2,
        ),
        (
            "management-ui-crud-delete.png",
            ViewMode::Grid,
            ThemePref::Dark,
            1100.,
            720.,
            3,
        ),
        (
            "management-ui-crud-settings.png",
            ViewMode::Grid,
            ThemePref::Light,
            1100.,
            720.,
            4,
        ),
        (
            "management-ui-crud-new-light.png",
            ViewMode::Grid,
            ThemePref::Light,
            1100.,
            800.,
            1,
        ),
        (
            "management-ui-crud-edit-light.png",
            ViewMode::Grid,
            ThemePref::Light,
            1100.,
            800.,
            2,
        ),
        (
            "management-ui-crud-delete-light.png",
            ViewMode::Grid,
            ThemePref::Light,
            1100.,
            720.,
            3,
        ),
        (
            "management-ui-crud-settings-dark.png",
            ViewMode::Grid,
            ThemePref::Dark,
            1100.,
            720.,
            4,
        ),
    ] {
        let mut book = AddressBook::load(StorePaths::in_dir(dir.path().to_owned())).unwrap();
        book.prefs_mut().theme = appearance;
        let handle = cx
            .open_window(size(px(width), px(height)), |window, cx| {
                let app =
                    cx.new(|cx| AddressBookApp::with_book(book, false, "Ready".into(), window, cx));
                app.update(cx, |app, cx| {
                    app.selected = first_id;
                    app.view_mode = mode;
                    match dialog {
                        1 => app.on_new(&super::NewConnection, window, cx),
                        2 => app.on_properties(&super::OpenProperties, window, cx),
                        3 => {
                            app.modal = Modal::DeleteConfirm {
                                id: first_id.unwrap(),
                                name: "测试 iPhone".into(),
                            }
                        }
                        4 => app.modal = Modal::Preferences,
                        _ => {}
                    }
                });
                let shell = cx.new(|_| WindowRoot::new(app));
                cx.new(|cx| Root::new(shell, window, cx))
            })
            .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
        })
        .unwrap();
        cx.capture_screenshot(handle.into())
            .unwrap()
            .save(output.join(name))
            .unwrap();
    }
}

#[test]
#[ignore = "manual macOS Metal screenshot export"]
fn export_connection_port_preview() {
    let platform = gpui_platform::current_platform(true);
    let mut cx = HeadlessAppContext::with_platform(
        platform.text_system(),
        Arc::new(crate::assets::Assets),
        gpui_platform::current_headless_renderer,
    );
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::bind_keys(cx);
    });
    let dir = tempfile::tempdir().unwrap();
    let mut book = AddressBook::load(StorePaths::in_dir(dir.path().to_owned())).unwrap();
    book.prefs_mut().theme = ThemePref::Light;
    let connection = Connection::new("iPhonX", "172.16.19.47", 5901);
    book.upsert(connection.clone());
    let handle = cx
        .open_window(size(px(900.), px(800.)), |window, cx| {
            let app = cx.new(|cx| {
                AddressBookApp::with_book(book, false, "Connecting to iPhonX…".into(), window, cx)
            });
            app.update(cx, |app, cx| app.open_editor(Some(&connection), window, cx));
            let shell = cx.new(|_| WindowRoot::new(app));
            cx.new(|cx| Root::new(shell, window, cx))
        })
        .unwrap();
    for _ in 0..3 {
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
    }
    cx.capture_screenshot(handle.into())
        .unwrap()
        .save(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../docs/connection-editor-zxtouch-default.png"),
        )
        .unwrap();
}
