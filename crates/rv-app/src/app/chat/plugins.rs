//! Bundle rendering hooks compose upstream Message/Bubble surfaces and local resources only.
use crate::{assets::FunctionIcon, theme};
use gpui::{prelude::FluentBuilder as _, *};
use gpui_component::{Icon, IconName, avatar::Avatar, h_flex};
use rv_business::chat::{ChatImage, ContextItem, Conversation, Message};
use rv_gpui_chat::{
    bubble::{Bubble, BubbleVariant},
    message::{Message as MessageView, MessageAlignment, MessageContent, MessageFooter},
};
use std::collections::HashMap;
pub(super) use rv_gpui_chat::selectable_text::selectable_text;
pub(super) trait Renderer {
    fn message(&self, m: &Message, conversation: &Conversation, avatar:Avatar, cx: &App) -> AnyElement {
        message(m, conversation, avatar, cx)
    }
    fn context(&self, item: &ContextItem, cx: &App) -> AnyElement {
        context(item, cx)
    }
}
struct Vinted;
impl Renderer for Vinted {}
struct Generic;
impl Renderer for Generic {}
pub(super) struct Registry {
    plugins: HashMap<&'static str, Box<dyn Renderer>>,
    fallback: Box<dyn Renderer>,
}
impl Default for Registry {
    fn default() -> Self {
        let mut plugins: HashMap<_, Box<dyn Renderer>> = HashMap::new();
        plugins.insert("lt.manodrabuziai.fr", Box::new(Vinted));
        Self {
            plugins,
            fallback: Box::new(Generic),
        }
    }
}
impl Registry {
    pub fn renderer(&self, bundle: &str) -> &dyn Renderer {
        self.plugins
            .get(bundle)
            .map(|p| p.as_ref())
            .unwrap_or(self.fallback.as_ref())
    }
}
pub(super) fn thumbnail(image: Option<&ChatImage>, size: f32, fit: ObjectFit, cx: &App) -> Div {
    div()
        .size(px(size))
        .flex_shrink_0()
        .overflow_hidden()
        .bg(theme::selected(cx))
        .when_some(image, |el, image| {
            if let Some(path) = image.path.clone() {
                el.child(img(path).size_full().object_fit(fit))
            } else if let Some(bytes) = image.bytes.as_ref() {
                el.child(
                    img(std::sync::Arc::new(Image::from_bytes(
                        ImageFormat::Jpeg,
                        bytes.clone(),
                    )))
                    .size_full()
                    .object_fit(fit),
                )
            } else {
                el
            }
        })
}
pub(super) fn avatar(c: &Conversation) -> Avatar {
    let avatar = Avatar::new().name(c.title.clone());
    match c.avatar.as_ref().and_then(|i| i.path.clone()) {
        Some(path) => avatar.src(path),
        None => avatar,
    }
}
fn context(item: &ContextItem, cx: &App) -> AnyElement {
    let text=match &item.price {
        Some(price)=>format!("{} · {}",item.title,price),
        None=>item.title.clone(),
    };
    h_flex().id(SharedString::from(format!("chat-context-{}",item.remote_id)))
        .gap_1().items_center().min_w_0()

        .child(Icon::new(FunctionIcon::Products).text_color(theme::muted(cx)))
        .child(div().text_xs().text_color(theme::muted(cx)).min_w_0().flex_1()
            .child(SharedString::from((text).to_string())))
        .into_any_element()
}
pub(super) fn stamp(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}
fn message(m: &Message, _conversation: &Conversation, avatar:Avatar, cx: &App) -> AnyElement {
    let main = m
        .translation
        .as_ref()
        .filter(|_| m.outgoing)
        .map(|t| t.original.clone())
        .unwrap_or_else(|| m.body.clone());
    let mut content = Bubble::new()
        .with_variant(if m.outgoing {
            BubbleVariant::Filled
        } else {
            BubbleVariant::Secondary
        })
        .child(selectable_text(SharedString::from(format!("chat-message-body-{}",m.id)),main));
    if let Some(translation) = &m.translation {
        content = content.child(
            h_flex()
                .gap_2()
                .items_start()
                .border_t_1()
                .border_color(theme::line(cx))
                .child(Icon::new(FunctionIcon::Languages))
                .child(
                    div()
                        .text_xs()
                        .whitespace_normal()
                        .child(selectable_text(SharedString::from(format!("chat-message-translation-{}",m.id)), &translation.translated)),
                ),
        );
    }
    for image in &m.images {
        content = content.child(thumbnail(Some(image), 150., ObjectFit::Contain, cx));
    }
    let footer = MessageFooter::new()
        .child(div().text_xs().text_color(theme::muted(cx)).child(SharedString::from((stamp(m.created_at)).to_string())))
        .when(m.outgoing, |footer| {
            let icon: Icon = match m.status.as_str() {
                "pending" | "submitted" => Icon::new(FunctionIcon::Recent),
                "sending" => Icon::new(IconName::LoaderCircle),
                "failed" | "unknown" => Icon::new(IconName::Info),
                "cancelled" => Icon::new(IconName::Close),
                _ => Icon::new(IconName::Check),
            };
            footer.child(
                div()
                    .id(SharedString::from(format!("message-status-{}", m.id)))

                    .child(icon)
                    .when(matches!(m.status.as_str(), "failed" | "unknown"), |el| {
                        el.text_color(theme::danger(cx))
                    }),
            )
        });
    MessageView::new()
        .id(SharedString::from(format!("chat-message-{}", m.id)))
        .alignment(if m.outgoing {
            MessageAlignment::End
        } else {
            MessageAlignment::Start
        })
        .when(!m.outgoing, |row| row.avatar(avatar))
        .content(MessageContent::new().bubble(content))
        .footer(footer)
        .into_any_element()
}
