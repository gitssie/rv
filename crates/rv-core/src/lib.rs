//! Shared models for the RV VNC viewer.

mod app_shortcuts;
mod connection;
pub use app_shortcuts::{AppShortcut, default_app_shortcuts, move_app_shortcut};
mod keyboard;
mod keysym;
mod prefs;
mod store;
mod unlock;
pub use unlock::{UnlockCode, delete_unlock_code, load_unlock_code, save_unlock_code};

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
