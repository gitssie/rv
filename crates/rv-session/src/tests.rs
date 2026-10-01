#[path = "../examples/support/ard.rs"]
mod ard_server;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use rv_core::{ClipboardMode, ConnectRequest, EncryptionMode, QualityPreset};

use crate::{FileCommand, TransferDirection, TransferStatus};
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

fn write_tight_cap(sock: &mut TcpStream, code: u32, signature: &[u8; 8]) {
    write_u32(sock, code);
    sock.write_all(b"TGHT").unwrap();
    sock.write_all(signature).unwrap();
}

fn tight_file_handshake(sock: &mut TcpStream) {
    sock.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
    sock.write_all(b"RFB 003.008\n").unwrap();
    assert_eq!(read_exact(sock, 12), b"RFB 003.008\n");
    sock.write_all(&[1, 16]).unwrap();
    assert_eq!(read_exact(sock, 1), [16]);
    write_u32(sock, 0); // no tunnels
    write_u32(sock, 0); // no auth capabilities, implicit None
    write_u32(sock, 0); // SecurityResult
    rfb_server_init(sock, 8, 8, b"tight-file-test");
    write_u16(sock, 4);
    write_u16(sock, 6);
    write_u16(sock, 0);
    write_u16(sock, 0);
    for (code, sig) in [
        (130, b"FTS_LSDT"),
        (131, b"FTS_DNDT"),
        (132, b"FTS_UPCN"),
        (133, b"FTS_DNFL"),
    ] {
        write_tight_cap(sock, code, sig);
    }
    for (code, sig) in [
        (130, b"FTC_LSRQ"),
        (131, b"FTC_DNRQ"),
        (132, b"FTC_UPRQ"),
        (133, b"FTC_UPDT"),
        (134, b"FTC_DNCN"),
        (135, b"FTC_UPFL"),
    ] {
        write_tight_cap(sock, code, sig);
    }
}

fn tight_file_server(mut sock: TcpStream) -> Vec<u8> {
    tight_file_handshake(&mut sock);
    let mut uploaded = Vec::new();
    let mut upload_started = false;
    loop {
        let kind = read_exact(&mut sock, 1)[0];
        match kind {
            0 => {
                let _ = read_exact(&mut sock, 19);
            }
            2 => {
                let _ = read_exact(&mut sock, 1);
                let count = u16::from_be_bytes(read_exact(&mut sock, 2).try_into().unwrap());
                let _ = read_exact(&mut sock, count as usize * 4);
            }
            3 => {
                let _ = read_exact(&mut sock, 9);
            }
            130 => {
                let _flags = read_exact(&mut sock, 1);
                let len = u16::from_be_bytes(read_exact(&mut sock, 2).try_into().unwrap()) as usize;
                assert_eq!(read_exact(&mut sock, len), b"/");
                let names = b"folder\0hello.txt\0";
                sock.write_all(&[130, 0]).unwrap();
                write_u16(&mut sock, 2);
                write_u16(&mut sock, names.len() as u16);
                write_u16(&mut sock, names.len() as u16);
                write_u32(&mut sock, u32::MAX);
                write_u32(&mut sock, 0);
                write_u32(&mut sock, 5);
                write_u32(&mut sock, 1_700_000_000);
                sock.write_all(names).unwrap();
            }
            131 | 132 => {
                let rest = read_exact(&mut sock, 7);
                let len = u16::from_be_bytes(rest[1..3].try_into().unwrap()) as usize;
                let path = read_exact(&mut sock, len);
                if kind == 131 {
                    assert_eq!(path, b"/hello.txt");
                    sock.write_all(&[131, 0, 0, 5, 0, 5]).unwrap();
                    sock.write_all(b"hello").unwrap();
                    sock.write_all(&[131, 0, 0, 0, 0, 0]).unwrap();
                    sock.write_all(&1_700_000_000u32.to_ne_bytes()).unwrap();
                } else {
                    assert_eq!(path, b"/upload.txt");
                    upload_started = true;
                }
            }
            133 => {
                assert!(upload_started);
                let rest = read_exact(&mut sock, 5);
                let real = u16::from_be_bytes(rest[1..3].try_into().unwrap()) as usize;
                let compressed = u16::from_be_bytes(rest[3..5].try_into().unwrap()) as usize;
                if real == 0 && compressed == 0 {
                    let _mtime = read_exact(&mut sock, 4);
                    return uploaded;
                }
                assert_eq!(real, compressed);
                uploaded.extend_from_slice(&read_exact(&mut sock, compressed));
            }
            kind => panic!("unexpected TightVNC client message {kind}"),
        }
    }
}

#[test]
fn tight_file_list_download_and_upload_roundtrip_without_ios() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (sock, _) = listener.accept().unwrap();
        tight_file_server(sock)
    });
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("upload.txt");
    let destination = temp.path().join("download.txt");
    std::fs::write(&source, b"upload contents").unwrap();
    let handle = SessionHandle::spawn(request_for(port));
    let files = handle.file_client();
    let until = Instant::now() + Duration::from_secs(5);
    while files.snapshot().caps.is_none() && Instant::now() < until {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(files.snapshot().caps.unwrap().upload);
    while (files.snapshot().entries.len() != 2
        || files.snapshot().listed_path.as_deref() != Some("/"))
        && Instant::now() < until
    {
        thread::sleep(Duration::from_millis(10));
    }
    let entries = files.snapshot().entries;
    assert_eq!(entries.len(), 2);
    assert!(entries[0].is_dir);
    assert_eq!(entries[1].name, "hello.txt");
    files.send(FileCommand::Upload {
        source: source.clone(),
        remote: "/hello.txt".into(),
    });
    while files.snapshot().error.is_none() && Instant::now() < until {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        files
            .snapshot()
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("already exists")
    );
    files.send(FileCommand::Download {
        remote: "/hello.txt".into(),
        destination: destination.clone(),
        size: 5,
    });
    while !destination.exists() || std::fs::read(&destination).unwrap_or_default() != b"hello" {
        assert!(
            Instant::now() < until,
            "download timed out: {:?}",
            files.snapshot().error
        );
        thread::sleep(Duration::from_millis(10));
    }
    files.send(FileCommand::Upload {
        source,
        remote: "/upload.txt".into(),
    });
    let uploaded = server.join().unwrap();
    assert_eq!(uploaded, b"upload contents");
    assert_eq!(std::fs::read(destination).unwrap(), b"hello");
    assert!(
        files
            .snapshot()
            .transfers
            .iter()
            .any(|item| matches!(item.status, TransferStatus::Complete))
    );
    handle.close();
}

#[test]
fn tight_cancelled_download_ignores_late_frames_without_ios() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (cancelled_tx, cancelled_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        tight_file_handshake(&mut sock);
        let mut download_started = false;
        loop {
            let kind = read_exact(&mut sock, 1)[0];
            match kind {
                0 => {
                    let _ = read_exact(&mut sock, 19);
                }
                2 => {
                    let _ = read_exact(&mut sock, 1);
                    let count = u16::from_be_bytes(read_exact(&mut sock, 2).try_into().unwrap());
                    let _ = read_exact(&mut sock, count as usize * 4);
                }
                3 => {
                    let _ = read_exact(&mut sock, 9);
                }
                130 => {
                    let _ = read_exact(&mut sock, 1);
                    let len = u16::from_be_bytes(read_exact(&mut sock, 2).try_into().unwrap());
                    assert_eq!(read_exact(&mut sock, len as usize), b"/");
                    let names = b"slow.bin\0";
                    sock.write_all(&[130, 0]).unwrap();
                    write_u16(&mut sock, 1);
                    write_u16(&mut sock, names.len() as u16);
                    write_u16(&mut sock, names.len() as u16);
                    write_u32(&mut sock, 10);
                    write_u32(&mut sock, 0);
                    sock.write_all(names).unwrap();
                }
                131 => {
                    let rest = read_exact(&mut sock, 7);
                    let len = u16::from_be_bytes(rest[1..3].try_into().unwrap());
                    assert_eq!(read_exact(&mut sock, len as usize), b"/slow.bin");
                    download_started = true;
                    sock.write_all(&[131, 0, 0, 3, 0, 3]).unwrap();
                    sock.write_all(b"abc").unwrap();
                }
                134 => {
                    assert!(download_started);
                    let _ = read_exact(&mut sock, 1);
                    cancelled_tx.send(()).unwrap();
                    sock.write_all(&[131, 0, 0, 7, 0, 7]).unwrap();
                    sock.write_all(b"defghij").unwrap();
                    sock.write_all(&[131, 0, 0, 0, 0, 0]).unwrap();
                    sock.write_all(&0u32.to_ne_bytes()).unwrap();
                    thread::sleep(Duration::from_millis(300));
                    return;
                }
                kind => panic!("unexpected TightVNC message after cancel: {kind}"),
            }
        }
    });
    let temp = tempfile::tempdir().unwrap();
    let destination = temp.path().join("slow.bin");
    let second = temp.path().join("second.bin");
    let handle = SessionHandle::spawn(request_for(port));
    let files = handle.file_client();
    let until = Instant::now() + Duration::from_secs(5);
    while files.snapshot().listed_path.as_deref() != Some("/") {
        assert!(Instant::now() < until, "file list timed out");
        thread::sleep(Duration::from_millis(10));
    }
    files.send(FileCommand::Download {
        remote: "/slow.bin".into(),
        destination: destination.clone(),
        size: 10,
    });
    while files
        .snapshot()
        .transfers
        .last()
        .map_or(0, |item| item.bytes)
        < 3
    {
        assert!(Instant::now() < until, "first download block timed out");
        thread::sleep(Duration::from_millis(10));
    }
    files.send(FileCommand::Cancel);
    cancelled_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    files.send(FileCommand::Download {
        remote: "/slow.bin".into(),
        destination: second.clone(),
        size: 10,
    });
    while files
        .snapshot()
        .error
        .as_deref()
        .is_none_or(|error| !error.contains("Reconnect"))
    {
        assert!(Instant::now() < until, "second download was not rejected");
        thread::sleep(Duration::from_millis(10));
    }
    assert!(files.snapshot().download_blocked);
    assert!(!destination.exists());
    assert!(!second.exists());
    handle.close();
    server.join().unwrap();
}

struct FixtureChild(Child);

impl Drop for FixtureChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn ascii_clipboard_on_real_libvncserver_matches_in_both_modes() {
    let Ok(binary) = std::env::var("RV_LIBVNCSERVER_FIXTURE") else {
        return;
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let remote = tempfile::tempdir().unwrap();
    let _server = FixtureChild(
        Command::new(binary)
            .arg(remote.path())
            .arg(port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let startup_deadline = Instant::now() + Duration::from_secs(5);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(
            Instant::now() < startup_deadline,
            "LibVNCServer fixture did not start"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let handle = SessionHandle::spawn(request_for(port));
    assert!(wait_for(&handle, Duration::from_secs(5), |event| matches!(
        event,
        SessionEvent::Connected { .. }
    )));
    let address = "user@example.com";
    handle.copy_text(address.into());
    let output = remote.path().join("clipboard.txt");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !output.exists() {
        assert!(
            Instant::now() < deadline,
            "LibVNCServer did not deliver clipboard text"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(std::fs::read(output).unwrap(), address.as_bytes());
    let mut expected_wire = address.as_bytes().to_vec();
    expected_wire.push(0);
    assert_eq!(
        std::fs::read(remote.path().join("clipboard-wire.bin")).unwrap(),
        expected_wire
    );
    handle.close();

    let mut latin1_request = request_for(port);
    latin1_request.clipboard = ClipboardMode::Latin1;
    let latin1_handle = SessionHandle::spawn(latin1_request);
    assert!(wait_for(
        &latin1_handle,
        Duration::from_secs(5),
        |event| matches!(event, SessionEvent::Connected { .. })
    ));
    latin1_handle.copy_text(address.into());
    let latin1_output = remote.path().join("clipboard-latin1.txt");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !latin1_output.exists() {
        assert!(
            Instant::now() < deadline,
            "LibVNCServer did not deliver Latin-1 text"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(std::fs::read(latin1_output).unwrap(), address.as_bytes());
    latin1_handle.close();
}

#[test]
fn tight_file_roundtrip_with_real_libvncserver_when_available() {
    let Ok(binary) = std::env::var("RV_LIBVNCSERVER_FIXTURE") else {
        return;
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let remote = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    std::fs::write(remote.path().join("hello.txt"), b"hello").unwrap();
    std::fs::create_dir_all(remote.path().join("Media/DCIM/.MISC/Incoming")).unwrap();
    let _server = FixtureChild(
        Command::new(binary)
            .arg(remote.path())
            .arg(port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let startup_deadline = Instant::now() + Duration::from_secs(5);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(
            Instant::now() < startup_deadline,
            "LibVNCServer fixture did not start"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let handle = SessionHandle::spawn(request_for(port));
    let files = handle.file_client();
    let until = Instant::now() + Duration::from_secs(5);
    while !files
        .snapshot()
        .entries
        .iter()
        .any(|entry| entry.name == "hello.txt")
    {
        assert!(
            Instant::now() < until,
            "LibVNCServer list: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    let caps = files.snapshot().caps.unwrap();
    assert!(caps.list && caps.download && caps.upload);
    while !files.snapshot().can_delete
        || !files.snapshot().can_mkdir
        || !files.snapshot().can_rename
        || !files.snapshot().can_replace
        || !files.snapshot().can_checksum
    {
        assert!(
            Instant::now() < until,
            "TrollVNC management capability: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    let destination = local.path().join("hello.txt");
    files.send(FileCommand::Download {
        remote: "/hello.txt".into(),
        destination: destination.clone(),
        size: 5,
    });
    while std::fs::read(&destination).unwrap_or_default() != b"hello" {
        assert!(
            Instant::now() < until,
            "LibVNCServer download: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    while !files.snapshot().transfers.iter().any(|transfer| {
        matches!(transfer.direction, TransferDirection::Download)
            && matches!(transfer.status, TransferStatus::Verified)
    }) {
        assert!(
            Instant::now() < until,
            "LibVNCServer download checksum: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    let source = local.path().join("upload.txt");
    std::fs::write(&source, b"uploaded").unwrap();
    files.send(FileCommand::Upload {
        source,
        remote: "/upload.txt".into(),
    });
    while std::fs::read(remote.path().join("upload.txt")).unwrap_or_default() != b"uploaded" {
        assert!(
            Instant::now() < until,
            "LibVNCServer upload: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    while !files
        .snapshot()
        .entries
        .iter()
        .any(|entry| entry.name == "upload.txt")
    {
        assert!(
            Instant::now() < until,
            "LibVNCServer post-upload list: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    while !files.snapshot().transfers.iter().any(|transfer| {
        transfer.name == "upload.txt"
            && matches!(transfer.direction, TransferDirection::Upload)
            && matches!(transfer.status, TransferStatus::Verified)
    }) {
        assert!(
            Instant::now() < until,
            "LibVNCServer upload verification: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    files.send(FileCommand::CreateFolder("/new-folder".into()));
    while !remote.path().join("new-folder").is_dir() || files.snapshot().management_busy {
        assert!(
            Instant::now() < until,
            "create folder: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    files.send(FileCommand::List("/".into()));
    while files.snapshot().listed_path.as_deref() != Some("/")
        || !files
            .snapshot()
            .entries
            .iter()
            .any(|entry| entry.name == "new-folder")
    {
        assert!(
            Instant::now() < until,
            "list new folder: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    files.send(FileCommand::Rename {
        from: "/new-folder".into(),
        to: "/renamed-folder".into(),
    });
    while remote.path().join("new-folder").exists()
        || !remote.path().join("renamed-folder").is_dir()
        || files.snapshot().management_busy
    {
        assert!(
            Instant::now() < until,
            "rename folder: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    files.send(FileCommand::List("/".into()));
    while files.snapshot().listed_path.as_deref() != Some("/")
        || !files
            .snapshot()
            .entries
            .iter()
            .any(|entry| entry.name == "renamed-folder")
    {
        assert!(
            Instant::now() < until,
            "list renamed folder: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    std::fs::write(remote.path().join("occupied.txt"), b"untouched").unwrap();
    files.send(FileCommand::Rename {
        from: "/renamed-folder".into(),
        to: "/occupied.txt".into(),
    });
    while files.snapshot().error.is_none() {
        assert!(
            Instant::now() < until,
            "reject overwrite: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert!(remote.path().join("renamed-folder").is_dir());
    assert_eq!(
        std::fs::read(remote.path().join("occupied.txt")).unwrap(),
        b"untouched"
    );
    std::fs::write(remote.path().join("replacement.txt"), b"new contents").unwrap();
    files.send(FileCommand::List("/".into()));
    while files.snapshot().listed_path.as_deref() != Some("/")
        || !files
            .snapshot()
            .entries
            .iter()
            .any(|entry| entry.name == "replacement.txt")
    {
        assert!(
            Instant::now() < until,
            "list replacement: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    files.send(FileCommand::Replace {
        from: "/replacement.txt".into(),
        to: "/occupied.txt".into(),
    });
    while remote.path().join("replacement.txt").exists() || files.snapshot().management_busy {
        assert!(
            Instant::now() < until,
            "replace file: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read(remote.path().join("occupied.txt")).unwrap(),
        b"new contents"
    );
    files.send(FileCommand::List("/".into()));
    while files.snapshot().listed_path.as_deref() != Some("/") {
        assert!(
            Instant::now() < until,
            "list after replace: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    files.send(FileCommand::Delete("/renamed-folder".into()));
    while remote.path().join("renamed-folder").exists() || files.snapshot().management_busy {
        assert!(
            Instant::now() < until,
            "delete folder: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    files.send(FileCommand::List("/".into()));
    while files.snapshot().listed_path.as_deref() != Some("/")
        || files
            .snapshot()
            .entries
            .iter()
            .any(|entry| entry.name == "renamed-folder")
    {
        assert!(
            Instant::now() < until,
            "list after delete: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("keep.txt"), b"safe").unwrap();
    std::os::unix::fs::symlink(outside.path(), remote.path().join("escape")).unwrap();
    files.send(FileCommand::List("/escape".into()));
    while files.snapshot().listed_path.as_deref() != Some("/escape") {
        assert!(
            Instant::now() < until,
            "list symlink: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    files.send(FileCommand::Delete("/escape/keep.txt".into()));
    while files.snapshot().error.is_none() {
        assert!(
            Instant::now() < until,
            "reject symlink traversal: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read(outside.path().join("keep.txt")).unwrap(),
        b"safe"
    );
    assert!(files.snapshot().can_photos);
    assert!(files.snapshot().can_photo_delete);
    assert!(files.snapshot().can_photo_batch_delete);
    files.send(FileCommand::PhotoList {
        offset: 0,
        album: None,
    });
    let photo_until = Instant::now() + Duration::from_secs(5);
    while !files
        .snapshot()
        .photo_entries
        .iter()
        .any(|entry| entry.id == "fixture-photo")
    {
        assert!(
            Instant::now() < photo_until,
            "photo list: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(files.snapshot().photo_albums[0].name, "Fixture album");
    files.send(FileCommand::PhotoList {
        offset: 0,
        album: Some("fixture-album".into()),
    });
    while files.snapshot().photo_album.as_deref() != Some("fixture-album") {
        assert!(
            Instant::now() < photo_until,
            "album filter: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(files.snapshot().photo_entries[0].name, "album-photo.png");
    let photo_revision = files.snapshot().photo_revision;
    files.send(FileCommand::PhotoImport {
        remote: "/upload.txt".into(),
        expected_sha256: None,
    });
    while !files
        .snapshot()
        .photo_status
        .as_deref()
        .is_some_and(|status| status.contains("Imported into Photos"))
    {
        assert!(
            Instant::now() < photo_until,
            "photo import: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    while files.snapshot().photo_revision <= photo_revision + 1 || files.snapshot().photo_busy {
        assert!(
            Instant::now() < photo_until,
            "photo refresh: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    let destination = local.path().join("fixture.png");
    files.send(FileCommand::PhotoExport {
        asset_id: "fixture-photo".into(),
        destination: destination.clone(),
    });
    while std::fs::read(&destination).unwrap_or_default() != b"photo" {
        assert!(
            Instant::now() < photo_until,
            "photo export: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    while remote
        .path()
        .join("Media/DCIM/.MISC/Incoming/rv-export-fixture.png")
        .exists()
    {
        assert!(
            Instant::now() < photo_until,
            "photo export cleanup: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    let source = local.path().join("photo-upload.png");
    std::fs::write(&source, b"fixture-photo-bytes").unwrap();
    let before_photo_upload = files.snapshot().photo_revision;
    files.send(FileCommand::UploadToPhotos(source));
    let upload_until = Instant::now() + Duration::from_secs(5);
    while files.snapshot().photo_revision <= before_photo_upload
        || !files
            .snapshot()
            .photo_status
            .as_deref()
            .is_some_and(|status| status.contains("Imported into Photos"))
        || files.snapshot().photo_busy
    {
        assert!(
            Instant::now() < upload_until,
            "upload to Photos: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    while files.snapshot().photo_entries[0].name != "photo-upload.png" {
        assert!(
            Instant::now() < upload_until,
            "original filename after import: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    files.send(FileCommand::PhotoDelete(vec![
        "fixture-photo".into(),
        "fixture-photo-2".into(),
    ]));
    while files.snapshot().photo_total != 0 || files.snapshot().photo_busy {
        assert!(
            Instant::now() < upload_until,
            "photo deletion: {:?}",
            files.snapshot()
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        std::fs::read_dir(remote.path().join("Media/DCIM/.MISC/Incoming"))
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("rv-upload-"))
    );
    handle.close();
}

#[test]
fn tight_security_vnc_auth_reaches_desktop() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        sock.write_all(b"RFB 003.008\n").unwrap();
        assert_eq!(read_exact(&mut sock, 12), b"RFB 003.008\n");
        sock.write_all(&[1, 16]).unwrap();
        assert_eq!(read_exact(&mut sock, 1), [16]);
        write_u32(&mut sock, 0); // tunnels
        write_u32(&mut sock, 1); // one auth method
        write_u32(&mut sock, 2); // VNC auth
        sock.write_all(b"STDVVNCAUTH_").unwrap();
        assert_eq!(read_exact(&mut sock, 4), 2u32.to_be_bytes());
        sock.write_all(&[7; 16]).unwrap();
        assert_eq!(read_exact(&mut sock, 16).len(), 16); // DES challenge response
        write_u32(&mut sock, 0); // SecurityResult
        rfb_server_init(&mut sock, 8, 8, b"tight-auth");
        sock.write_all(&[0; 8]).unwrap(); // no file capabilities
        mock_rfb_messages(sock)
    });
    let mut request = request_for(port);
    request.password = Some("test-password".into());
    let handle = SessionHandle::spawn(request);
    assert!(wait_for(&handle, Duration::from_secs(5), |event| matches!(
        event,
        SessionEvent::Connected { .. }
    )));
    handle.pointer(1, 1, 1);
    handle.key(0xff0d, true);
    assert!(server.join().unwrap());
    handle.close();
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

fn write_app_reply(sock: &mut TcpStream, op: u8, id: u32, payload: serde_json::Value) {
    let json = payload.to_string();
    sock.write_all(&[140, 1, op, 0]).unwrap();
    write_u32(sock, id);
    write_u32(sock, json.len() as u32);
    sock.write_all(json.as_bytes()).unwrap();
}

#[test]
fn passive_lock_push_over_rfb_needs_no_state_request_and_preserves_frames() {
    use crate::AppEvent;
    use serde_json::json;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        rfb_handshake(&mut sock, 1, 1, b"lock-push");
        let mut sent_change = false;
        loop {
            let mut typ = [0];
            if sock.read_exact(&mut typ).is_err() {
                break;
            }
            match typ[0] {
                0 => {
                    read_exact(&mut sock, 19);
                }
                2 => {
                    read_exact(&mut sock, 1);
                    let n = u16::from_be_bytes(read_exact(&mut sock, 2).try_into().unwrap());
                    let encodings = read_exact(&mut sock, n as usize * 4);
                    assert!(
                        encodings
                            .chunks_exact(4)
                            .any(|v| v == 0xC0A1_A990u32.to_be_bytes())
                    );
                    write_app_reply(
                        &mut sock,
                        0,
                        0,
                        json!({"unlock":true,"lock":true,"control":true}),
                    );
                    write_app_reply(&mut sock, 11, 0, json!({"locked":true}));
                }
                3 => {
                    read_exact(&mut sock, 9);
                    sock.write_all(&[0, 0]).unwrap();
                    write_u16(&mut sock, 1);
                    for v in [0, 0, 1, 1] {
                        write_u16(&mut sock, v);
                    }
                    write_u32(&mut sock, 0);
                    sock.write_all(&[11, 22, 33, 255]).unwrap();
                    if !sent_change {
                        write_app_reply(&mut sock, 11, 0, json!({"locked":false}));
                        sent_change = true;
                    }
                }
                139 => panic!("Passive state push must not require a device-state request"),
                typ => panic!("Unexpected RFB message {typ}"),
            }
        }
        assert!(sent_change);
    });
    let handle = SessionHandle::spawn(request_for(port));
    let mut observed = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && observed.len() < 2 {
        for event in handle.drain() {
            if let SessionEvent::Apps(AppEvent::LockState(locked)) = event {
                observed.push(locked);
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(observed, [true, false]);
    assert_eq!(
        &handle.framebuffer.lock().unwrap().pixels[..4],
        &[11, 22, 33, 255]
    );
    handle.close();
    drop(handle);
    server.join().unwrap();
}

#[test]
fn app_control_roundtrip_over_plain_rfb_preserves_framebuffer() {
    use crate::{AppCommand, AppEvent};
    use base64::Engine;
    use serde_json::json;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        rfb_handshake(&mut sock, 1, 1, b"apps-plain");
        let mut saw_launch = false;
        let mut saw_icon = false;
        loop {
            let mut typ = [0];
            if sock.read_exact(&mut typ).is_err() {
                break;
            }
            match typ[0] {
                0 => {
                    read_exact(&mut sock, 19);
                }
                2 => {
                    read_exact(&mut sock, 1);
                    let n = u16::from_be_bytes(read_exact(&mut sock, 2).try_into().unwrap());
                    let bytes = read_exact(&mut sock, n as usize * 4);
                    let encodings: Vec<_> = bytes
                        .chunks_exact(4)
                        .map(|b| u32::from_be_bytes(b.try_into().unwrap()))
                        .collect();
                    assert!(encodings.contains(&0xC0A1_A990));
                    assert!(
                        !encodings.contains(&(vnc::VncEncoding::TrollFileManagementPseudo as u32)),
                        "App extension must not depend on Tight file transfer"
                    );
                    write_app_reply(
                        &mut sock,
                        0,
                        0,
                        json!({"list":true,"launch":true,"icons":true,"control":true}),
                    );
                }
                3 => {
                    read_exact(&mut sock, 9);
                    sock.write_all(&[0, 0]).unwrap();
                    write_u16(&mut sock, 1);
                    for v in [0, 0, 1, 1] {
                        write_u16(&mut sock, v);
                    }
                    write_u32(&mut sock, 0);
                    sock.write_all(&[11, 22, 33, 255]).unwrap();
                }
                139 => {
                    let head = read_exact(&mut sock, 11);
                    assert_eq!(head[0], 1);
                    assert_eq!(head[2], 0);
                    let id = u32::from_be_bytes(head[3..7].try_into().unwrap());
                    assert_ne!(id, 0);
                    let length = u32::from_be_bytes(head[7..11].try_into().unwrap()) as usize;
                    let request: serde_json::Value =
                        serde_json::from_slice(&read_exact(&mut sock, length)).unwrap();
                    match head[1] {
                        1 => write_app_reply(
                            &mut sock,
                            1,
                            id,
                            json!({"apps":[{"bundle_id":"com.example.app","name":"Example","can_launch":true,"can_terminate":true}]}),
                        ),
                        2 => {
                            assert_eq!(request["bundle_id"], "com.example.app");
                            saw_launch = true;
                            write_app_reply(
                                &mut sock,
                                2,
                                id,
                                json!({"bundle_id":"com.example.app"}),
                            );
                        }
                        6 => {
                            assert_eq!(request["bundle_id"], "com.example.app");
                            saw_icon = true;
                            let image =
                                image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]));
                            let mut png = std::io::Cursor::new(vec![]);
                            image.write_to(&mut png, image::ImageFormat::Png).unwrap();
                            write_app_reply(
                                &mut sock,
                                6,
                                id,
                                json!({"png":base64::engine::general_purpose::STANDARD.encode(png.into_inner())}),
                            );
                        }
                        op => panic!("Unexpected app operation {op}"),
                    }
                }
                5 => {
                    read_exact(&mut sock, 5);
                }
                typ => panic!("Unexpected RFB message {typ}"),
            }
        }
        assert!(saw_launch && saw_icon);
    });
    let handle = SessionHandle::spawn(request_for(port));
    assert!(wait_for(
        &handle,
        Duration::from_secs(5),
        |e| matches!(e, SessionEvent::Apps(AppEvent::List(apps)) if apps.len() == 1)
    ));
    handle.app(AppCommand::Icon("com.example.app".into()));
    handle.app(AppCommand::Launch("com.example.app".into()));
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut icon = false;
    let mut launched = false;
    while Instant::now() < deadline && !(icon && launched) {
        for event in handle.drain() {
            match event {
                SessionEvent::Apps(AppEvent::Icon {
                    bgra,
                    width,
                    height,
                    ..
                }) => {
                    assert_eq!((width, height), (1, 1));
                    assert_eq!(bgra, [3, 2, 1, 255]);
                    icon = true;
                }
                SessionEvent::Apps(AppEvent::Finished(AppCommand::Launch(_))) => launched = true,
                SessionEvent::Apps(AppEvent::Failed { message, .. }) => panic!("{message}"),
                _ => {}
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(icon && launched);
    assert_eq!(
        &handle.framebuffer.lock().unwrap().pixels[..4],
        &[11, 22, 33, 255]
    );
    handle.close();
    drop(handle);
    server.join().unwrap();
}

#[test]
fn unsupported_app_command_does_not_break_old_rfb_peer() {
    use crate::{AppCommand, AppEvent};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        rfb_handshake(&mut sock, 8, 8, b"old-peer");
        mock_rfb_messages(sock)
    });
    let handle = SessionHandle::spawn(request_for(port));
    assert!(wait_for(&handle, Duration::from_secs(5), |e| matches!(
        e,
        SessionEvent::Connected { .. }
    )));
    handle.app(AppCommand::Launch("com.example.app".into()));
    assert!(wait_for(&handle, Duration::from_secs(3), |e| matches!(
        e,
        SessionEvent::Apps(AppEvent::Failed { .. })
    )));
    handle.pointer(1, 1, 0);
    handle.key(0xff0d, true);
    assert!(
        server.join().unwrap(),
        "normal frame/input must survive unsupported App control"
    );
    handle.close();
}
