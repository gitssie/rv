//! Unified host chat: account-scoped storage and Bundle protocol strategies.
mod models;
mod notice;
mod repository;
mod service;
#[cfg(test)]
mod tests;
mod vinted;
pub use models::*;
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
