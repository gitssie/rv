use super::{CodeStrategy, Message};
use crate::mailbox::message::plain_text;
use regex::Regex;
use std::sync::LazyLock;

pub(super) struct Generic;

static MARKER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?ix)(?:\b(?:otp|pin)\b|\bcode\s*[:=]|\b(?:verification|security|login|authentication|one[- ]time|[0-9]{1,2}[- ]digit|four[- ]digit|six[- ]digit|eight[- ]digit)\s+(?:code|password)\b|\byour\s+(?:code|otp|pin)\b|\bcode\s+(?:is|to\s+verify)\b|验证码|驗證碼|一次性密码|認証コード|確認コード|\bcode\s+de\s+(?:vérification|verification|sécurité|connexion)\b|bestätigungscode|sicherheitscode)"
).expect("static code marker")
});
static TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9]+").expect("static code token"));

fn valid_token(value: &str) -> bool {
    (4..=12).contains(&value.len())
        && value.bytes().all(|b| b.is_ascii_alphanumeric())
        && value.bytes().any(|b| b.is_ascii_digit())
}

pub(super) fn extract(subject: &str, body: &str) -> Option<String> {
    let body = plain_text(body);
    for text in [body.as_str(), subject] {
        for marker in MARKER.find_iter(text) {
            let tail: String = text[marker.end()..]
                .chars()
                .take(160)
                .take_while(|c| {
                    !matches!(
                        c,
                        '.' | '!' | '?' | ';' | ',' | '。' | '！' | '？' | '；' | '，'
                    )
                })
                .collect();
            if let Some(token) = TOKEN
                .find_iter(&tail)
                .find(|token| valid_token(token.as_str()))
            {
                // Expiry notices are not code values. Do not walk into a later order/date.
                let prefix = tail[..token.start()].to_lowercase();
                if [
                    "expire",
                    "valid for",
                    "valid until",
                    "有效期",
                    "过期",
                    "到期",
                ]
                .iter()
                .any(|word| prefix.contains(word))
                {
                    continue;
                }
                return Some(token.as_str().to_owned());
            }
        }
    }
    // Some services send only the code in the subject or message body.
    [body.trim(), subject.trim()]
        .into_iter()
        .find(|value| valid_token(value))
        .map(str::to_owned)
}

impl CodeStrategy for Generic {
    fn matches(&self, _: &Message<'_>) -> bool {
        true
    }
    fn extract(&self, message: &Message<'_>) -> Option<String> {
        extract(message.subject, message.body)
    }
}
