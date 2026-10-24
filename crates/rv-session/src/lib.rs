//! Tokio-backed VNC session for RV.

mod apps;
mod karing;
mod photo_confirmation;
pub use karing::{KaringRequest, KaringResponse, karing_component};
mod screen_control;
pub use screen_control::{ScreenPasscode, ScreenProgress};
pub mod functions;
pub mod mailbox;
pub use functions::{FunctionCommand, FunctionEvent};
mod ard;
mod phone_control;
pub mod phone_http;
mod zxtouch;
pub use apps::{AppCapabilities, AppCommand, AppEvent, RemoteApp, ScreenState, valid_bundle_id};
mod app_profile;
pub use app_profile::AppProfileResponse;
mod compositor;
mod encodings;
mod error;
mod file_transfer;
mod notifications;
mod pointer_timing;
mod session;
pub use notifications::{NotificationSnapshot, PhoneNotification};
mod vencrypt;

pub use compositor::Framebuffer;
pub use encodings::encodings_for;
pub use error::SessionError;
pub use file_transfer::{
    FileCommand, FileTransferClient, FileTransferSnapshot, PHOTO_PAGE_SIZE, PhotoAlbum,
    PhotoDeleteStatus, PhotoEntry, PhotoFailure, TransferDirection, TransferRecord, TransferStatus,
};
#[cfg(feature = "test-support")]
pub use session::SessionTestPeer;
pub use session::{SessionCommand, SessionEvent, SessionHandle, coalesce_frames};
pub use vencrypt::VENCRYPT_SECURITY_TYPE;

#[cfg(test)]
mod tests;
