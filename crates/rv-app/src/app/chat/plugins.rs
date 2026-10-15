//! Bundle UI strategies. The shell renders only the unified chat contract.
use crate::theme;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{StyledExt, h_flex, v_flex};
use rv_business::chat::{ChatImage, ContextItem, Conversation, Message};
use std::collections::HashMap;
pub(super) trait Renderer {
    fn identity(&self) -> (&'static str, &'static str, u32);
    fn message(&self, m: &Message, cx: &App) -> AnyElement {
        bubble(m, cx)
    }
    fn context(&self, item: &ContextItem, cx: &App) -> AnyElement {
        context(item, cx)
    }
}
struct Generic;
struct Vinted;
impl Renderer for Generic {
    fn identity(&self) -> (&'static str, &'static str, u32) {
        ("App", "A", 0x607080)
    }
}
impl Renderer for Vinted {
    fn identity(&self) -> (&'static str, &'static str, u32) {
        ("Vinted", "V", 0x007e8d)
    }
}
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
pub(super) fn thumbnail(image: Option<&ChatImage>, size: f32, cx: &App) -> Div {
    let mut el = div()
        .size(px(size))
        .flex_shrink_0()
        .rounded(theme::radius(cx))
        .overflow_hidden()
        .bg(theme::selected(cx));
    if let Some(path) = image.and_then(|i| i.path.as_ref()) {
        el = el.child(img(path.clone()).size_full().object_fit(ObjectFit::Cover));
    }
    el
}
pub(super) fn avatar(c: &Conversation, size: f32, cx: &App) -> Div {
    thumbnail(c.avatar.as_ref(), size, cx)
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .when(
            c.avatar.as_ref().and_then(|i| i.path.as_ref()).is_none(),
            |el| el.child(c.title.chars().next().unwrap_or('·').to_string()),
        )
}
fn context(item: &ContextItem, cx: &App) -> AnyElement {
    h_flex()
        .gap_2()
        .items_center()
        .child(thumbnail(item.image.as_ref(), 42., cx))
        .child(
            v_flex()
                .gap_1()
                .min_w_0()
                .flex_1()
                .child(div().text_sm().text_ellipsis().child(item.title.clone()))
                .children(item.price.iter().map(|p| div().text_xs().child(p.clone()))),
        )
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
pub(super) fn status(status: &str) -> &str {
    match status {
        "sending" => "发送中",
        "submitted" => "已提交，等待确认",
        "failed" => "发送失败",
        "unknown" => "结果未知，请核对",
        _ => "",
    }
}
fn bubble(m: &Message, cx: &App) -> AnyElement {
    let mut body = v_flex().max_w(relative(0.82)).gap_1();
    if !m.body.is_empty() {
        body = body.child(
            div()
                .px_3()
                .py_2()
                .rounded(theme::radius_large(cx))
                .font_normal()
                .whitespace_normal()
                .bg(if m.outgoing {
                    theme::accent(cx)
                } else {
                    theme::sidebar(cx)
                })
                .text_color(if m.outgoing {
                    theme::accent_fg(cx)
                } else {
                    theme::ink(cx)
                })
                .child(m.body.clone()),
        );
    }
    body = body.children(
        m.images
            .iter()
            .map(|image| thumbnail(Some(image), 150., cx)),
    );
    body = body.child(
        h_flex()
            .gap_2()
            .text_xs()
            .text_color(theme::muted(cx))
            .child(stamp(m.created_at))
            .child(status(&m.status).to_owned()),
    );
    h_flex()
        .w_full()
        .when(m.outgoing, |el| el.justify_end())
        .child(body)
        .into_any_element()
}
