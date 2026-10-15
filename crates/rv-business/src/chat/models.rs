use rv_core::workspace::AppId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatImage {
    pub url: String,
    #[serde(skip)]
    pub path: Option<PathBuf>,
}
impl ChatImage {
    pub fn from_url(url: Option<&str>) -> Option<Self> {
        url.filter(|s| !s.is_empty()).map(|s| Self {
            url: s.into(),
            path: None,
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
    pub payload: Value,
}
#[derive(Clone, Debug)]
pub struct Participant {
    pub remote_id: String,
    pub name: String,
    pub payload: Value,
}
#[derive(Clone, Debug)]
pub struct ChatAccount {
    pub id: AppId,
    pub title: String,
    pub bundle: String,
    pub has_auth: bool,
    pub session_status: String,
    pub market: String,
}
#[derive(Clone, Debug, Default)]
pub struct Thread {
    pub messages: Vec<Message>,
    pub pending: Vec<Message>,
}
pub fn local_images(
    paths: &rv_core::StorePaths,
    conversations: &mut [Conversation],
    thread: &mut Thread,
) {
    images(conversations, thread, |image| {
        image.path = crate::assets::cached(paths, &image.url)
    });
}
pub fn hydrate_images(
    paths: &rv_core::StorePaths,
    conversations: &mut [Conversation],
    thread: &mut Thread,
    thread_app: Option<&AppId>,
) {
    for conversation in conversations {
        let id = conversation.app_id.clone();
        images(
            std::slice::from_mut(conversation),
            &mut Thread::default(),
            |image| {
                image.path = crate::assets::fetch_for_app(paths, &id, &image.url).ok();
            },
        );
    }
    if let Some(id) = thread_app {
        images(&mut [], thread, |image| {
            image.path = crate::assets::fetch_for_app(paths, id, &image.url).ok();
        });
    }
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
