use super::{CodeStrategy, Message, generic};
use crate::mailbox::message::plain_text;
use regex::Regex;
use std::sync::LazyLock;

pub(super) struct Vinted;

static LOGIN_CODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b4[- ]digit\s+code\b[^0-9]{0,100}([0-9]{4})(?:[^0-9]|$)")
        .expect("static Vinted login code")
});

impl CodeStrategy for Vinted {
    fn matches(&self, message: &Message<'_>) -> bool {
        let sender = message.sender.to_ascii_lowercase();
        let Some((local, domain)) = sender.rsplit_once('@') else {
            return false;
        };
        // Recognize direct messages and Apple's encoded original sender.
        domain.split('.').next() == Some("vinted")
            || (domain == "privaterelay.appleid.com" && local.contains("_at_vinted_"))
    }
    fn extract(&self, message: &Message<'_>) -> Option<String> {
        // Vinted's "4-digit code to verify that it's you" must keep leading zeros.
        LOGIN_CODE
            .captures(&plain_text(message.body))
            .map(|capture| capture[1].to_owned())
            .or_else(|| generic::extract(message.subject, message.body))
    }
}
