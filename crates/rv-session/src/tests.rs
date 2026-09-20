#[path = "../examples/support/ard.rs"]
mod ard_server;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use rv_core::{ClipboardMode, ConnectRequest, EncryptionMode, QualityPreset};

use crate::{SessionEvent, SessionHandle, coalesce_frames};

fn write_u16(s: &mut TcpStream, v: u16) {
    s.write_all(&v.to_be_bytes()).unwrap();
}

fn write_u32(s: &mut TcpStream, v: u32) {
    s.write_all(&v.to_be_bytes()).unwrap();
}

fn read_exact(s: &mut TcpStream, n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    s.read_exact(&mut buf).unwrap();
    buf
}

const EXTENDED_CLIPBOARD_ENCODING: u32 = 0xC0A1_E5CE;
const CLIPBOARD_TEXT: u32 = 1;
const CLIPBOARD_CAPS: u32 = 1 << 24;
const CLIPBOARD_REQUEST: u32 = 1 << 25;
const CLIPBOARD_PEEK: u32 = 1 << 26;
const CLIPBOARD_NOTIFY: u32 = 1 << 27;
const CLIPBOARD_PROVIDE: u32 = 1 << 28;

fn write_extended_clipboard(sock: &mut TcpStream, flags: u32, payload: &[u8]) {
    let length = i32::try_from(4 + payload.len()).unwrap();
    sock.write_all(&[3, 0, 0, 0]).unwrap();
    sock.write_all(&(-length).to_be_bytes()).unwrap();
    sock.write_all(&flags.to_be_bytes()).unwrap();
    sock.write_all(payload).unwrap();
}

fn extended_clipboard_provide(text: &str) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\n").replace(['\r', '\n'], "\r\n");
    let mut plain = Vec::new();
    plain.extend_from_slice(&((normalized.len() + 1) as u32).to_be_bytes());
    plain.extend_from_slice(normalized.as_bytes());
    plain.push(0);
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&plain).unwrap();
    encoder.finish().unwrap()
}

fn decode_extended_clipboard_text(payload: &[u8]) -> String {
    let mut decoder = ZlibDecoder::new(payload);
    let mut plain = Vec::new();
    decoder.read_to_end(&mut plain).unwrap();
    let length = u32::from_be_bytes(plain[..4].try_into().unwrap()) as usize;
    let text = &plain[4..4 + length - 1];
    String::from_utf8(text.to_vec())
        .unwrap()
        .replace("\r\n", "\n")
}

/// RFB 3.8 handshake: None auth, `width`×`height` framebuffer named `name`.
fn rfb_handshake(sock: &mut TcpStream, width: u16, height: u16, name: &[u8]) {
    sock.write_all(b"RFB 003.008\n").unwrap();
    let _ver = read_exact(sock, 12);
    sock.write_all(&[1, 1]).unwrap(); // one type: None
    let _choice = read_exact(sock, 1);
    write_u32(sock, 0); // SecurityResult OK
    rfb_server_init(sock, width, height, name);
}

fn rfb_server_init(sock: &mut TcpStream, width: u16, height: u16, name: &[u8]) {
    assert_eq!(read_exact(sock, 1), [1], "expected shared ClientInit");

    write_u16(sock, width);
    write_u16(sock, height);
    // Pixel format (ignored; client sends its own).
    sock.write_all(&[32, 24, 0, 1]).unwrap();
    sock.write_all(&255u16.to_be_bytes()).unwrap();
    sock.write_all(&255u16.to_be_bytes()).unwrap();
    sock.write_all(&255u16.to_be_bytes()).unwrap();
    sock.write_all(&[16, 8, 0, 0, 0, 0]).unwrap();
    write_u32(sock, name.len() as u32);
    sock.write_all(name).unwrap();
}

/// Minimal RFB 3.8 server: None auth, 8×8 Raw framebuffer, then echo input.
fn mock_rfb_server(mut sock: TcpStream) {
    rfb_handshake(&mut sock, 8, 8, b"test");
    mock_rfb_messages(sock);
}

fn mock_rfb_messages(mut sock: TcpStream) -> bool {
    let mut saw_pointer = false;
    let mut saw_key = false;
    let mut sent_frame = false;
    sock.set_read_timeout(Some(Duration::from_secs(3))).ok();

    loop {
        let mut typ = [0u8; 1];
        if sock.read_exact(&mut typ).is_err() {
            break;
        }
        match typ[0] {
            0 => {
                // SetPixelFormat
                let _ = read_exact(&mut sock, 3 + 16);
            }
            2 => {
                // SetEncodings
                let _ = read_exact(&mut sock, 1);
                let n = u16::from_be_bytes(read_exact(&mut sock, 2).try_into().unwrap());
                let _ = read_exact(&mut sock, n as usize * 4);
            }
            3 => {
                // FramebufferUpdateRequest
                let _ = read_exact(&mut sock, 9);
                if !sent_frame {
                    sock.write_all(&[0, 0]).unwrap(); // FramebufferUpdate + pad
                    write_u16(&mut sock, 1); // nrects
                    write_u16(&mut sock, 0);
                    write_u16(&mut sock, 0);
                    write_u16(&mut sock, 8);
                    write_u16(&mut sock, 8);
                    write_u32(&mut sock, 0); // Raw
                    let mut pixels = Vec::with_capacity(8 * 8 * 4);
                    for _ in 0..64 {
                        pixels.extend_from_slice(&[0, 80, 200, 255]); // BGRA
                    }
                    sock.write_all(&pixels).unwrap();
                    sent_frame = true;
                }
            }
            4 => {
                // KeyEvent
                let _ = read_exact(&mut sock, 7);
                saw_key = true;
            }
            5 => {
                // PointerEvent
                let _ = read_exact(&mut sock, 5);
                saw_pointer = true;
            }
            6 => {
                let _ = read_exact(&mut sock, 3);
                let len = u32::from_be_bytes(read_exact(&mut sock, 4).try_into().unwrap());
                let _ = read_exact(&mut sock, len as usize);
            }
            _ => break,
        }
        if sent_frame && saw_pointer && saw_key {
            break;
        }
    }
    sent_frame && saw_pointer && saw_key
}

fn extended_clipboard_server(
    mut sock: TcpStream,
    ready: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
    accepts_notify: bool,
) -> String {
    rfb_handshake(&mut sock, 8, 8, b"clipboard-test");
    sock.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut capabilities =
        CLIPBOARD_CAPS | CLIPBOARD_REQUEST | CLIPBOARD_PEEK | CLIPBOARD_PROVIDE | CLIPBOARD_TEXT;
    if accepts_notify {
        capabilities |= CLIPBOARD_NOTIFY;
    }
    let mut local_text = None;

    loop {
        let message_type = read_exact(&mut sock, 1)[0];
        match message_type {
            0 => {
                let _ = read_exact(&mut sock, 3 + 16);
            }
            2 => {
                let _ = read_exact(&mut sock, 1);
                let count = u16::from_be_bytes(read_exact(&mut sock, 2).try_into().unwrap());
                let encodings = read_exact(&mut sock, count as usize * 4);
                let advertised = encodings.chunks_exact(4).any(|encoding| {
                    u32::from_be_bytes(encoding.try_into().unwrap()) == EXTENDED_CLIPBOARD_ENCODING
                });
                assert!(advertised, "client did not advertise Extended Clipboard");
                let unsolicited_limit = if accepts_notify { 0 } else { 0x0010_0000_u32 };
                write_extended_clipboard(&mut sock, capabilities, &unsolicited_limit.to_be_bytes());
            }
            3 => {
                let _ = read_exact(&mut sock, 9);
            }
            4 => {
                let _ = read_exact(&mut sock, 7);
            }
            5 => {
                let _ = read_exact(&mut sock, 5);
            }
            6 => {
                let _ = read_exact(&mut sock, 3);
                let length = i32::from_be_bytes(read_exact(&mut sock, 4).try_into().unwrap());
                assert!(length < 0, "UTF-8 mode sent legacy ClientCutText");
                let data = read_exact(&mut sock, (-length) as usize);
                let flags = u32::from_be_bytes(data[..4].try_into().unwrap());
                let action = flags & 0xFF00_0000;
                if action & CLIPBOARD_CAPS != 0 {
                    ready.send(()).unwrap();
                    continue;
                }
                match action {
                    CLIPBOARD_NOTIFY => {
                        assert!(accepts_notify, "server did not advertise Notify support");
                        write_extended_clipboard(
                            &mut sock,
                            CLIPBOARD_REQUEST | CLIPBOARD_TEXT,
                            &[],
                        );
                    }
                    CLIPBOARD_PROVIDE => {
                        local_text = Some(decode_extended_clipboard_text(&data[4..]));
                        write_extended_clipboard(&mut sock, CLIPBOARD_NOTIFY | CLIPBOARD_TEXT, &[]);
                    }
                    CLIPBOARD_REQUEST => {
                        assert!(local_text.is_some());
                        let compressed = extended_clipboard_provide("远程剪贴板");
                        write_extended_clipboard(
                            &mut sock,
                            CLIPBOARD_PROVIDE | CLIPBOARD_TEXT,
                            &compressed,
                        );
                        let _ = release.recv_timeout(Duration::from_secs(5));
                        return local_text.unwrap();
                    }
                    action => panic!("unexpected Extended Clipboard action: {action:#x}"),
                }
            }
            message_type => panic!("unexpected client message: {message_type}"),
        }
    }
}

fn wait_for(
    handle: &SessionHandle,
    timeout: Duration,
    pred: impl Fn(&SessionEvent) -> bool,
) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        for ev in handle.drain() {
            if pred(&ev) {
                return true;
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
}

fn request_for(port: u16) -> ConnectRequest {
    ConnectRequest {
        connection_id: None,
        name: "mock".into(),
        host: "127.0.0.1".into(),
        port,
        username: None,
        password: None,
        encryption: EncryptionMode::Off,
        quality: QualityPreset::Fast,
        clipboard: ClipboardMode::Utf8,
        local_cursor: rv_core::LocalCursorMode::Automatic,
        view_only: false,
        shared: true,
    }
}

#[test]
fn handshake_frame_and_input() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (sock, _) = listener.accept().unwrap();
        mock_rfb_server(sock);
    });

    let handle = SessionHandle::spawn(request_for(addr.port()));

    assert!(
        wait_for(&handle, Duration::from_secs(5), |e| matches!(
            e,
            SessionEvent::Connected {
                width: 8,
                height: 8,
                ..
            } | SessionEvent::FrameReady { .. }
        )),
        "expected connected/frame event"
    );
    assert!(
        wait_for(&handle, Duration::from_secs(3), |e| {
            matches!(e, SessionEvent::FrameReady { .. })
        }) || handle.framebuffer.lock().unwrap().width == 8,
        "expected a framebuffer"
    );

    {
        let fb = handle.framebuffer.lock().unwrap();
        assert_eq!(fb.width, 8);
        assert_eq!(fb.height, 8);
        assert_eq!(&fb.pixels[0..4], &[0, 80, 200, 255]);
    }

    handle.pointer(1, 1, 1);
    handle.key(0xff0d, true);
    handle.key(0xff0d, false);

    thread::sleep(Duration::from_millis(200));
    handle.close();
    drop(handle);
    let _ = server.join();
}

fn assert_extended_clipboard_roundtrip(accepts_notify: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (sock, _) = listener.accept().unwrap();
        extended_clipboard_server(sock, ready_tx, release_rx, accepts_notify)
    });

    let handle = SessionHandle::spawn(request_for(addr.port()));
    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("client should complete Extended Clipboard capabilities exchange");
    handle.copy_text("中文剪贴板".into());
    assert!(
        wait_for(&handle, Duration::from_secs(5), |event| {
            matches!(event, SessionEvent::Clipboard(text) if text == "远程剪贴板")
        }),
        "expected UTF-8 clipboard text from the server"
    );
    release_tx.send(()).unwrap();
    handle.close();
    drop(handle);
    assert_eq!(server.join().unwrap(), "中文剪贴板");
}

#[test]
fn utf8_clipboard_matches_trollvnc_capabilities_in_both_directions() {
    assert_extended_clipboard_roundtrip(false);
}

#[test]
fn utf8_clipboard_uses_notify_when_the_server_supports_it() {
    assert_extended_clipboard_roundtrip(true);
}

/// Regression: a busy desktop produces one `FrameReady` per rectangle. The UI
/// rebuilt its image for every one of them, stalling the thread that also
/// delivers key events; the late key-up made the remote auto-repeat the key.
#[test]
fn drain_collapses_frame_events_to_newest_generation() {
    let mut events = vec![
        SessionEvent::Status("a".into()),
        SessionEvent::FrameReady { generation: 1 },
        SessionEvent::FrameReady { generation: 2 },
        SessionEvent::Bell,
        SessionEvent::FrameReady { generation: 3 },
        SessionEvent::Clipboard("x".into()),
    ];
    coalesce_frames(&mut events);
    assert!(
        matches!(
            events.as_slice(),
            [
                SessionEvent::Status(_),
                SessionEvent::Bell,
                SessionEvent::FrameReady { generation: 3 },
                SessionEvent::Clipboard(_),
            ]
        ),
        "unexpected events: {events:?}"
    );

    let mut none = vec![SessionEvent::Bell, SessionEvent::Disconnected];
    coalesce_frames(&mut none);
    assert!(matches!(
        none.as_slice(),
        [SessionEvent::Bell, SessionEvent::Disconnected]
    ));

    let mut single = vec![SessionEvent::FrameReady { generation: 7 }];
    coalesce_frames(&mut single);
    assert!(matches!(
        single.as_slice(),
        [SessionEvent::FrameReady { generation: 7 }]
    ));
}

/// Server that floods the client with tiny Raw rectangles as fast as the
/// socket accepts them, while reporting every KeyEvent it reads on `keys`.
fn flooding_rfb_server(mut sock: TcpStream, keys: mpsc::Sender<(u32, bool)>) {
    const RECTS_PER_UPDATE: u16 = 256;
    rfb_handshake(&mut sock, 64, 64, b"flood");

    let mut writer = sock.try_clone().unwrap();
    let flood = thread::spawn(move || {
        // One update = RECTS_PER_UPDATE 1×1 Raw rects (16 bytes each).
        let mut update = vec![0u8, 0];
        update.extend_from_slice(&RECTS_PER_UPDATE.to_be_bytes());
        for i in 0..RECTS_PER_UPDATE {
            update.extend_from_slice(&(i % 64).to_be_bytes());
            update.extend_from_slice(&(i / 64).to_be_bytes());
            update.extend_from_slice(&1u16.to_be_bytes());
            update.extend_from_slice(&1u16.to_be_bytes());
            update.extend_from_slice(&0u32.to_be_bytes());
            update.extend_from_slice(&[200, 30, 30, 255]);
        }
        while writer.write_all(&update).is_ok() {}
    });

    sock.set_read_timeout(Some(Duration::from_secs(10))).ok();
    loop {
        let mut typ = [0u8; 1];
        if sock.read_exact(&mut typ).is_err() {
            break;
        }
        match typ[0] {
            0 => {
                let _ = read_exact(&mut sock, 3 + 16);
            }
            2 => {
                let _ = read_exact(&mut sock, 1);
                let n = u16::from_be_bytes(read_exact(&mut sock, 2).try_into().unwrap());
                let _ = read_exact(&mut sock, n as usize * 4);
            }
            3 => {
                let _ = read_exact(&mut sock, 9);
            }
            4 => {
                let b = read_exact(&mut sock, 7);
                let keysym = u32::from_be_bytes(b[3..7].try_into().unwrap());
                if keys.send((keysym, b[0] == 1)).is_err() {
                    break;
                }
            }
            5 => {
                let _ = read_exact(&mut sock, 5);
            }
            6 => {
                let _ = read_exact(&mut sock, 3);
                let len = u32::from_be_bytes(read_exact(&mut sock, 4).try_into().unwrap());
                let _ = read_exact(&mut sock, len as usize);
            }
            _ => break,
        }
    }
    let _ = sock.shutdown(std::net::Shutdown::Both);
    let _ = flood.join();
}

/// Regression for one keystroke typing a whole row of characters.
///
/// A streaming server delivers hundreds of rectangles per second. The UI used
/// to rebuild its image once per rectangle, stalling the thread that also
/// delivers key events; the delayed key-up made the remote auto-repeat the
/// key. Two guarantees keep that from coming back: `drain()` reports at most
/// one frame per call however many rectangles arrived, and input reaches the
/// server promptly while the flood is in progress.
#[test]
fn key_events_are_not_starved_by_frame_updates() {
    const KEY_TIMEOUT: Duration = Duration::from_millis(500);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (keys_tx, keys_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (sock, _) = listener.accept().unwrap();
        flooding_rfb_server(sock, keys_tx);
    });

    let handle = SessionHandle::spawn(request_for(addr.port()));
    assert!(
        wait_for(&handle, Duration::from_secs(5), |e| matches!(
            e,
            SessionEvent::FrameReady { .. }
        )),
        "expected frames from the flooding server"
    );
    // Let the backlog of decoded rectangles build up before typing. The
    // generation is sampled around the pause, not around `drain()` alone: the
    // decoder runs on its own thread and may not be scheduled during the few
    // microseconds a drain takes.
    let before = handle.framebuffer.lock().unwrap().generation;
    thread::sleep(Duration::from_millis(300));

    let events = handle.drain();
    let after = handle.framebuffer.lock().unwrap().generation;
    assert!(
        after - before > 1,
        "expected many rectangles during the pause, generation {before} -> {after}"
    );
    let frames = events
        .iter()
        .filter(|e| matches!(e, SessionEvent::FrameReady { .. }))
        .count();
    assert!(
        frames <= 1,
        "drain() must report at most one frame per call, got {frames}"
    );

    for round in 0..3 {
        let keysym = u32::from(b'a') + round;
        let pressed = Instant::now();
        handle.key(keysym, true);
        handle.key(keysym, false);
        for expect_down in [true, false] {
            let got = keys_rx
                .recv_timeout(KEY_TIMEOUT)
                .unwrap_or_else(|_| panic!("key event {round} lost behind frame updates"));
            assert_eq!(got, (keysym, expect_down));
        }
        let latency = pressed.elapsed();
        assert!(
            latency < KEY_TIMEOUT,
            "key round {round} took {latency:?} to reach the server"
        );
        thread::sleep(Duration::from_millis(100));
    }

    handle.close();
    drop(handle);
    let _ = server.join();
}

/// Exercises the complete session, including the adapter's replayed greeting.
/// The mock decrypts the credentials before sending a successful SecurityResult.
#[test]
fn ard_authentication_frame_and_input() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        ard_server::authenticate(&mut sock, "test-user", "test-password").unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        rfb_server_init(&mut sock, 8, 8, b"ARD mock");
        assert!(
            mock_rfb_messages(sock),
            "expected framebuffer and both input types"
        );
    });
    let mut request = request_for(port);
    request.username = Some("test-user".into());
    request.password = Some("test-password".into());
    let handle = SessionHandle::spawn(request);
    assert!(
        wait_for(&handle, Duration::from_secs(5), |event| {
            if let SessionEvent::Error(error) = event {
                panic!("ARD connection failed: {error}");
            }
            matches!(event, SessionEvent::FrameReady { .. })
                && handle.framebuffer.lock().unwrap().pixels.get(..4) == Some(&[0, 80, 200, 255])
        }),
        "expected authenticated desktop frame"
    );
    {
        let fb = handle.framebuffer.lock().unwrap();
        assert_eq!((fb.width, fb.height), (8, 8));
        assert_eq!(&fb.pixels[..4], &[0, 80, 200, 255]);
    }
    handle.pointer(1, 1, 1);
    handle.key(0xff0d, true);
    handle.key(0xff0d, false);
    server.join().unwrap();
    handle.close();
}

fn assert_ard_rejected(username: Option<&str>, password: Option<&str>, expected: &str) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let sent_credentials = username.is_some() && password.is_some();
    let server = thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        let error = ard_server::authenticate(&mut sock, "test-user", "test-password").unwrap_err();
        if sent_credentials {
            assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
            // A rejected login must never be followed by ClientInit, even
            // though vnc-rs's own None path ignores authentication failures.
            let mut byte = [0];
            match sock.read(&mut byte) {
                Ok(0) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
                    ) => {}
                result => panic!("unexpected traffic after failed authentication: {result:?}"),
            }
        } else {
            assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
        }
    });
    let mut request = request_for(port);
    request.username = username.map(str::to_owned);
    request.password = password.map(str::to_owned);
    let handle = SessionHandle::spawn(request);
    assert!(
        wait_for(&handle, Duration::from_secs(5), |event| {
            match event {
                SessionEvent::Connected { .. } | SessionEvent::FrameReady { .. } => {
                    panic!("unauthenticated session started")
                }
                SessionEvent::Error(error) => {
                    assert!(error.contains(expected), "unexpected error: {error}");
                    true
                }
                _ => false,
            }
        }),
        "expected authentication error"
    );
    server.join().unwrap();
    handle.close();
}

#[test]
fn ard_wrong_password_never_starts_session() {
    assert_ard_rejected(
        Some("test-user"),
        Some("wrong-password"),
        "Mac login was rejected",
    );
}

#[test]
fn ard_wrong_username_never_starts_session() {
    assert_ard_rejected(
        Some("wrong-user"),
        Some("test-password"),
        "Mac login was rejected",
    );
}

#[test]
fn ard_missing_credentials_explain_mac_login_requirement() {
    assert_ard_rejected(
        None,
        Some("test-password"),
        "Mac requires a username and password",
    );
    assert_ard_rejected(
        Some("test-user"),
        None,
        "Mac requires a username and password",
    );
}
