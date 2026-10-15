//! Read-only ZXTouch authentication probe using the device's stored VNC password.
//! Optional shared RFB handshake verifies the same credential without opening a UI.
//! Never prints passwords/JWTs or sends touch/app mutations.
use rv_core::{AddressBook, ConnectRequest, StorePaths};
use rv_session::{SessionEvent, SessionHandle};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args().collect();
    let host = arguments
        .get(1)
        .ok_or("usage: live_control_status_probe HOST [--check-vnc-password] [--empty-password]")?;
    let paths = StorePaths::default_dir()?;
    let book = AddressBook::load(paths.clone())?;
    let connection = book
        .connections()
        .iter()
        .find(|c| &c.host == host)
        .ok_or("phone is not saved in RV")?;
    println!(
        "Device: {} ({}:{}), ZXTouch port {}",
        connection.name,
        connection.host,
        connection.port,
        connection.zxtouch_port.unwrap_or(6000)
    );
    let password = if connection.remember_password {
        paths.load_password(connection.id)?
    } else {
        None
    };
    let password = if arguments.iter().any(|arg| arg == "--empty-password") {
        println!("Probing empty password without changing saved credentials");
        Some(String::new())
    } else {
        println!("Using saved VNC credential: {}", password.is_some());
        password
    };
    let mut request = ConnectRequest::from_connection(connection, password);
    let handle = SessionHandle::spawn_control(request.clone());
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut authenticated = false;
    while Instant::now() < deadline {
        for event in handle.drain() {
            match event {
                SessionEvent::ControlState(value) => {
                    authenticated = value;
                    println!("ZXTouch authenticated={value}");
                }
                SessionEvent::Status(message) => println!("ZXTouch status: {message}"),
                _ => {}
            }
        }
        if authenticated {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    handle.close();
    if arguments.iter().any(|arg| arg == "--check-vnc-password") {
        let mut socket = TcpStream::connect((request.host.as_str(), request.port))?;
        socket.set_read_timeout(Some(Duration::from_secs(3)))?;
        let mut version = [0u8; 12];
        socket.read_exact(&mut version)?;
        socket.write_all(b"RFB 003.008\n")?;
        let mut count = [0u8; 1];
        socket.read_exact(&mut count)?;
        let mut types = vec![0u8; count[0] as usize];
        socket.read_exact(&mut types)?;
        let password_required = !types.contains(&1) && types.contains(&2);
        drop(socket);
        println!("VNC requires password authentication: {password_required}");
        request.shared = true;
        let vnc = SessionHandle::spawn(request);
        let deadline = Instant::now() + Duration::from_secs(12);
        let mut accepted = false;
        'handshake: while Instant::now() < deadline {
            for event in vnc.drain() {
                match event {
                    SessionEvent::Connected { .. } => {
                        accepted = true;
                        break 'handshake;
                    }
                    SessionEvent::Error(_) => break 'handshake,
                    _ => {}
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        vnc.close();
        println!(
            "VNC connection handshake: {}",
            if accepted {
                "PASS (connection only; password validation is not proven by this probe)"
            } else {
                "FAILED"
            }
        );
    }
    println!("ZXTouch online={authenticated}");
    if authenticated {
        Ok(())
    } else {
        Err("independent ZXTouch authentication has not succeeded".into())
    }
}
