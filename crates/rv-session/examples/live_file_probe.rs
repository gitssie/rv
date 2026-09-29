//! Opt-in probe for a real TightVNC file server. The password comes from an
//! environment variable and is never stored in RV's address book. Round trips
//! use uniquely named remote items. `--management` cleans them up after checks.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rv_core::{ClipboardMode, ConnectRequest, EncryptionMode, LocalCursorMode, QualityPreset};
use rv_session::{
    FileCommand, FileTransferClient, FileTransferSnapshot, SessionEvent, SessionHandle,
    TransferStatus,
};

fn wait_for(
    session: &SessionHandle,
    files: &FileTransferClient,
    label: &str,
    predicate: impl Fn(&FileTransferSnapshot) -> bool,
) -> Result<FileTransferSnapshot, String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        while let Some(event) = session.try_recv() {
            if let SessionEvent::Error(error) = event {
                return Err(format!("{label}: {error}"));
            }
        }
        let snapshot = files.snapshot();
        if predicate(&snapshot) {
            return Ok(snapshot);
        }
        if Instant::now() >= deadline {
            return Err(format!("{label}: timed out ({:?})", snapshot.error));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_management(
    session: &SessionHandle,
    files: &FileTransferClient,
    previous_revision: u64,
    label: &str,
) -> Result<(), String> {
    let snapshot = wait_for(session, files, label, |snapshot| {
        !snapshot.management_busy
            && (snapshot.management_revision > previous_revision || snapshot.error.is_some())
    })?;
    if let Some(error) = snapshot.error {
        return Err(format!("{label}: {error}"));
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = std::env::var("RV_LIVE_VNC_HOST")?;
    let port: u16 = std::env::var("RV_LIVE_VNC_PORT")?.parse()?;
    let password = std::env::var("RV_LIVE_VNC_PASSWORD")?;
    let management = std::env::args().any(|arg| arg == "--management");
    let roundtrip = management || std::env::args().any(|arg| arg == "--roundtrip");
    let request = ConnectRequest {
        connection_id: None,
        name: "Live TightVNC probe".into(),
        host,
        port,
        username: None,
        password: Some(password),
        encryption: EncryptionMode::LetServerChoose,
        quality: QualityPreset::Auto,
        clipboard: ClipboardMode::Utf8,
        local_cursor: LocalCursorMode::Automatic,
        view_only: !roundtrip,
        shared: true,
    };
    let session = SessionHandle::spawn(request);
    let files = session.file_client();
    let snapshot = wait_for(&session, &files, "capabilities", |snapshot| {
        snapshot.caps.is_some()
    })?;
    let caps = snapshot.caps.unwrap();
    println!(
        "caps list={} upload={} download={}",
        caps.list, caps.upload, caps.download
    );
    if !caps.list {
        return Err("TightVNC file transfer is not enabled on the server".into());
    }
    let snapshot = wait_for(&session, &files, "root listing", |snapshot| {
        snapshot.listed_path.as_deref() == Some("/")
    })?;
    println!("root entries={}", snapshot.entries.len());
    if !roundtrip {
        session.close();
        return Ok(());
    }
    if !caps.upload || !caps.download {
        return Err("server did not advertise upload and download".into());
    }

    let local = tempfile::tempdir()?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let name = format!("rv-transfer-probe-{stamp}.txt");
    let remote_path = format!("/{name}");
    let source = local.path().join(&name);
    let destination = local.path().join(format!("download-{name}"));
    let content = format!("RV TightVNC live probe {stamp}\n");
    std::fs::write(&source, content.as_bytes())?;
    println!("remote_artifact={remote_path}");
    files.send(FileCommand::Upload {
        source,
        remote: remote_path.clone(),
    });
    wait_for(&session, &files, "upload", |snapshot| {
        snapshot.transfers.iter().any(|transfer| {
            transfer.name == name
                && matches!(
                    transfer.status,
                    TransferStatus::Sent | TransferStatus::Complete | TransferStatus::Verified
                )
        })
    })?;
    wait_for(&session, &files, "post-upload listing", |snapshot| {
        snapshot.entries.iter().any(|entry| {
            entry.name == name && !entry.is_dir && entry.size as usize == content.len()
        })
    })?;
    files.send(FileCommand::Download {
        remote: remote_path.clone(),
        destination: destination.clone(),
        size: content.len() as u64,
    });
    wait_for(&session, &files, "download", |snapshot| {
        snapshot.transfers.iter().any(|transfer| {
            transfer.name == format!("download-{name}")
                && matches!(
                    transfer.status,
                    TransferStatus::Complete | TransferStatus::Verified
                )
        })
    })?;
    if std::fs::read(&destination)? != content.as_bytes() {
        return Err("downloaded bytes differ from uploaded bytes".into());
    }
    println!("roundtrip=ok bytes={}", content.len());
    if management {
        wait_for(&session, &files, "management capabilities", |snapshot| {
            snapshot.can_delete && snapshot.can_mkdir && snapshot.can_rename
        })?;
        let folder = format!("/rv-management-probe-{stamp}");
        let renamed = format!("{folder}-renamed");

        let revision = files.snapshot().management_revision;
        files.send(FileCommand::CreateFolder(folder.clone()));
        wait_management(&session, &files, revision, "create folder")?;
        files.send(FileCommand::List("/".into()));
        wait_for(&session, &files, "folder listing", |snapshot| {
            snapshot.listed_path.as_deref() == Some("/")
                && snapshot
                    .entries
                    .iter()
                    .any(|entry| entry.name == folder[1..] && entry.is_dir)
        })?;

        let revision = files.snapshot().management_revision;
        files.send(FileCommand::Rename {
            from: folder.clone(),
            to: renamed.clone(),
        });
        wait_management(&session, &files, revision, "rename folder")?;
        files.send(FileCommand::List("/".into()));
        wait_for(&session, &files, "renamed listing", |snapshot| {
            snapshot.listed_path.as_deref() == Some("/")
                && snapshot
                    .entries
                    .iter()
                    .any(|entry| entry.name == renamed[1..] && entry.is_dir)
                && !snapshot
                    .entries
                    .iter()
                    .any(|entry| entry.name == folder[1..])
        })?;

        let revision = files.snapshot().management_revision;
        files.send(FileCommand::Delete(remote_path.clone()));
        wait_management(&session, &files, revision, "delete uploaded file")?;
        files.send(FileCommand::List("/".into()));
        wait_for(&session, &files, "file removed listing", |snapshot| {
            snapshot.listed_path.as_deref() == Some("/")
                && !snapshot.entries.iter().any(|entry| entry.name == name)
        })?;

        let revision = files.snapshot().management_revision;
        files.send(FileCommand::Delete(renamed.clone()));
        wait_management(&session, &files, revision, "delete folder")?;
        files.send(FileCommand::List("/".into()));
        wait_for(&session, &files, "folder removed listing", |snapshot| {
            snapshot.listed_path.as_deref() == Some("/")
                && !snapshot
                    .entries
                    .iter()
                    .any(|entry| entry.name == renamed[1..])
        })?;
        println!("management=ok create rename delete");
    }
    session.close();
    Ok(())
}
