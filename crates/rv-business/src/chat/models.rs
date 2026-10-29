use rv_core::workspace::AppId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatImage {
    pub url: String,
    #[serde(skip)]
    pub path: Option<PathBuf>,
    #[serde(skip)]
    pub bytes: Option<Vec<u8>>,
}
impl ChatImage {
    pub fn from_url(url: Option<&str>) -> Option<Self> {
        url.filter(|s| !s.is_empty()).map(|s| Self {
            url: s.into(),
            path: None,
            bytes: None,
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextItem {
    pub remote_id: String,
    pub title: String,
    pub price: Option<String>,
    pub image: Option<ChatImage>,
    pub payload: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub app_id: AppId,
    pub remote_id: String,
    pub title: String,
    pub account: String,
    pub bundle: String,
    pub avatar: Option<ChatImage>,
    pub preview: String,
    pub updated_at: i64,
    pub unread: bool,
    pub allow_reply: Option<bool>,
    pub kind: String,
    pub items: Vec<ContextItem>,
    pub payload: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub remote_id: Option<String>,
    pub outgoing: bool,
    pub sender_id: String,
    pub created_at: i64,
    pub kind: String,
    pub body: String,
    pub status: String,
    pub images: Vec<ChatImage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation: Option<super::translation::Translation>,
    pub payload: Value,
}
#[derive(Clone, Debug)]
pub struct Participant {
    pub remote_id: String,
    pub name: String,
    pub username: Option<String>,
    pub avatar: Option<ChatImage>,
    pub country_code: Option<String>,
    pub language: Option<String>,
    pub payload: Value,
}
#[derive(Clone, Debug)]
pub struct ChatAccount {
    pub id: AppId,
    pub title: String,
    pub bundle: String,
    pub has_auth: bool,
    pub avatar: Option<ChatImage>,
    pub unread: Option<usize>,
    pub notification_unread: Option<usize>,
    pub can_send: bool,
    pub session_status: String,
    pub market: String,
}
#[derive(Clone, Debug, Default)]
pub struct Thread {
    pub messages: Vec<Message>,
    pub pending: Vec<Message>,
    pub customer: Option<Customer>,
}
pub fn local_images(
    paths: &rv_core::StorePaths,
    conversations: &mut [Conversation],
    thread: &mut Thread,
) {
    images(conversations, thread, |image| {
        if image.bytes.is_none() {
            image.path = crate::assets::cached(paths, &image.url)
        }
    });
}

fn images(
    conversations: &mut [Conversation],
    thread: &mut Thread,
    mut f: impl FnMut(&mut ChatImage),
) {
    for conversation in conversations {
        if let Some(image) = &mut conversation.avatar {
            f(image);
        }
        for item in &mut conversation.items {
            if let Some(image) = &mut item.image {
                f(image);
            }
        }
    }
    for message in thread.messages.iter_mut().chain(&mut thread.pending) {
        for image in &mut message.images {
            f(image);
        }
    }
}

#[derive(Clone, Debug)]
pub struct Customer {
    pub id: String,
    pub name: String,
    pub country_code: Option<String>,
    pub language: Option<String>,
    pub language_source: Option<String>,
    pub tags: Vec<String>,
    pub note: String,
}
