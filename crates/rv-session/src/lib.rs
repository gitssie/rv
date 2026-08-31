//! Tokio-backed VNC session for RV.

mod compositor;
mod encodings;
mod error;
mod session;
mod vencrypt;

pub use compositor::Framebuffer;
pub use encodings::encodings_for;
pub use error::SessionError;
pub use session::{SessionCommand, SessionEvent, SessionHandle, coalesce_frames};
pub use vencrypt::VENCRYPT_SECURITY_TYPE;

#[cfg(test)]
mod tests;
