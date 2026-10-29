//! Translation port only. No provider, network request or simulated translation is installed.
use serde::{Deserialize, Serialize};
use std::{future::Future, pin::Pin};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Translation {
    pub original: String,
    pub translated: String,
    pub source_language: Option<String>,
    pub target_language: String,
}
pub struct TranslationRequest<'a> {
    pub text: &'a str,
    pub source_language: Option<&'a str>,
    pub target_language: &'a str,
}
/// Future providers are invoked by tasks, never by rendering code.
pub trait TranslationProvider: Send + Sync {
    fn translate<'a>(
        &'a self,
        request: TranslationRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<Translation, String>> + Send + 'a>>;
}
pub fn language_name(code: &str) -> &str {
    match code {
        "zh" | "zh-CN" => "中文",
        "en" => "English",
        "fr" => "Français",
        "es" => "Español",
        "de" => "Deutsch",
        "it" => "Italiano",
        "pt" => "Português",
        "nl" => "Nederlands",
        other => other,
    }
}
pub(super) fn country_language(code: &str) -> Option<&'static str> {
    match code {
        "ES" => Some("es"),
        "FR" => Some("fr"),
        "DE" | "AT" => Some("de"),
        "IT" => Some("it"),
        "PT" => Some("pt"),
        "NL" => Some("nl"),
        "GB" | "IE" | "US" => Some("en"),
        _ => None,
    }
}
