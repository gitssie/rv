//! Real native rendering with synthetic framebuffer and isolated local data.
use super::{SessionOptions, SessionView};
use gpui::{AppContext, HeadlessAppContext, px, size};
use gpui_component::Root;
use rv_core::assistant::{AssistantService, Draft, Template};
use rv_core::{ConnectRequest, Connection, ScaleMode, ThemePref};
use rv_session::{SessionEvent, SessionHandle};
use std::{sync::Arc, time::Duration};
#[test]
#[ignore = "manual macOS Metal screenshot export"]
fn export_assistant_previews() {
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
    std::fs::create_dir_all(dir.path().join("thumbs")).unwrap();
    let service = AssistantService::shared(dir.path().to_owned());
    let draft = service.call(|s| s.save_draft(&Draft {title:"Blue knit cardigan - Size M".into(), description:"Size M. Excellent condition. Soft blue knit cardigan, worn twice. Measurements available on request.".into(),price_minor:Some(1800),stock_note:"A-023".into(),..Default::default()})).unwrap();
    for index in 0..4 {
        let path = dir.path().join(format!("cardigan-{index}.png"));
        image::RgbaImage::from_fn(120, 120, |x, y| {
            let garment = (40..80).contains(&x) && (25..106).contains(&y)
                || (20..40).contains(&x) && (30..65).contains(&y)
                || (80..100).contains(&x) && (30..65).contains(&y);
            if garment {
                image::Rgba([65 + index * 8, 116 + index * 5, 162 + index * 6, 255])
            } else {
                image::Rgba([225, 228, 231, 255])
            }
        })
        .save(&path)
        .unwrap();
        let id = draft.id.clone();
        service.call(move |s| s.add_image(&id, &path)).unwrap();
    }
    service.call(|s| s.save_template(&Template { name:"Item description".into(), category:"Listing".into(), content:"Excellent condition. Carefully stored in a smoke-free home. Happy to provide measurements.".into(), ..Default::default() })).unwrap();
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
    for (name, appearance, tab, width, height) in [
        (
            "assistant-implemented-dark.png",
            ThemePref::Dark,
            0,
            1160.,
            780.,
        ),
        (
            "assistant-implemented-light.png",
            ThemePref::Light,
            0,
            1160.,
            780.,
        ),
        (
            "assistant-templates-dark.png",
            ThemePref::Dark,
            1,
            1160.,
            780.,
        ),
        (
            "assistant-templates-light.png",
            ThemePref::Light,
            1,
            1160.,
            780.,
        ),
        (
            "assistant-notifications-dark.png",
            ThemePref::Dark,
            2,
            1160.,
            780.,
        ),
        (
            "assistant-notifications-light.png",
            ThemePref::Light,
            2,
            1160.,
            780.,
        ),
        (
            "assistant-implemented-compact.png",
            ThemePref::Dark,
            0,
            840.,
            650.,
        ),
    ] {
        let (session, peer) = SessionHandle::test_pair();
        {
            let mut fb = session.framebuffer.lock().unwrap();
            fb.resize(390, 844);
            for (i, pixel) in fb.pixels.chunks_exact_mut(4).enumerate() {
                let x = i % 390;
                let y = i / 390;
                let value = if (x / 65 + y / 65) % 2 == 0 { 34 } else { 46 };
                pixel.copy_from_slice(&[value + 25, value + 12, value, 255]);
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
        let connection = Connection::new("Vinted iPhone", "fixture.test", 5900);
        let request = ConnectRequest::from_connection(&connection, None);
        let mut view = None;
        let handle = cx
            .open_window(size(px(width), px(height)), |window, cx| {
                crate::theme::apply(appearance, window, cx);
                let entity = cx.new(|cx| {
                    SessionView::with_handle(
                        request,
                        "Vinted iPhone".into(),
                        SessionOptions {
                            scale: ScaleMode::Actual,
                            pin_toolbar: true,
                            menu_key: "f8".into(),
                            hide_shots: true,
                            thumb_path: Some(dir.path().join("thumbs/fixture.png")),
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
                entity.update(cx, |v, cx| {
                    v.pump(cx);
                    v.toggle_assistant(window, cx);
                });
                view = Some(entity.clone());
                cx.new(|cx| Root::new(entity, window, cx))
            })
            .unwrap();
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(10));
            cx.run_until_parked();
            cx.update_window(handle.into(), |_, w, cx| w.draw(cx).clear(cx))
                .unwrap();
        }
        cx.update_window(handle.into(), |_, window, cx| {
            view.as_ref().unwrap().update(cx, |v, cx| {
                let assistant = v.assistant.as_ref().unwrap();
                assistant.update(cx, |a, cx| {
                    a.set_connection(true, true, false, true, true, cx);
                    a.receive_notifications(
                        rv_session::NotificationSnapshot {
                            revision: "preview".into(),
                            notifications: vec![rv_session::PhoneNotification {
                                id: "preview-1".into(),
                                source: "Vinted".into(),
                                title: "New message about your item".into(),
                                subtitle: "Blue knit cardigan".into(),
                                message: "Hi! Could you share the measurements?".into(),
                                timestamp: 1791205200,
                            }],
                        },
                        cx,
                    );
                    a.preview_tab(tab, window, cx);
                });
            });
        })
        .unwrap();
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(10));
            cx.run_until_parked();
            cx.update_window(handle.into(), |_, w, cx| w.draw(cx).clear(cx))
                .unwrap();
        }
        assert!(cx.update(|cx| view.as_ref().unwrap().read(cx).assistant.is_some()));
        cx.capture_screenshot(handle.into())
            .unwrap()
            .save(output.join(name))
            .unwrap();
    }
}
