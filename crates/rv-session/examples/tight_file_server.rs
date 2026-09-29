//! Local TightVNC 1.x file transfer fixture for RV UI and protocol testing.
//! Run: cargo run -p rv-session --example tight_file_server -- /tmp/rv-files
//! Then connect RV to 127.0.0.1:5999. The root directory must exist.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

fn read_u16(sock: &mut TcpStream) -> io::Result<u16> {
    let mut bytes = [0; 2];
    sock.read_exact(&mut bytes)?;
    Ok(u16::from_be_bytes(bytes))
}

fn read_u32(sock: &mut TcpStream) -> io::Result<u32> {
    let mut bytes = [0; 4];
    sock.read_exact(&mut bytes)?;
    Ok(u32::from_be_bytes(bytes))
}

fn write_cap(sock: &mut TcpStream, code: u32, signature: &[u8; 8]) -> io::Result<()> {
    sock.write_all(&code.to_be_bytes())?;
    sock.write_all(b"TGHT")?;
    sock.write_all(signature)
}

fn write_caps(sock: &mut TcpStream) -> io::Result<()> {
    sock.write_all(&4u16.to_be_bytes())?;
    sock.write_all(&6u16.to_be_bytes())?;
    sock.write_all(&0u16.to_be_bytes())?;
    sock.write_all(&0u16.to_be_bytes())?;
    for (code, signature) in [
        (130, b"FTS_LSDT"),
        (131, b"FTS_DNDT"),
        (132, b"FTS_UPCN"),
        (133, b"FTS_DNFL"),
    ] {
        write_cap(sock, code, signature)?;
    }
    for (code, signature) in [
        (130, b"FTC_LSRQ"),
        (131, b"FTC_DNRQ"),
        (132, b"FTC_UPRQ"),
        (133, b"FTC_UPDT"),
        (134, b"FTC_DNCN"),
        (135, b"FTC_UPFL"),
    ] {
        write_cap(sock, code, signature)?;
    }
    Ok(())
}

fn handshake(sock: &mut TcpStream) -> io::Result<()> {
    sock.write_all(b"RFB 003.008\n")?;
    let mut version = [0; 12];
    sock.read_exact(&mut version)?;
    if &version != b"RFB 003.008\n" {
        return Err(io::Error::other("expected RFB 3.8"));
    }
    sock.write_all(&[1, 16])?;
    let mut choice = [0];
    sock.read_exact(&mut choice)?;
    if choice != [16] {
        return Err(io::Error::other("expected Tight security type"));
    }
    sock.write_all(&0u32.to_be_bytes())?; // no tunnels
    sock.write_all(&0u32.to_be_bytes())?; // implicit None auth
    sock.write_all(&0u32.to_be_bytes())?; // SecurityResult OK
    sock.read_exact(&mut choice)?; // ClientInit
    sock.write_all(&320u16.to_be_bytes())?;
    sock.write_all(&568u16.to_be_bytes())?;
    sock.write_all(&[32, 24, 0, 1])?;
    for _ in 0..3 {
        sock.write_all(&255u16.to_be_bytes())?;
    }
    sock.write_all(&[16, 8, 0, 0, 0, 0])?;
    let name = b"Local TightVNC file fixture";
    sock.write_all(&(name.len() as u32).to_be_bytes())?;
    sock.write_all(name)?;
    write_caps(sock)
}

fn remote_path(root: &Path, raw: &[u8], creating: bool) -> io::Result<PathBuf> {
    let name = std::str::from_utf8(raw).map_err(|_| io::Error::other("invalid UTF-8 path"))?;
    if !name.starts_with('/') || name.contains('\0') {
        return Err(io::Error::other("invalid remote path"));
    }
    let mut path = root.to_path_buf();
    for component in name.split('/').filter(|part| !part.is_empty()) {
        if component == "." || component == ".." || component.contains('\\') {
            return Err(io::Error::other("invalid remote path"));
        }
        path.push(component);
    }
    if creating {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("invalid destination"))?;
        if !parent.canonicalize()?.starts_with(root) {
            return Err(io::Error::other("path outside fixture root"));
        }
        Ok(path)
    } else {
        let canonical = path.canonicalize()?;
        if !canonical.starts_with(root) {
            return Err(io::Error::other("path outside fixture root"));
        }
        Ok(canonical)
    }
}

fn read_path(sock: &mut TcpStream, list: bool) -> io::Result<Vec<u8>> {
    let mut pad = [0];
    sock.read_exact(&mut pad)?;
    let len = read_u16(sock)? as usize;
    if !list {
        let _position = read_u32(sock)?;
    }
    let mut path = vec![0; len];
    sock.read_exact(&mut path)?;
    Ok(path)
}

fn file_list(sock: &mut TcpStream, root: &Path, raw: &[u8]) -> io::Result<()> {
    let Ok(path) = remote_path(root, raw, false) else {
        return sock.write_all(&[130, 0x80, 0, 0, 0, 0, 0, 0]);
    };
    let Ok(listing) = fs::read_dir(path) else {
        return sock.write_all(&[130, 0x80, 0, 0, 0, 0, 0, 0]);
    };
    let mut entries = Vec::new();
    for item in listing.flatten() {
        let name = item.file_name().to_string_lossy().into_owned();
        if name.contains('\0') {
            continue;
        }
        let Ok(metadata) = item.metadata() else {
            continue;
        };
        let size = if metadata.is_dir() {
            u32::MAX
        } else {
            metadata.len().min(u32::MAX as u64 - 1) as u32
        };
        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs().min(u32::MAX as u64) as u32);
        entries.push((name, size, modified));
    }
    entries.truncate(u16::MAX as usize);
    let mut names = Vec::new();
    let mut sizes = Vec::new();
    for (name, size, modified) in &entries {
        if names.len() + name.len() + 1 > u16::MAX as usize {
            break;
        }
        sizes.extend_from_slice(&size.to_be_bytes());
        sizes.extend_from_slice(&modified.to_be_bytes());
        names.extend_from_slice(name.as_bytes());
        names.push(0);
    }
    let count = (sizes.len() / 8) as u16;
    sock.write_all(&[130, 0])?;
    sock.write_all(&count.to_be_bytes())?;
    sock.write_all(&(names.len() as u16).to_be_bytes())?;
    sock.write_all(&(names.len() as u16).to_be_bytes())?;
    sock.write_all(&sizes)?;
    sock.write_all(&names)
}

fn reason(sock: &mut TcpStream, kind: u8, message: &str) -> io::Result<()> {
    let bytes = message.as_bytes();
    sock.write_all(&[kind, 0])?;
    sock.write_all(&(bytes.len() as u16).to_be_bytes())?;
    sock.write_all(bytes)
}

fn download(sock: &mut TcpStream, root: &Path, raw: &[u8]) -> io::Result<()> {
    let result = remote_path(root, raw, false).and_then(|path| {
        let file = File::open(&path)?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::other("not a file"));
        }
        Ok(file)
    });
    let mut file = match result {
        Ok(file) => file,
        Err(error) => return reason(sock, 133, &error.to_string()),
    };
    let mut block = [0; 8192];
    loop {
        let count = file.read(&mut block)?;
        if count == 0 {
            break;
        }
        sock.write_all(&[131, 0])?;
        sock.write_all(&(count as u16).to_be_bytes())?;
        sock.write_all(&(count as u16).to_be_bytes())?;
        sock.write_all(&block[..count])?;
    }
    sock.write_all(&[131, 0, 0, 0, 0, 0])?;
    sock.write_all(&0u32.to_ne_bytes())
}

fn serve(mut sock: TcpStream, root: &Path) -> io::Result<()> {
    handshake(&mut sock)?;
    let mut first_frame = true;
    let mut upload: Option<(PathBuf, File)> = None;
    let result = (|| -> io::Result<()> {
        loop {
            let mut kind = [0];
            sock.read_exact(&mut kind)?;
            match kind[0] {
                0 => {
                    let mut data = [0; 19];
                    sock.read_exact(&mut data)?;
                }
                2 => {
                    let mut pad = [0];
                    sock.read_exact(&mut pad)?;
                    let count = read_u16(&mut sock)? as usize;
                    let mut encodings = vec![0; count * 4];
                    sock.read_exact(&mut encodings)?;
                }
                3 => {
                    let mut request = [0; 9];
                    sock.read_exact(&mut request)?;
                    if first_frame {
                        sock.write_all(&[0, 0, 0, 1])?;
                        sock.write_all(&[0, 0, 0, 0])?;
                        sock.write_all(&320u16.to_be_bytes())?;
                        sock.write_all(&568u16.to_be_bytes())?;
                        sock.write_all(&0u32.to_be_bytes())?;
                        let pixels = [45, 64, 93, 255].repeat(320 * 568);
                        sock.write_all(&pixels)?;
                        first_frame = false;
                    }
                }
                4 => {
                    let mut data = [0; 7];
                    sock.read_exact(&mut data)?;
                }
                5 => {
                    let mut data = [0; 5];
                    sock.read_exact(&mut data)?;
                }
                6 => {
                    let mut pad = [0; 3];
                    sock.read_exact(&mut pad)?;
                    let count = read_u32(&mut sock)? as usize;
                    if count > 1024 * 1024 {
                        return Err(io::Error::other("clipboard too large"));
                    }
                    let mut data = vec![0; count];
                    sock.read_exact(&mut data)?;
                }
                130 => {
                    let path = read_path(&mut sock, true)?;
                    file_list(&mut sock, root, &path)?;
                }
                131 => {
                    let path = read_path(&mut sock, false)?;
                    download(&mut sock, root, &path)?;
                }
                132 => {
                    let raw = read_path(&mut sock, false)?;
                    let result = remote_path(root, &raw, true).and_then(|path| {
                        let file = OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&path)?;
                        Ok((path, file))
                    });
                    match result {
                        Ok(next) => upload = Some(next),
                        Err(error) => reason(&mut sock, 132, &error.to_string())?,
                    }
                }
                133 => {
                    let mut level = [0];
                    sock.read_exact(&mut level)?;
                    let real = read_u16(&mut sock)? as usize;
                    let compressed = read_u16(&mut sock)? as usize;
                    if real == 0 && compressed == 0 {
                        let mut mtime = [0; 4];
                        sock.read_exact(&mut mtime)?;
                        upload.take();
                    } else {
                        let mut data = vec![0; compressed];
                        sock.read_exact(&mut data)?;
                        if level != [0] || real != compressed {
                            return Err(io::Error::other("unsupported compression"));
                        }
                        if let Some((_, file)) = upload.as_mut() {
                            file.write_all(&data)?;
                        }
                    }
                }
                134 | 135 => {
                    let mut pad = [0];
                    sock.read_exact(&mut pad)?;
                    let len = read_u16(&mut sock)? as usize;
                    let mut reason = vec![0; len];
                    sock.read_exact(&mut reason)?;
                    if kind[0] == 135
                        && let Some((path, file)) = upload.take()
                    {
                        drop(file);
                        let _ = fs::remove_file(path);
                    }
                }
                _ => return Err(io::Error::other("unsupported RFB message")),
            }
        }
    })();
    if let Some((path, file)) = upload.take() {
        drop(file);
        let _ = fs::remove_file(path);
    }
    result
}

fn main() -> io::Result<()> {
    let root = std::env::args().nth(1).ok_or_else(|| {
        io::Error::other("usage: tight_file_server <existing-root> [listen-address]")
    })?;
    let root = PathBuf::from(root).canonicalize()?;
    if !root.is_dir() {
        return Err(io::Error::other("root must be a directory"));
    }
    let address = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "127.0.0.1:5999".into());
    let listener = TcpListener::bind(&address)?;
    println!("TightVNC file fixture: {address}, root {}", root.display());
    for client in listener.incoming() {
        match client {
            Ok(sock) => {
                let _ = sock.set_nodelay(true);
                if let Err(error) = serve(sock, &root) {
                    eprintln!("client ended: {error}");
                }
            }
            Err(error) => eprintln!("accept failed: {error}"),
        }
    }
    Ok(())
}
