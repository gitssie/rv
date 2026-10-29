//! Native renderer snapshots of the real functional menu, using a mock frame.
use super::{SessionOptions, SessionView};
use gpui::{AppContext, HeadlessAppContext, px, size};
use gpui_component::Root;
use rv_core::{ConnectRequest, Connection, ScaleMode, ThemePref};
use rv_session::{SessionEvent, SessionHandle};
use std::{sync::Arc, time::Duration};

#[test]
#[ignore = "manual macOS Metal screenshot export"]
fn export_function_previews() {
    let platform = gpui_platform::current_platform(true);
    let mut cx = HeadlessAppContext::with_platform(
        platform.text_system(),
        Arc::new(crate::assets::Assets),
        gpui_platform::current_headless_renderer,
    );
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::bind_keys(cx);
        cx.set_reduce_motion(true);
    });
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
    for (name, id, width, height) in [
        ("apple-ui-menu.png", "", 1100., 760.),
        (
            "apple-ui-menu-signed-out.png",
            "menu-signed-out",
            640.,
            620.,
        ),
        (
            "apple-ui-menu-store-only.png",
            "menu-store-only",
            640.,
            620.,
        ),
        ("apple-ui-icloud.png", "cloud-login", 640., 400.),
        (
            "apple-ui-authorizations.png",
            "authorization-result",
            1100.,
            760.,
        ),
        ("apple-ui-devices.png", "devices-result", 640., 400.),
        ("apple-ui-code.png", "login-challenge", 640., 400.),
        ("apple-ui-unknown.png", "unknown-operation", 640., 400.),
        ("apple-ui-email-unknown.png", "email-unknown", 640., 400.),
        (
            "apple-ui-confirmation.png",
            "authorization-confirmation",
            640.,
            400.,
        ),
        ("apple-ui-name.png", "name", 1100., 760.),
        ("zxtouch-email-processing.png", "email-loading", 640., 400.),
        (
            "zxtouch-query-raw-implemented-light.png",
            "query-result",
            640.,
            400.,
        ),
        (
            "zxtouch-query-raw-implemented-dark.png",
            "query-result",
            1100.,
            760.,
        ),
        (
            "zxtouch-query-loading.png",
            "authorization-list",
            640.,
            400.,
        ),
        ("zxtouch-functions-implemented.png", "", 1100., 760.),
        (
            "zxtouch-notifications-implemented-light.png",
            "notifications",
            640.,
            400.,
        ),
        ("zxtouch-function-menu-light.png", "", 640., 620.),
        (
            "zxtouch-logout-result-compact.png",
            "logout-success",
            640.,
            400.,
        ),
        (
            "zxtouch-icloud-logout-light.png",
            "cloud-logout",
            640.,
            400.,
        ),
        (
            "zxtouch-app-first-common-dark.png",
            "env-common",
            1100.,
            760.,
        ),
        (
            "zxtouch-app-first-records-dark.png",
            "env-records",
            1100.,
            760.,
        ),
        (
            "zxtouch-app-first-records-light.png",
            "env-records",
            1100.,
            760.,
        ),
        (
            "zxtouch-environment-catalog-light.png",
            "env-catalog",
            640.,
            400.,
        ),
        ("zxtouch-app-first-save-light.png", "env-save", 640., 400.),
        ("zxtouch-app-save-light.png", "app-save", 640., 400.),
        ("zxtouch-app-save-dark.png", "app-save", 640., 400.),
        (
            "zxtouch-login-text-implemented.png",
            "store-login",
            1100.,
            760.,
        ),
        (
            "zxtouch-app-text-implemented.png",
            "app-switch",
            1100.,
            760.,
        ),
        (
            "zxtouch-functions-compact-implemented.png",
            "store-login",
            640.,
            400.,
        ),
    ] {
        if std::env::var("RV_PREVIEW_FILTER")
            .ok()
            .is_some_and(|filter| !name.contains(&filter))
        {
            continue;
        }
        let (session, peer) = SessionHandle::test_pair();
        {
            let mut fb = session.framebuffer.lock().unwrap();
            fb.resize(390, 844);
            for (i, pixel) in fb.pixels.chunks_exact_mut(4).enumerate() {
                let v = if (i % 390 / 65 + i / 390 / 65) % 2 == 0 {
                    34
                } else {
                    46
                };
                pixel.copy_from_slice(&[v + 25, v + 12, v, 255]);
            }
        }
        peer.events
            .send(SessionEvent::Connected {
                width: 390,
                height: 844,
            })
            .unwrap();
        peer.events
            .send(SessionEvent::FrameReady { generation: 1 })
            .unwrap();
        let req = ConnectRequest::from_connection(
            &Connection::new("我的 iPhone", "fixture.test", 5901),
            None,
        );
        let mut preview_view = None;
        let handle = cx
            .open_window(size(px(width), px(height)), |window, cx| {
                crate::theme::apply(
                    if name.contains("-light") {
                        ThemePref::Light
                    } else {
                        ThemePref::Dark
                    },
                    window,
                    cx,
                );
                let view = cx.new(|cx| {
                    SessionView::with_handle(
                        req,
                        "我的 iPhone".into(),
                        SessionOptions {
                            scale: ScaleMode::Fit,
                            pin_toolbar: true,
                            menu_key: "f8".into(),
                            hide_shots: true,
                            thumb_path: None,
                            app_icon_cache: None,
                            address_book: None,
                            transfer_folders: Default::default(),
                            app_shortcuts: rv_core::default_app_shortcuts(),
                            remember_password: false,
                        },
                        session,
                        window,
                        cx,
                    )
                });
                preview_view = Some(view.clone());
                view.update(cx, |view, cx| {
                    view.pump(cx);
                    if id == "notifications" {
                        view.preview_environment("common", window, cx);
                        view.functions.open = false;
                        view.apps.caps.as_mut().unwrap().notifications = true;
                        view.notifications.open = true;
                        view.apply_app_event(
                            rv_session::AppEvent::Notifications(rv_session::NotificationSnapshot {
                                revision: "preview".into(),
                                notifications: vec![rv_session::PhoneNotification {
                                    id: "1".into(),
                                    source: "com.apple.Preferences".into(),
                                    title: "TrollVNC".into(),
                                    subtitle: "".into(),
                                    message: "A VNC client connected.".into(),
                                    timestamp: 1791600000,
                                }],
                            }),
                            cx,
                        );
                    } else if id == "email-loading" {
                        view.preview_function("email-add", window, cx);
                        view.apply_function_event(
                            rv_session::functions::FunctionEvent::Progress("正在验证邮箱…".into()),
                            cx,
                        );
                        view.apply_function_event(
                            rv_session::functions::FunctionEvent::VerificationCodeReceived(
                                "012345".into(),
                            ),
                            cx,
                        );
                    } else if let Some(state) = id.strip_prefix("env-") {
                        view.preview_environment(state, window, cx);
                    } else {
                        view.preview_function(id, window, cx);
                    }
                });
                cx.new(|cx| Root::new(cx.new(|cx| super::SessionShell::new(view, cx)), window, cx))
            })
            .unwrap();
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(10));
            cx.run_until_parked();
            cx.update_window(handle.into(), |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
        }
        if id == "authorization-confirmation" {
            cx.update_window(handle.into(), |_, window, cx| {
                preview_view.as_ref().unwrap().update(cx, |view, cx| {
                    view.preview_function_confirmation(window, cx)
                });
                window.draw(cx).clear(cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle.into(), |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
        }
        let destination = if name.starts_with("apple-ui-") {
            let directory = output.parent().unwrap().join("tempdir/apple-ui");
            std::fs::create_dir_all(&directory).unwrap();
            directory.join(name)
        } else {
            output.join(name)
        };
        cx.capture_screenshot(handle.into())
            .unwrap()
            .save(destination)
            .unwrap();
    }
}
