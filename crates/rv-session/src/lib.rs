//! Tokio-backed VNC session for RV.

mod ard;
mod compositor;
mod encodings;
mod error;
mod file_transfer;
mod session;
mod vencrypt;

pub use compositor::Framebuffer;
pub use encodings::encodings_for;
pub use error::SessionError;
pub use file_transfer::{
    FileCommand, FileTransferClient, FileTransferSnapshot, PHOTO_PAGE_SIZE, PhotoAlbum, PhotoEntry,
    TransferDirection, TransferRecord, TransferStatus,
};
#[cfg(feature = "test-support")]
pub use session::SessionTestPeer;
pub use session::{SessionCommand, SessionEvent, SessionHandle, coalesce_frames};
pub use vencrypt::VENCRYPT_SECURITY_TYPE;

#[cfg(test)]
mod tests;
