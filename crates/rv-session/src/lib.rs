//! Tokio-backed VNC session for RV.

mod apps;
pub mod automation;
mod karing;
mod photo_confirmation;
pub use karing::{
    KaringCatalogue, KaringRequest, KaringResponse, KaringSelection, KaringSubscription,
    karing_component,
};
mod screen_control;
pub use screen_control::{ScreenPasscode, ScreenProgress};
pub mod functions;
pub mod mailbox;
pub use functions::{FunctionCommand, FunctionEvent};
mod ard;
mod phone_control;
mod phone_tokens;
pub mod phone_http;
mod zxtouch;
pub use apps::{AppCapabilities, AppCommand, AppEvent, RemoteApp, ScreenState, valid_bundle_id};
mod app_profile;
mod phone_handoff;
pub use app_profile::{AppProfileResponse, read_shadow_state};
pub use phone_handoff::write_shadow_backup;
mod compositor;
mod encodings;
mod error;
mod file_transfer;
mod notifications;
pub mod paste_touch;
mod pointer_timing;
mod safari_downloads;
mod session;
mod touch_gesture;
pub use notifications::{NotificationSnapshot, PhoneNotification};
mod vencrypt;

pub use compositor::Framebuffer;
pub use encodings::encodings_for;
pub use error::SessionError;
pub use file_transfer::{
    FileCommand, FileTransferClient, FileTransferSnapshot, PHOTO_PAGE_SIZE, PhotoAlbum,
    PhotoDeleteStatus, PhotoEntry, PhotoFailure, SafariFolder, TransferDirection, TransferRecord,
    TransferStatus,
};
#[cfg(feature = "test-support")]
pub use session::SessionTestPeer;
pub use session::{SessionCommand, SessionEvent, SessionHandle, coalesce_frames};
pub use vencrypt::VENCRYPT_SECURITY_TYPE;

#[cfg(test)]
mod tests;
