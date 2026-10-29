//! Unified host chat: account-scoped storage and Bundle protocol strategies.
mod customers;
pub mod images;
mod models;
pub(crate) mod notice;
mod queue;
mod repository;
mod service;
pub mod sync;
#[cfg(test)]
mod tests;
pub mod translation;
mod vinted;
pub(crate) mod workflow;
pub use customers::set_customer_language;
pub use models::*;
pub use queue::{cancel_queued, queue_message, request_read, request_sync};
pub use repository::{accounts, conversations, dismiss_pending, draft, save_draft, thread};
use rv_core::workspace::AppInstance;
use serde_json::Value;
pub use service::Action;
#[derive(Clone, Copy, Default)]
pub struct Capabilities {
    pub text: bool,
    pub mark_read: bool,
    pub upload: bool,
}
pub struct ChatRequest {
    pub method: reqwest::Method,
    pub url: String,
    pub body: Option<Value>,
}
pub struct ImageUpload {
    pub url: String,
    pub fields: Vec<(String, String)>,
    pub file_field: String,
}
pub struct Batch<T> {
    pub rows: Vec<T>,
    pub next: Option<String>,
}
pub struct DetailBatch {
    pub conversation: Conversation,
    pub messages: Vec<Message>,
    pub participants: Vec<Participant>,
    pub next: Option<String>,
}
pub trait ChatStrategy: Send + Sync {
    fn requires_proxy(&self) -> bool {
        false
    }
    fn capabilities(&self) -> Capabilities;
    fn inbox(&self, cursor: Option<&str>) -> Result<ChatRequest, String>;
    fn detail(&self, id: &str, cursor: Option<&str>) -> Result<ChatRequest, String>;
    fn parse_inbox(&self, app: &AppInstance, reply: &Value) -> Result<Batch<Conversation>, String>;
    fn parse_detail(
        &self,
        app: &AppInstance,
        id: &str,
        reply: &Value,
    ) -> Result<DetailBatch, String>;
    fn reply(&self, id: &str, text: &str) -> Result<ChatRequest, String>;
    fn image_upload(&self) -> Result<ImageUpload, String> {
        Err("此 App 不支持发送图片".into())
    }
    fn image_reference(&self, _reply: &Value) -> Result<String, String> {
        Err("此 App 不支持发送图片".into())
    }
    fn reply_image(&self, _id: &str, _reference: &str) -> Result<ChatRequest, String> {
        Err("此 App 不支持发送图片".into())
    }
    fn sent_message_id(&self, _conversation: &str, _reply: &Value) -> Option<String> {
        None
    }
    fn mark_read(&self, id: &str) -> Result<ChatRequest, String>;
}
pub fn strategy(bundle: &str) -> Option<Box<dyn ChatStrategy>> {
    match bundle {
        "lt.manodrabuziai.fr" => Some(Box::new(vinted::VintedChat)),
        _ => None,
    }
}

pub(crate) fn recover_queue(store: &mut rv_core::workspace::WorkspaceStore) -> Result<(), String> {
    store
        .business_transaction(|db| {
            repository::schema(db)?;
            queue::recover(db)
        })
        .map_err(|e| e.to_string())
}
