//! Local device, App and account repositories. App keys come from real IDFV values.
mod apple_accounts;
pub use apple_accounts::*;
mod mailboxes;
mod models;
mod offline;
mod proxies;
mod repository;
pub mod strategies;
#[cfg(test)]
mod tests;
pub use mailboxes::*;
pub use models::*;
pub use offline::*;
pub use proxies::*;
pub use repository::WorkspaceStore;
