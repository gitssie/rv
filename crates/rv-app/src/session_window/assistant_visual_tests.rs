//! Real native rendering with synthetic framebuffer and isolated local data.
use super::{SessionOptions, SessionView};
use gpui::{AppContext, HeadlessAppContext, px, size};
use gpui_component::Root;
use rv_core::assistant::{AssistantService, Draft};
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
    service.call(|s| s.save_draft(&Draft {title:"Vintage leather shoulder bag".into(), description:"Brown leather shoulder bag. Good condition, with light signs of use. Please see all photos.".into(),price_minor:Some(1850),stock_note:"A-023".into(),..Default::default()})).unwrap();
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
    for (name, appearance) in [
        ("assistant-implemented-dark.png", ThemePref::Dark),
        ("assistant-implemented-light.png", ThemePref::Light),
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
            .open_window(size(px(1160.), px(900.)), |window, cx| {
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
        assert!(cx.update(|cx| view.as_ref().unwrap().read(cx).assistant.is_some()));
        cx.capture_screenshot(handle.into())
            .unwrap()
            .save(output.join(name))
            .unwrap();
    }
}
