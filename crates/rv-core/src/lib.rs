//! Shared models for the RV VNC viewer.

mod connection;
mod keyboard;
mod keysym;
mod prefs;
mod store;

pub use connection::{
    ClipboardMode, ConnectRequest, Connection, ConnectionId, EncryptionMode, LocalCursorMode,
    QualityPreset, ScaleMode, TransferFolders, parse_server,
};
pub use keyboard::{Keyboard, keysym_for_keystroke};
pub use keysym::{
    CAD_KEYSYMS, XK_ALT_L, XK_CAPS_LOCK, XK_CONTROL_L, XK_DELETE, XK_ESCAPE, XK_SUPER_L, XK_TAB,
    keysym_name, keysym_of,
};
pub use prefs::{Preferences, ThemePref};
pub use store::{AddressBook, StoreError, StorePaths};
