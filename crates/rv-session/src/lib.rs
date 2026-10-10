//! Tokio-backed VNC session for RV.

mod apps;
pub mod functions;
pub use functions::{FunctionCommand, FunctionEvent};
mod ard;
mod zxtouch;
pub use apps::{AppCapabilities, AppCommand, AppEvent, RemoteApp, ScreenState, valid_bundle_id};
mod compositor;
mod encodings;
mod error;
mod file_transfer;
mod notifications;
mod session;
pub use notifications::{NotificationSnapshot, PhoneNotification};
mod vencrypt;

pub use compositor::Framebuffer;
pub use encodings::encodings_for;
pub use error::SessionError;
pub use file_transfer::{
    FileCommand, FileTransferClient, FileTransferSnapshot, PHOTO_PAGE_SIZE, PhotoAlbum, PhotoEntry,
    PhotoFailure, TransferDirection, TransferRecord, TransferStatus,
};
#[cfg(feature = "test-support")]
pub use session::SessionTestPeer;
pub use session::{SessionCommand, SessionEvent, SessionHandle, coalesce_frames};
pub use vencrypt::VENCRYPT_SECURITY_TYPE;

#[cfg(test)]
mod tests;
