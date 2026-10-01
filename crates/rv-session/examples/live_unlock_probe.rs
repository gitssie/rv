//! Device probe: credentials arrive on stdin; this never sends passcode keys.
use rv_core::{ConnectRequest, Connection, UnlockCode};
use rv_session::{AppCommand, AppEvent, SessionEvent, SessionHandle};
use std::io::Read;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let settings: serde_json::Value = serde_json::from_str(&input)?;
    let connection = Connection::new("USB unlock probe", "127.0.0.1", 15999);
    let mut req = ConnectRequest::from_connection(
        &connection,
        settings["password"].as_str().map(str::to_owned),
    );
    req.view_only = false;
    let session = SessionHandle::spawn(req);
    let unlock = std::env::args().any(|arg| arg == "--unlock");
    let mut code = if unlock {
        Some(UnlockCode::new(
            settings["passcode"]
                .as_str()
                .ok_or("passcode missing")?
                .to_owned(),
        )?)
    } else {
        None
    };
    let mut arm_requested = false;
    let mut submitted = false;
    let mut unlocked = false;
    let prepare = std::env::args().any(|arg| arg == "--prepare");
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut next = Instant::now();
    let mut available = false;
    let mut prepare_sent = false;
    let mut next_swipe = Instant::now();
    let mut input_ready = false;
    let mut locked = false;
    while Instant::now() < deadline {
        while let Some(event) = session.try_recv() {
            match event {
                SessionEvent::Apps(AppEvent::Capabilities(caps)) => {
                    println!(
                        "capabilities unlock={} control={}",
                        caps.unlock, caps.control
                    );
                    available = caps.unlock && caps.control;
                    if available && std::env::args().any(|arg| arg == "--home") {
                        session.pointer(0, 0, 4);
                        std::thread::sleep(Duration::from_millis(80));
                        session.pointer(0, 0, 0);
                    }
                }
                SessionEvent::Apps(AppEvent::ScreenState(_, state)) => {
                    locked = state.locked;
                    input_ready = state.input_ready;
                    println!(
                        "locked={} ready={} empty={} reason={}",
                        state.locked, state.input_ready, state.input_empty, state.reason
                    );
                    if unlock && !state.locked {
                        unlocked = true;
                    }
                    if unlock
                        && !arm_requested
                        && state.locked
                        && state.input_ready
                        && state.input_empty
                    {
                        arm_requested = true;
                        session.app(AppCommand::ArmUnlock(1, 6));
                    }
                }
                SessionEvent::Apps(AppEvent::UnlockArmed(_, true)) if unlock && !submitted => {
                    submitted = true;
                    for &digit in code.take().unwrap().digits() {
                        session.key(digit as u32, true);
                        session.key(digit as u32, false);
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    println!("six keyboard digits submitted once");
                }
                SessionEvent::Apps(AppEvent::Failed { command, message })
                    if command.unlock_attempt().is_some() =>
                {
                    println!("op={command:?} failed={message}")
                }
                SessionEvent::Error(error) => return Err(error.into()),
                _ => {}
            }
        }
        if unlocked {
            break;
        }
        if available
            && locked
            && !input_ready
            && !arm_requested
            && Instant::now() >= next_swipe
            && std::env::args().any(|arg| arg == "--swipe")
        {
            let (width, height) = {
                let fb = session.framebuffer.lock().unwrap();
                (fb.width, fb.height)
            };
            if width > 0 && height > 0 {
                next_swipe = Instant::now() + Duration::from_secs(2);
                std::thread::sleep(Duration::from_millis(800));
                let start = height.saturating_sub(12);
                session.pointer(width / 2, start, 1);
                for step in 1..=20 {
                    std::thread::sleep(Duration::from_millis(18));
                    let y = start as u32 - (start as u32 - height as u32 / 3) * step / 20;
                    session.pointer(width / 2, y as u16, 1);
                }
                session.pointer(width / 2, height / 3, 0);
            }
        }
        if available && Instant::now() >= next {
            session.app(if prepare && !prepare_sent {
                prepare_sent = true;
                AppCommand::PrepareUnlock(1)
            } else {
                AppCommand::ScreenState(1)
            });
            next = Instant::now() + Duration::from_millis(500);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if locked && std::env::args().any(|arg| arg == "--snapshot") {
        if let Some(png) = session.framebuffer.lock().unwrap().thumbnail_png(1200) {
            std::fs::write("/tmp/cazer-unlock-screen.png", png)?;
        }
    }
    if unlock {
        session.app(AppCommand::ArmUnlock(1, 0));
    }
    session.close();
    if unlock && !unlocked {
        return Err("unlock was not confirmed".into());
    }
    Ok(())
}
