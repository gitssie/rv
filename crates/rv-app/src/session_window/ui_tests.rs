use super::{Phase, SessionOptions, SessionView, password_error};
use crate::actions::*;
use gpui::{
    AppContext, Bounds, ClipboardItem, Entity, Focusable, Modifiers, MouseButton, MouseExitEvent,
    Pixels, TestAppContext, VisualTestContext, point, px, size,
};
use rv_core::{ClipboardMode, ConnectRequest, LocalCursorMode, ScaleMode};
use rv_session::{SessionCommand, SessionEvent, SessionHandle, SessionTestPeer};

pub(super) fn setup(
    cx: &mut TestAppContext,
    view_only: bool,
) -> (Entity<SessionView>, &mut VisualTestContext, SessionTestPeer) {
    setup_with_cursor_mode(cx, view_only, LocalCursorMode::Automatic)
}

fn setup_with_cursor_mode(
    cx: &mut TestAppContext,
    view_only: bool,
    local_cursor: LocalCursorMode,
) -> (Entity<SessionView>, &mut VisualTestContext, SessionTestPeer) {
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::bind_keys(cx);
    });
    let mut connection = rv_core::Connection::new("Offscreen desktop", "example.test", 5900);
    connection.view_only = view_only;
    connection.local_cursor = local_cursor;
    let request = ConnectRequest::from_connection(&connection, None);
    let (handle, peer) = SessionHandle::test_pair();
    handle.framebuffer.lock().unwrap().resize(800, 400);
    peer.events
        .send(SessionEvent::Connected {
            width: 800,
            height: 400,
        })
        .unwrap();
    peer.events
        .send(SessionEvent::FrameReady { generation: 1 })
        .unwrap();
    let mut entity = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        crate::theme::apply(rv_core::ThemePref::Dark, window, cx);
        let view = cx.new(|cx| {
            let mut view = SessionView::with_handle(
                request,
                "Offscreen desktop".into(),
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
                handle,
                window,
                cx,
            );
            view.pump(cx);
            view
        });
        entity = Some(view.clone());
        gpui_component::Root::new(cx.new(|cx| super::SessionShell::new(view, cx)), window, cx)
    });
    cx.simulate_resize(size(px(1000.), px(800.)));
    cx.update(|window, cx| {
        window.activate_window();
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
    (entity.unwrap(), cx, peer)
}

fn picture(view: &Entity<SessionView>, cx: &VisualTestContext) -> Bounds<Pixels> {
    view.read_with(cx, |view, _| view.image_box.get())
}

fn hidden(view: &Entity<SessionView>, cx: &VisualTestContext) -> bool {
    view.read_with(cx, |view, _| view.local_cursor.is_hidden())
}

fn commands(peer: &mut SessionTestPeer) -> Vec<SessionCommand> {
    let mut commands = vec![];
    while let Ok(command) = peer.commands.try_recv() {
        commands.push(command);
    }
    commands
}

#[test]
fn password_failures_offer_credential_retry() {
    assert!(password_error(
        "VNC error: VNC Error with message: password check failed!"
    ));
    assert!(password_error("VNC error: Wrong password"));
    assert!(password_error("Mac login was rejected"));
    assert!(!password_error("connection timed out"));
}

#[gpui::test]
fn offscreen_password_failure_requires_input_then_retries(cx: &mut TestAppContext) {
    let (view, cx, _peer) = setup(cx, false);
    view.update(cx, |view, cx| {
        view.phase = Phase::Ended;
        view.error = Some("password check failed!".into());
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| view.retry_with_password(window, cx));
    });
    view.read_with(cx, |view, _| {
        assert_eq!(view.phase, Phase::Ended);
        assert_eq!(
            view.password_validation.as_deref(),
            Some("Enter a password to reconnect.")
        );
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.password_input
                .update(cx, |input, cx| input.set_value("replacement", window, cx));
            view.retry_with_password(window, cx);
        });
    });
    view.read_with(cx, |view, _| {
        assert_eq!(view.phase, Phase::Connecting);
        assert_eq!(view.req.password.as_deref(), Some("replacement"));
        assert!(view.retried_password);
    });
}

#[gpui::test]
fn offscreen_password_failure_accepts_typed_password(cx: &mut TestAppContext) {
    let (view, cx, mut peer) = setup(cx, false);
    view.update(cx, |view, cx| {
        view.phase = Phase::Ended;
        view.error = Some("password check failed!".into());
        cx.notify();
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        view.read(cx)
            .password_input
            .focus_handle(cx)
            .focus(window, cx);
    });
    cx.simulate_input("replacement");
    view.read_with(cx, |view, cx| {
        assert_eq!(view.password_input.read(cx).unmask_value(), "replacement");
    });
    assert!(commands(&mut peer).is_empty());
    cx.simulate_keystrokes("enter");
    view.read_with(cx, |view, _| {
        assert_eq!(view.phase, Phase::Connecting);
        assert_eq!(view.req.password.as_deref(), Some("replacement"));
    });
    cx.update(|window, cx| {
        assert!(view.read(cx).focus.is_focused(window));
    });
}

#[gpui::test]
fn offscreen_file_transfer_opens_once_per_session(cx: &mut TestAppContext) {
    let (view, cx, _peer) = setup(cx, false);
    view.update(cx, |view, cx| {
        for _ in 0..2 {
            view.open_file_transfer(cx);
        }
    });
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 2);
    let first = view.read_with(cx, |view, _| view.file_transfer_window.unwrap());

    view.update(cx, |view, cx| view.open_file_transfer(cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 2);
    assert_eq!(
        view.read_with(cx, |view, _| view.file_transfer_window.unwrap()),
        first
    );

    cx.update(|_, cx| {
        first
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
    });
    view.update(cx, |view, cx| view.open_file_transfer(cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| cx.windows().len()), 2);
    assert_ne!(
        view.read_with(cx, |view, _| view.file_transfer_window.unwrap()),
        first
    );
}

#[gpui::test]
fn offscreen_pointer_maps_to_framebuffer_releases_buttons_and_keeps_cursor_visible(
    cx: &mut TestAppContext,
) {
    let (view, cx, mut peer) = setup(cx, false);
    let center = picture(&view, cx).center();
    cx.simulate_mouse_move(center, None, Modifiers::default());
    cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
    let pointers: Vec<_> = commands(&mut peer)
        .into_iter()
        .map(|command| {
            let SessionCommand::Input(vnc::X11Event::PointerEvent(event)) = command else {
                panic!("unexpected command: {command:?}");
            };
            (event.position_x, event.position_y, event.bottons)
        })
        .collect();
    assert_eq!(pointers, [(400, 200, 0), (400, 200, 1), (400, 200, 0)]);
    view.read_with(cx, |view, _| {
        assert_eq!(view.map_pointer(center), Some((400, 200)));
        assert_eq!(view.buttons, 0);
    });
    assert!(!hidden(&view, cx));
}

#[gpui::test]
fn offscreen_letterbox_and_local_menu_keep_cursor_visible(cx: &mut TestAppContext) {
    let (view, cx, mut peer) = setup(cx, false);
    let bounds = picture(&view, cx);
    cx.simulate_mouse_move(bounds.center(), None, Modifiers::default());
    assert!(!hidden(&view, cx));
    commands(&mut peer);
    cx.simulate_mouse_move(
        bounds.origin + point(px(2.), px(2.)),
        None,
        Modifiers::default(),
    );
    assert!(!hidden(&view, cx));
    assert!(commands(&mut peer).is_empty());
    cx.simulate_mouse_move(bounds.center(), None, Modifiers::default());
    cx.simulate_keystrokes("f8");
    view.read_with(cx, |view, _| assert!(view.show_menu));
    assert!(!hidden(&view, cx));
}

#[gpui::test]
fn offscreen_explicit_show_cursor_mode_keeps_cursor_visible(cx: &mut TestAppContext) {
    let (view, cx, _peer) = setup_with_cursor_mode(cx, false, LocalCursorMode::Show);
    cx.simulate_mouse_move(picture(&view, cx).center(), None, Modifiers::default());
    assert!(!hidden(&view, cx));
}

#[gpui::test]
fn offscreen_hide_cursor_mode_only_hides_over_remote_picture(cx: &mut TestAppContext) {
    let (view, cx, _peer) = setup_with_cursor_mode(cx, false, LocalCursorMode::Hide);
    let bounds = picture(&view, cx);
    cx.simulate_mouse_move(bounds.center(), None, Modifiers::default());
    assert!(hidden(&view, cx));
    cx.simulate_mouse_move(
        bounds.origin + point(px(2.), px(2.)),
        None,
        Modifiers::default(),
    );
    assert!(!hidden(&view, cx));
    cx.simulate_mouse_move(bounds.center(), None, Modifiers::default());
    assert!(hidden(&view, cx));
    cx.simulate_keystrokes("f8");
    assert!(!hidden(&view, cx));
}

#[gpui::test]
fn offscreen_cursor_stays_visible_across_mouse_exit_focus_loss_and_frame_redraw(
    cx: &mut TestAppContext,
) {
    let (view, cx, peer) = setup_with_cursor_mode(cx, false, LocalCursorMode::Hide);
    let center = picture(&view, cx).center();
    cx.simulate_mouse_move(center, None, Modifiers::default());
    assert!(hidden(&view, cx));
    cx.simulate_event(MouseExitEvent {
        position: center,
        pressed_button: None,
        modifiers: Modifiers::default(),
    });
    assert!(!hidden(&view, cx));
    peer.events
        .send(SessionEvent::FrameReady { generation: 2 })
        .unwrap();
    view.update(cx, |view, cx| view.pump(cx));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    assert!(!hidden(&view, cx));
    cx.simulate_mouse_move(center, None, Modifiers::default());
    assert!(hidden(&view, cx));
    cx.deactivate_window();
    assert!(!hidden(&view, cx));
}

#[gpui::test]
fn offscreen_view_only_blocks_remote_input_and_keeps_cursor(cx: &mut TestAppContext) {
    let (view, cx, mut peer) = setup(cx, true);
    let center = picture(&view, cx).center();
    cx.simulate_mouse_move(center, None, Modifiers::default());
    cx.simulate_click(center, Modifiers::default());
    cx.simulate_keystrokes("a");
    assert!(commands(&mut peer).is_empty());
    assert!(!hidden(&view, cx));
}

#[gpui::test]
fn offscreen_disconnect_keeps_cursor_visible_and_stops_pointer_input(cx: &mut TestAppContext) {
    let (view, cx, mut peer) = setup_with_cursor_mode(cx, false, LocalCursorMode::Hide);
    let center = picture(&view, cx).center();
    cx.simulate_mouse_move(center, None, Modifiers::default());
    assert!(hidden(&view, cx));
    commands(&mut peer);
    cx.dispatch_action(SessionDisconnect);
    assert!(!hidden(&view, cx));
    assert!(matches!(
        peer.commands.try_recv(),
        Ok(SessionCommand::Close)
    ));
    view.read_with(cx, |view, _| assert_eq!(view.phase, Phase::Disconnecting));
    cx.simulate_mouse_move(center, None, Modifiers::default());
    assert!(!hidden(&view, cx));
    assert!(commands(&mut peer).is_empty());
    peer.events.send(SessionEvent::Disconnected).unwrap();
    view.update(cx, |view, cx| view.pump(cx));
    cx.run_until_parked();
    cx.simulate_mouse_move(center, None, Modifiers::default());
    assert!(!hidden(&view, cx));
    assert!(commands(&mut peer).is_empty());
    view.read_with(cx, |view, _| assert_eq!(view.phase, Phase::Ended));
}

#[gpui::test]
fn offscreen_late_connected_event_cannot_cancel_pending_disconnect(cx: &mut TestAppContext) {
    let (view, cx, mut peer) = setup_with_cursor_mode(cx, false, LocalCursorMode::Hide);
    let center = picture(&view, cx).center();
    commands(&mut peer);
    view.update(cx, |view, _| view.phase = Phase::Connecting);
    cx.dispatch_action(SessionDisconnect);
    assert!(matches!(
        peer.commands.try_recv(),
        Ok(SessionCommand::Close)
    ));
    peer.events
        .send(SessionEvent::Connected {
            width: 800,
            height: 400,
        })
        .unwrap();
    view.update(cx, |view, cx| view.pump(cx));
    cx.simulate_mouse_move(center, None, Modifiers::default());
    assert!(!hidden(&view, cx));
    assert!(commands(&mut peer).is_empty());
    view.read_with(cx, |view, _| assert_eq!(view.phase, Phase::Disconnecting));
}

#[gpui::test]
fn offscreen_close_keeps_cursor_visible_and_closes_session(cx: &mut TestAppContext) {
    let (view, cx, mut peer) = setup_with_cursor_mode(cx, false, LocalCursorMode::Hide);
    cx.simulate_mouse_move(picture(&view, cx).center(), None, Modifiers::default());
    assert!(hidden(&view, cx));
    commands(&mut peer);
    cx.dispatch_action(SessionClose);
    assert!(!hidden(&view, cx));
    assert!(matches!(
        peer.commands.try_recv(),
        Ok(SessionCommand::Close)
    ));
}

#[gpui::test]
fn offscreen_keyboard_forwards_pairs_and_consumes_menu_shortcuts(cx: &mut TestAppContext) {
    let (view, cx, mut peer) = setup(cx, false);
    for key in ["a", "f8", "escape"] {
        cx.simulate_keystrokes(key);
        // GPUI's typing helper sends key-down only; release explicitly to
        // verify the remote cannot be left with a stuck key.
        cx.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse(key).unwrap(),
        });
    }
    let keys: Vec<_> = commands(&mut peer)
        .into_iter()
        .map(|command| {
            let SessionCommand::Input(vnc::X11Event::KeyEvent(event)) = command else {
                panic!("unexpected command: {command:?}");
            };
            (event.keycode, event.down)
        })
        .collect();
    assert_eq!(keys, [(u32::from(b'a'), true), (u32::from(b'a'), false)]);
    view.read_with(cx, |view, _| assert!(!view.show_menu));
}

#[gpui::test]
fn offscreen_trollvnc_paste_waits_for_ack_and_does_not_repeat(cx: &mut TestAppContext) {
    let (view, cx, mut peer) = setup(cx, false);
    peer.events.send(SessionEvent::Apps(rv_session::AppEvent::Capabilities(
        rv_session::AppCapabilities { control: true, clipboard_write: true, ..Default::default() }
    ))).unwrap();
    view.update(cx, |view, cx| view.pump(cx));
    commands(&mut peer);
    cx.write_to_clipboard(ClipboardItem::new_string("中文输入".into()));
    for is_held in [false, false, true] {
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: gpui::Keystroke::parse("cmd-v").unwrap(), is_held, prefer_character_input: false,
        });
    }
    cx.simulate_event(gpui::KeyUpEvent { keystroke: gpui::Keystroke::parse("v").unwrap() });
    let sent = commands(&mut peer);
    let writes: Vec<_> = sent.iter().filter_map(|command| match command {
        SessionCommand::App(command @ rv_session::AppCommand::WriteClipboard { .. }) => Some(command.clone()),
        _ => None,
    }).collect();
    assert_eq!(writes.len(), 1);
    assert!(!sent.iter().any(|command| matches!(command,
        SessionCommand::Input(vnc::X11Event::KeyEvent(event)) if event.keycode == u32::from(b'v'))));
    view.update(cx, |view, cx| view.apply_app_event(rv_session::AppEvent::Finished(writes[0].clone()), cx));
    cx.update(|window, cx| { view.update(cx, |view, cx| view.finish_keyboard_paste(window, cx)); });
    let sent = commands(&mut peer);
    assert_eq!(sent.iter().filter(|command| matches!(command,
        SessionCommand::Input(vnc::X11Event::KeyEvent(event)) if event.keycode == u32::from(b'v'))).count(), 2);
    assert!(cx.read_from_clipboard().is_none_or(|item| item.text().is_none_or(|text| text.is_empty())));
    cx.simulate_keystrokes("cmd-v");
    assert!(!commands(&mut peer).iter().any(|command| matches!(command,
        SessionCommand::App(rv_session::AppCommand::WriteClipboard { .. })) || matches!(command,
        SessionCommand::Input(vnc::X11Event::KeyEvent(event)) if event.keycode == u32::from(b'v'))));
}

#[gpui::test]
fn offscreen_trollvnc_paste_without_capability_does_not_paste_old_phone_text(cx: &mut TestAppContext) {
    let (view, cx, mut peer) = setup(cx, false);
    peer.events.send(SessionEvent::Apps(rv_session::AppEvent::Capabilities(
        rv_session::AppCapabilities::default()
    ))).unwrap();
    view.update(cx, |view, cx| view.pump(cx));
    commands(&mut peer);
    for key in ["ctrl-v", "cmd-v"] {
        cx.simulate_keystrokes(key);
        cx.simulate_event(gpui::KeyUpEvent { keystroke: gpui::Keystroke::parse("v").unwrap() });
        assert!(!commands(&mut peer).iter().any(|command| matches!(command,
            SessionCommand::App(rv_session::AppCommand::WriteClipboard { .. })) || matches!(command,
            SessionCommand::Input(vnc::X11Event::KeyEvent(event)) if event.keycode == u32::from(b'v'))));
    }
}

#[gpui::test]
fn offscreen_trollvnc_paste_respects_view_only(cx: &mut TestAppContext) {
    let (view, cx, mut peer) = setup(cx, true);
    peer.events
        .send(SessionEvent::Apps(rv_session::AppEvent::Capabilities(
            rv_session::AppCapabilities::default(),
        )))
        .unwrap();
    view.update(cx, |view, cx| view.pump(cx));
    commands(&mut peer);
    cx.simulate_keystrokes("ctrl-v");
    cx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("v").unwrap(),
    });
    assert!(commands(&mut peer).is_empty());
}

#[gpui::test]
fn offscreen_paste_keeps_plain_vnc_and_mac_shortcuts(cx: &mut TestAppContext) {
    use rv_core::{XK_CONTROL_L, XK_SUPER_L};
    let (view, cx, mut peer) = setup(cx, false);
    for (trollvnc, shortcut, modifier) in [
        (false, "ctrl-v", XK_CONTROL_L),
        (false, "cmd-v", XK_SUPER_L),
        (true, "ctrl-shift-v", XK_CONTROL_L),
    ] {
        if trollvnc {
            peer.events
                .send(SessionEvent::Apps(rv_session::AppEvent::Capabilities(
                    rv_session::AppCapabilities::default(),
                )))
                .unwrap();
            view.update(cx, |view, cx| view.pump(cx));
        }
        commands(&mut peer);
        cx.simulate_keystrokes(shortcut);
        cx.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse("v").unwrap(),
        });
        let keys: Vec<_> = commands(&mut peer)
            .into_iter()
            .map(|command| {
                let SessionCommand::Input(vnc::X11Event::KeyEvent(event)) = command else {
                    panic!("unexpected command: {command:?}");
                };
                (event.keycode, event.down)
            })
            .collect();
        assert_eq!(keys.len(), 4, "{shortcut}");
        assert_eq!(keys[0], (modifier, true), "{shortcut}");
        assert!(keys[1].1);
        assert_eq!(keys[2], (keys[1].0, false));
        assert_eq!(keys[3], (modifier, false));
        view.read_with(cx, |view, _| assert!(!view.command_paste_pressed));
    }
}

#[gpui::test]
fn offscreen_clipboard_sends_utf8_text_to_remote(cx: &mut TestAppContext) {
    let (view, cx, mut peer) = setup(cx, false);
    commands(&mut peer);
    cx.write_to_clipboard(ClipboardItem::new_string("中文剪贴板".into()));
    view.update(cx, |view, cx| view.send_clipboard(cx));
    assert!(matches!(
        peer.commands.try_recv(),
        Ok(SessionCommand::App(rv_session::AppCommand::WriteClipboard { text, .. }))
            if text == "中文剪贴板"
    ));
    view.read_with(cx, |view, _| {
        assert_eq!(view.status.as_ref(), "正在发送剪贴板")
    });
}

#[gpui::test]
fn offscreen_zxtouch_clipboard_ignores_vnc_text_encoding(cx: &mut TestAppContext) {
    let (view, cx, mut peer) = setup(cx, false);
    commands(&mut peer);
    view.update(cx, |view, _| view.req.clipboard = ClipboardMode::Latin1);
    cx.write_to_clipboard(ClipboardItem::new_string("中文剪贴板".into()));
    view.update(cx, |view, cx| view.send_clipboard(cx));
    assert!(
        matches!(peer.commands.try_recv(), Ok(SessionCommand::App(rv_session::AppCommand::WriteClipboard { text, .. })) if text == "中文剪贴板")
    );
}

#[gpui::test]
fn offscreen_notification_panel_keeps_keyboard_local(cx: &mut TestAppContext) {
    let (view, cx, mut peer) = setup(cx, false);
    commands(&mut peer);
    view.update(cx, |view, cx| {
        view.notifications.open = true;
        cx.notify();
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("a").unwrap(),
    });
    assert!(commands(&mut peer).is_empty());
    cx.simulate_keystrokes("escape");
    view.read_with(cx, |view, _| assert!(!view.notifications.open));
}

#[gpui::test]
fn assistant_uses_fit_for_both_picture_and_pointer_then_restores_actual(cx: &mut TestAppContext) {
    let (view, cx, _) = setup(cx, false);
    view.update(cx, |v, cx| {
        v.scale = ScaleMode::Actual;
        v.assistant_open = true;
        cx.notify();
    });
    cx.update(|w, cx| w.draw(cx).clear(cx));
    let bounds = picture(&view, cx);
    view.read_with(cx, |v, _| {
        assert_eq!(v.effective_scale(), ScaleMode::Fit);
        assert_eq!(v.map_pointer(bounds.center()), Some((400, 200)));
    });
    view.update(cx, |v, _| v.assistant_open = false);
    view.read_with(cx, |v, _| {
        assert_eq!(v.effective_scale(), ScaleMode::Actual)
    });
}

#[gpui::test]
fn assistant_opens_with_notifications_received_before_it_existed(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (view, cx, peer) = setup(cx, false);
    let snapshot = rv_session::NotificationSnapshot {
        revision: "cached-1".into(),
        notifications: vec![rv_session::PhoneNotification {
            id: "n1".into(),
            source: "vinted".into(),
            title: "Message".into(),
            subtitle: String::new(),
            message: "Hello".into(),
            timestamp: 1,
        }],
    };
    peer.events
        .send(SessionEvent::Apps(rv_session::AppEvent::Notifications(
            snapshot.clone(),
        )))
        .unwrap();
    cx.update(|w, cx| {
        view.update(cx, |v, cx| {
            v.pump(cx);
            assert!(v.assistant.is_none());
            v.thumb_path = Some(dir.path().join("thumbs/fixture.png"));
            v.toggle_assistant(w, cx);
            assert_eq!(
                v.assistant
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .notification_snapshot(),
                Some(&snapshot)
            );
        })
    });
}

#[gpui::test]
fn offscreen_clipboard_displays_control_failure(cx: &mut TestAppContext) {
    let (view, cx, _) = setup(cx, false);
    view.update(cx, |view, cx| {
        view.apply_app_event(
            rv_session::AppEvent::Failed {
                command: rv_session::AppCommand::WriteClipboard {
                    token: uuid::Uuid::new_v4().to_string(),
                    text: "test".into(),
                },
                message: "ZXTouch unavailable".into(),
            },
            cx,
        )
    });
    view.read_with(cx, |view, _| {
        assert!(view.status.contains("ZXTouch unavailable"))
    });
}

#[gpui::test]
fn retry_exhaustion_keeps_the_same_compact_dialog(cx: &mut TestAppContext) {
    let (view, cx, _) = setup(cx, false);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.phase = Phase::Ended;
            view.recovery.connected();
            view.recovery.schedule(
                Some("server closed the connection"),
                std::time::Instant::now(),
            );
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    let before = cx.debug_bounds("recovery-dialog").unwrap();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            for _ in 0..5 {
                view.recovery.schedule(
                    Some("server closed the connection"),
                    std::time::Instant::now(),
                );
            }
            assert!(view.recovery.deadline.is_none());
            cx.notify();
        });
        window.draw(cx).clear(cx);
    });
    let after = cx.debug_bounds("recovery-dialog").unwrap();
    assert_eq!(before.size.width, after.size.width);
    assert!(cx.debug_bounds("session-recovery").is_some());
}

#[gpui::test]
fn control_toolbar_keeps_primary_actions_reachable_at_640px(cx: &mut TestAppContext) {
    let (_, cx, mut peer) = setup(cx, false);
    cx.simulate_resize(size(px(640.), px(800.)));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let toolbar = cx.debug_bounds("session-toolbar").unwrap();
    assert_eq!(toolbar.top(), crate::theme::toolbar_height());
    assert_eq!(toolbar.size.height, crate::theme::toolbar_height());
    assert_eq!(cx.debug_bounds("session-stage").unwrap().top(), crate::theme::toolbar_height() * 2.);
    let settings = cx.debug_bounds("session-settings").unwrap();
    assert!(settings.bottom() <= toolbar.top());
    let notifications = cx.debug_bounds("phone-notifications").unwrap();
    assert!(notifications.bottom() <= toolbar.top());
    assert!(notifications.right() <= settings.left());
    let files = cx.debug_bounds("files").unwrap();
    assert!(files.bottom() <= toolbar.top());
    assert!(files.right() <= notifications.left());
    assert!(cx.debug_bounds("disconnect-control").is_none());
    let functions = cx.debug_bounds("function-menu-toggle").unwrap();
    assert!(functions.left() > cx.debug_bounds("app-plus").unwrap().right());
    let clipboard = cx.debug_bounds("clip").unwrap();
    let app_plus = cx.debug_bounds("app-plus").unwrap();
    assert!(clipboard.left() >= app_plus.right());
    assert!(clipboard.left() - app_plus.right() < px(12.));
    let keyboard = cx.debug_bounds("keyboard-menu").unwrap();
    assert!(keyboard.bottom() <= toolbar.top());
    assert!(keyboard.right() <= files.left());
    for selector in ["function-menu-toggle", "clip"] {
        let bounds = cx.debug_bounds(selector).unwrap();
        assert!(bounds.left() >= toolbar.left() && bounds.right() <= toolbar.right(), "{selector}: {bounds:?}");
    }
    while peer.commands.try_recv().is_ok() {}
    let more = cx.debug_bounds("session-settings").unwrap();
    cx.simulate_click(more.center(), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("session-menu").is_some());
    assert!(peer.commands.try_recv().is_err());
    cx.simulate_keystrokes("escape");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("session-menu").is_none());
    assert!(peer.commands.try_recv().is_err());
}

#[gpui::test]
fn keyboard_menu_dismiss_does_not_send_remote_keys(cx: &mut TestAppContext) {
    let (_, cx, mut peer) = setup(cx, false);
    cx.update(|window, cx| {
        crate::theme::apply(rv_core::ThemePref::Light, window, cx);
        window.draw(cx).clear(cx);
    });
    let _ = commands(&mut peer);
    let trigger = cx.debug_bounds("keyboard-menu").unwrap();
    cx.simulate_click(trigger.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(commands(&mut peer).is_empty());
    cx.simulate_keystrokes("escape");
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(commands(&mut peer).is_empty());
}
