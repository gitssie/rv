//! TightVNC 1.x file transfer wire format (Security Type 16).
//! This is separate from framebuffer encoding 7 and UltraVNC's type 7.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::VncError;

const MAX_LIST_BYTES: usize = 16 * 1024 * 1024;
const MAX_CAPS: usize = 256;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TightFileCaps {
    pub list: bool,
    pub download: bool,
    pub upload: bool,
}

#[derive(Debug, Clone)]
pub struct TightFileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u32,
    pub modified: u32,
}

#[derive(Debug, Clone)]
pub enum TightFileEvent {
    Capabilities(TightFileCaps),
    ManagementAvailable {
        delete: bool,
        mkdir: bool,
        rename: bool,
        photos: bool,
        photo_delete: bool,
        photo_batch_delete: bool,
        replace: bool,
        checksum: bool,
    },
    ManagementResult {
        op: u8,
        id: u32,
        status: u8,
        message: String,
    },
    List {
        failed: bool,
        entries: Vec<TightFileEntry>,
    },
    DownloadData(Vec<u8>),
    DownloadEnd {
        modified: u32,
    },
    DownloadFailed(String),
    UploadCancelled(String),
}

#[derive(Debug, Clone)]
pub enum TightFileCommand {
    List(String),
    Download(String),
    Upload(String),
    UploadData(Vec<u8>),
    UploadEnd(u32),
    CancelDownload,
    FailUpload,
    Manage {
        op: u8,
        id: u32,
        path: String,
        destination: Option<String>,
    },
}

fn invalid(message: &str) -> VncError {
    VncError::General(message.to_owned())
}

pub(crate) async fn read_caps<S: AsyncRead + Unpin>(
    reader: &mut S,
) -> Result<TightFileCaps, VncError> {
    let server_count = reader.read_u16().await? as usize;
    let client_count = reader.read_u16().await? as usize;
    let encoding_count = reader.read_u16().await? as usize;
    let _padding = reader.read_u16().await?;
    if server_count > MAX_CAPS || client_count > MAX_CAPS || encoding_count > MAX_CAPS {
        return Err(invalid("too many TightVNC interaction capabilities"));
    }
    let mut server = Vec::with_capacity(server_count);
    let mut client = Vec::with_capacity(client_count);
    for (count, out) in [(server_count, &mut server), (client_count, &mut client)] {
        for _ in 0..count {
            let mut cap = [0u8; 16];
            reader.read_exact(&mut cap).await?;
            if &cap[4..8] == b"TGHT" {
                out.push(u32::from_be_bytes(cap[..4].try_into().unwrap()));
            }
        }
    }
    let mut unused = [0u8; 16];
    for _ in 0..encoding_count {
        reader.read_exact(&mut unused).await?;
    }
    Ok(TightFileCaps {
        list: server.contains(&130) && client.contains(&130),
        download: server.contains(&131)
            && server.contains(&133)
            && client.contains(&131)
            && client.contains(&134),
        upload: server.contains(&132)
            && client.contains(&132)
            && client.contains(&133)
            && client.contains(&135),
    })
}

pub(crate) async fn read_message<S: AsyncRead + Unpin>(
    reader: &mut S,
    kind: u8,
) -> Result<TightFileEvent, VncError> {
    match kind {
        137 => {
            let version = reader.read_u8().await?;
            let flags = reader.read_u8().await?;
            let _reserved = reader.read_u8().await?;
            Ok(TightFileEvent::ManagementAvailable {
                delete: version == 1 && flags & 1 != 0,
                mkdir: version == 1 && flags & 2 != 0,
                rename: version == 1 && flags & 4 != 0,
                photos: version == 1 && flags & 8 != 0,
                photo_delete: version == 1 && flags & 16 != 0,
                photo_batch_delete: version == 1 && flags & 32 != 0,
                replace: version == 1 && flags & 64 != 0,
                checksum: version == 1 && flags & 128 != 0,
            })
        }
        138 => {
            let op = reader.read_u8().await?;
            let status = reader.read_u8().await?;
            let id = reader.read_u32().await?;
            let len = reader.read_u16().await? as usize;
            if len > if op <= 3 { 255 } else { 60000 } {
                return Err(invalid("file management reply is too large"));
            }
            let mut reason = vec![0; len];
            reader.read_exact(&mut reason).await?;
            Ok(TightFileEvent::ManagementResult {
                op,
                id,
                status,
                message: String::from_utf8(reason)
                    .map_err(|_| invalid("file management reply is not UTF-8"))?,
            })
        }
        130 => {
            let flags = reader.read_u8().await?;
            let count = reader.read_u16().await? as usize;
            let data_size = reader.read_u16().await? as usize;
            let compressed_size = reader.read_u16().await? as usize;
            if count * 8 + compressed_size > MAX_LIST_BYTES || data_size > MAX_LIST_BYTES {
                return Err(invalid("TightVNC file list is too large"));
            }
            let mut metadata = vec![0u8; count * 8];
            reader.read_exact(&mut metadata).await?;
            let mut names = vec![0u8; compressed_size];
            reader.read_exact(&mut names).await?;
            if compressed_size != data_size {
                return Err(invalid("compressed TightVNC file lists are unsupported"));
            }
            let mut entries = Vec::with_capacity(count);
            let mut remaining = names.as_slice();
            for i in 0..count {
                let end = remaining
                    .iter()
                    .position(|byte| *byte == 0)
                    .ok_or_else(|| invalid("invalid TightVNC file list names"))?;
                let name = String::from_utf8(remaining[..end].to_vec())
                    .map_err(|_| invalid("non-UTF-8 TightVNC file name is unsupported"))?;
                remaining = &remaining[end + 1..];
                let size = u32::from_be_bytes(metadata[i * 8..i * 8 + 4].try_into().unwrap());
                let modified =
                    u32::from_be_bytes(metadata[i * 8 + 4..i * 8 + 8].try_into().unwrap());
                entries.push(TightFileEntry {
                    name,
                    is_dir: size == u32::MAX,
                    size,
                    modified,
                });
            }
            Ok(TightFileEvent::List {
                failed: flags & 0x80 != 0,
                entries,
            })
        }
        131 => {
            let level = reader.read_u8().await?;
            let real = reader.read_u16().await? as usize;
            let compressed = reader.read_u16().await? as usize;
            if real == 0 && compressed == 0 {
                // LibVNCServer writes mtime in host byte order.
                let mut time = [0u8; 4];
                reader.read_exact(&mut time).await?;
                Ok(TightFileEvent::DownloadEnd {
                    modified: u32::from_ne_bytes(time),
                })
            } else {
                let mut data = vec![0u8; compressed];
                reader.read_exact(&mut data).await?;
                if level != 0 || real != compressed {
                    return Err(invalid("compressed TightVNC downloads are unsupported"));
                }
                Ok(TightFileEvent::DownloadData(data))
            }
        }
        132 | 133 => {
            let _unused = reader.read_u8().await?;
            let len = reader.read_u16().await? as usize;
            let mut reason = vec![0u8; len];
            reader.read_exact(&mut reason).await?;
            let reason = String::from_utf8_lossy(&reason)
                .trim_end_matches('\0')
                .to_owned();
            if kind == 132 {
                Ok(TightFileEvent::UploadCancelled(reason))
            } else {
                Ok(TightFileEvent::DownloadFailed(reason))
            }
        }
        _ => Err(invalid("unknown TightVNC file message")),
    }
}

impl TightFileCommand {
    pub(crate) async fn write<S: AsyncWrite + Unpin>(self, writer: &mut S) -> Result<(), VncError> {
        match self {
            Self::List(path) => {
                let bytes = path.as_bytes();
                let len =
                    u16::try_from(bytes.len()).map_err(|_| invalid("remote path is too long"))?;
                writer.write_all(&[130, 0]).await?;
                writer.write_u16(len).await?;
                writer.write_all(bytes).await?;
            }
            Self::Download(path) => {
                write_file_request(writer, 131, &path).await?;
            }
            Self::Upload(path) => {
                write_file_request(writer, 132, &path).await?;
            }
            Self::UploadData(data) => {
                let len =
                    u16::try_from(data.len()).map_err(|_| invalid("upload block is too large"))?;
                writer.write_all(&[133, 0]).await?;
                writer.write_u16(len).await?;
                writer.write_u16(len).await?;
                writer.write_all(&data).await?;
            }
            Self::UploadEnd(modified) => {
                writer.write_all(&[133, 0, 0, 0, 0, 0]).await?;
                writer.write_all(&modified.to_ne_bytes()).await?;
            }
            Self::CancelDownload => {
                writer
                    .write_all(&[
                        134, 0, 0, 9, b'C', b'a', b'n', b'c', b'e', b'l', b'l', b'e', b'd',
                    ])
                    .await?
            }
            Self::FailUpload => {
                writer
                    .write_all(&[
                        135, 0, 0, 9, b'C', b'a', b'n', b'c', b'e', b'l', b'l', b'e', b'd',
                    ])
                    .await?
            }
            Self::Manage {
                op,
                id,
                path,
                destination,
            } => {
                if !(1..=11).contains(&op)
                    || ((op == 3 || op == 10) && destination.is_none())
                    || (op != 3 && op != 4 && op != 10 && destination.is_some())
                    || (op == 4 && destination.as_ref().is_some_and(|hash| hash.len() != 64))
                {
                    return Err(invalid("invalid file management operation"));
                }
                let bytes = path.as_bytes();
                if bytes.len() > 4096 {
                    return Err(invalid("remote management path is too long"));
                }
                let len =
                    u16::try_from(bytes.len()).map_err(|_| invalid("remote path is too long"))?;
                let destination = destination.as_deref().unwrap_or("");
                if destination.len() > 4096 {
                    return Err(invalid("remote management destination is too long"));
                }
                let dest_len = u16::try_from(destination.len())
                    .map_err(|_| invalid("remote destination is too long"))?;
                writer.write_all(&[137, op, 0, 0]).await?;
                writer.write_u32(id).await?;
                writer.write_u16(len).await?;
                writer.write_u16(dest_len).await?;
                writer.write_all(bytes).await?;
                writer.write_all(destination.as_bytes()).await?;
            }
        }
        Ok(())
    }
}

async fn write_file_request<S: AsyncWrite + Unpin>(
    writer: &mut S,
    kind: u8,
    path: &str,
) -> Result<(), VncError> {
    let bytes = path.as_bytes();
    let len = u16::try_from(bytes.len()).map_err(|_| invalid("remote path is too long"))?;
    writer.write_all(&[kind, 0]).await?;
    writer.write_u16(len).await?;
    writer.write_u32(0).await?;
    writer.write_all(bytes).await?;
    Ok(())
}
