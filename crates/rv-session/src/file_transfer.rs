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

/// Import failures are classified by submission phase, never by translated text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhotoFailure {
    NotSubmitted,
    Unknown,
    TaskNotFound,
}
#[derive(Debug, Clone)]
pub struct FileTransferSnapshot {
    pub revision: u64,
    #[doc(hidden)]
    pub reserved_owner: Option<u64>,
    #[doc(hidden)]
    pub pending_commands: usize,
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
    pub photo_failure: Option<PhotoFailure>,
    pub photo_revision: u64,
    pub photo_asset_id: Option<String>,
    pub photo_task_id: Option<String>,
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
            reserved_owner: None,
            pending_commands: 0,
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
            photo_failure: None,
            photo_revision: 0,
            photo_asset_id: None,
            photo_task_id: None,
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
    UploadToPhotosTask {
        source: PathBuf,
        task_id: String,
    },
    PhotoQueryTask(String),
    PhotoCleanupExport(String),
}

#[derive(Clone)]
pub struct FileTransferClient {
    pub(crate) state: Arc<Mutex<FileTransferSnapshot>>,
    pub(crate) commands: UnboundedSender<SessionCommand>,
    pub(crate) owner: Option<Arc<TransferReservation>>,
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

    pub fn send(&self, command: FileCommand) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.reserved_owner.is_some()
            && state.reserved_owner != self.owner.as_ref().map(|v| v.id)
        {
            return false;
        }
        if self.commands.send(SessionCommand::File(command)).is_err() {
            return false;
        }
        state.pending_commands += 1;
        true
    }
    pub fn reserved_by_other(&self) -> bool {
        let state = self.state.lock().unwrap();
        state.reserved_owner.is_some() && state.reserved_owner != self.owner.as_ref().map(|v| v.id)
    }
    /// Reserve an idle transport for a batch. All clones retain ownership until dropped.
    pub fn try_reserve(&self) -> Option<Self> {
        if self.owner.is_some() {
            return Some(self.clone());
        }
        let mut state = self.state.lock().unwrap();
        if state.reserved_owner.is_some()
            || state.pending_commands > 0
            || state.photo_busy
            || state.management_busy
            || state.listing
            || state
                .transfers
                .iter()
                .any(|r| matches!(r.status, TransferStatus::Running | TransferStatus::Sent))
        {
            return None;
        }
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        state.reserved_owner = Some(id);
        Some(Self {
            state: self.state.clone(),
            commands: self.commands.clone(),
            owner: Some(Arc::new(TransferReservation {
                id,
                state: self.state.clone(),
            })),
        })
    }
}

pub(crate) struct TransferReservation {
    id: u64,
    state: Arc<Mutex<FileTransferSnapshot>>,
}
impl Drop for TransferReservation {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap();
        if state.reserved_owner == Some(self.id) {
            state.reserved_owner = None;
            state.revision = state.revision.wrapping_add(1);
        }
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
            owner: None,
        };
        assert!(client.snapshot_if_changed(0).is_none());
        update(&state, |snapshot| snapshot.photos_need_refresh = true);
        let changed = client.snapshot_if_changed(0).unwrap();
        assert!(changed.photos_need_refresh);
        assert!(client.snapshot_if_changed(changed.revision).is_none());
    }
    #[test]
    fn reservation_blocks_other_clients_and_lives_through_clones() {
        let state = Arc::new(Mutex::new(FileTransferSnapshot::default()));
        let (commands, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let client = FileTransferClient {
            state: state.clone(),
            commands,
            owner: None,
        };
        let owned = client.try_reserve().unwrap();
        let clone = owned.clone();
        assert!(client.try_reserve().is_none());
        assert!(!client.send(FileCommand::PhotoList {
            offset: 0,
            album: None
        }));
        drop(owned);
        assert!(client.reserved_by_other());
        assert!(clone.send(FileCommand::PhotoList {
            offset: 0,
            album: None
        }));
        assert!(receiver.try_recv().is_ok());
        drop(clone);
        assert!(!client.reserved_by_other());
        assert!(client.try_reserve().is_none()); // queued commands retain the idle gate
        state.lock().unwrap().pending_commands = 0;
        assert!(client.try_reserve().is_some());
    }
}
