use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc::UnboundedSender;
use vnc::tight::{TightFileCaps, TightFileEntry};

use crate::SessionCommand;

pub const PHOTO_PAGE_SIZE: usize = 50;

#[derive(Debug, Clone)]
pub struct TransferRecord {
    pub name: String,
    pub direction: TransferDirection,
    pub bytes: u64,
    pub total: Option<u64>,
    pub status: TransferStatus,
}

#[derive(Debug, Clone)]
pub struct PhotoEntry {
    pub id: String,
    pub name: String,
    pub created: i64,
    pub width: u32,
    pub height: u32,
    pub thumbnail: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct PhotoAlbum {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy)]
pub enum TransferDirection {
    Upload,
    Download,
}

#[derive(Debug, Clone)]
pub enum TransferStatus {
    Running,
    Sent,
    Complete,
    Verified,
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct FileTransferSnapshot {
    pub revision: u64,
    pub caps: Option<TightFileCaps>,
    pub can_delete: bool,
    pub can_mkdir: bool,
    pub can_rename: bool,
    pub can_replace: bool,
    pub can_checksum: bool,
    pub can_photos: bool,
    pub can_photo_delete: bool,
    pub can_photo_batch_delete: bool,
    pub photo_entries: Vec<PhotoEntry>,
    pub photo_albums: Vec<PhotoAlbum>,
    pub photo_album: Option<String>,
    pub photo_total: usize,
    pub photo_offset: usize,
    pub photo_busy: bool,
    pub photos_need_refresh: bool,
    pub photo_status: Option<String>,
    pub photo_error: Option<String>,
    pub photo_revision: u64,
    pub management_busy: bool,
    pub management_revision: u64,
    pub remote_path: String,
    pub listed_path: Option<String>,
    pub entries: Vec<TightFileEntry>,
    pub listing: bool,
    pub download_blocked: bool,
    pub error: Option<String>,
    pub transfers: Vec<TransferRecord>,
}

impl Default for FileTransferSnapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            caps: None,
            can_delete: false,
            can_mkdir: false,
            can_rename: false,
            can_replace: false,
            can_checksum: false,
            can_photos: false,
            can_photo_delete: false,
            can_photo_batch_delete: false,
            photo_entries: Vec::new(),
            photo_albums: Vec::new(),
            photo_album: None,
            photo_total: 0,
            photo_offset: 0,
            photo_busy: false,
            photos_need_refresh: false,
            photo_status: None,
            photo_error: None,
            photo_revision: 0,
            management_busy: false,
            management_revision: 0,
            remote_path: "/".into(),
            listed_path: None,
            entries: Vec::new(),
            listing: false,
            download_blocked: false,
            error: None,
            transfers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum FileCommand {
    List(String),
    Download {
        remote: String,
        destination: PathBuf,
        size: u64,
    },
    Upload {
        source: PathBuf,
        remote: String,
    },
    Cancel,
    Delete(String),
    CreateFolder(String),
    Rename {
        from: String,
        to: String,
    },
    Replace {
        from: String,
        to: String,
    },
    Checksum {
        remote: String,
        index: usize,
        expected: String,
        destination: Option<PathBuf>,
    },
    PhotoList {
        offset: usize,
        album: Option<String>,
    },
    PhotoImport {
        remote: String,
        expected_sha256: Option<String>,
    },
    PhotoExport {
        asset_id: String,
        destination: PathBuf,
    },
    PhotoDelete(Vec<String>),
    UploadToPhotos(PathBuf),
    /// Queue owner refreshes Photos once after all imports finish.
    UploadToPhotosQueued(PathBuf),
    PhotoCleanupExport(String),
}

#[derive(Clone)]
pub struct FileTransferClient {
    pub(crate) state: Arc<Mutex<FileTransferSnapshot>>,
    pub(crate) commands: UnboundedSender<SessionCommand>,
}

impl FileTransferClient {
    pub fn revision(&self) -> u64 {
        self.state
            .lock()
            .expect("file transfer state lock")
            .revision
    }

    /// Avoid copying directory entries and thumbnails on unchanged UI ticks.
    pub fn snapshot_if_changed(&self, revision: u64) -> Option<FileTransferSnapshot> {
        let state = self.state.lock().expect("file transfer state lock");
        (state.revision != revision).then(|| state.clone())
    }

    pub fn snapshot(&self) -> FileTransferSnapshot {
        self.state.lock().expect("file transfer state lock").clone()
    }

    pub fn send(&self, command: FileCommand) {
        let _ = self.commands.send(SessionCommand::File(command));
    }
}

pub(crate) fn update(
    state: &Arc<Mutex<FileTransferSnapshot>>,
    f: impl FnOnce(&mut FileTransferSnapshot),
) {
    let mut state = state.lock().expect("file transfer state lock");
    f(&mut state);
    state.revision = state.revision.wrapping_add(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_snapshot_is_not_cloned_and_new_revision_is_visible() {
        let state = Arc::new(Mutex::new(FileTransferSnapshot::default()));
        let (commands, _rx) = tokio::sync::mpsc::unbounded_channel();
        let client = FileTransferClient {
            state: state.clone(),
            commands,
        };
        assert!(client.snapshot_if_changed(0).is_none());
        update(&state, |snapshot| snapshot.photos_need_refresh = true);
        let changed = client.snapshot_if_changed(0).unwrap();
        assert!(changed.photos_need_refresh);
        assert!(client.snapshot_if_changed(changed.revision).is_none());
    }
}
