//! Provider routing and bounded verification-code retrieval. No credentials are persisted.
mod message;
mod microsoft;
use futures_util::future::BoxFuture;
use serde_json::{Map, Value};
use std::time::Duration;
use zeroize::Zeroizing;

pub(crate) struct Credentials {
    pub email: String,
    pub client_id: String,
    pub refresh_token: Zeroizing<String>,
}
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MailboxCredentials { contents: [redacted] }")
    }
}
impl Credentials {
    pub fn take_from(params: &mut Map<String, Value>) -> Result<Option<Self>, String> {
        let password = Zeroizing::new(
            params
                .remove("password")
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
        );
        let client = params
            .remove("client_id")
            .and_then(|v| v.as_str().map(str::to_owned));
        let token = params
            .remove("refresh_token")
            .and_then(|v| v.as_str().map(str::to_owned));
        if password.is_empty() && client.is_none() && token.is_none() {
            return Ok(None);
        }
        let (Some(client_id), Some(token)) = (client, token) else {
            return Err("邮箱资料须为：邮箱----密码----客户端ID----刷新令牌".into());
        };
        let refresh_token = Zeroizing::new(token);
        if password.is_empty() || client_id.trim().is_empty() || refresh_token.is_empty() {
            return Err("邮箱 OAuth 资料不完整或客户端ID格式错误".into());
        }
        let email = params["email"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_string();
        let credentials = Self {
            email,
            client_id,
            refresh_token,
        };
        provider_for(&credentials.email)?.validate(&credentials)?;
        params.insert("email".into(), Value::String(credentials.email.clone()));
        Ok(Some(credentials))
    }
}

/// Providers must establish mailbox identity and a pre-send baseline without modifying mail.
pub(super) trait MailboxProvider: Sync {
    fn validate(&self, credentials: &Credentials) -> Result<(), String>;
    fn prepare(
        &self,
        credentials: Credentials,
    ) -> BoxFuture<'_, Result<Box<dyn CodeMailbox>, String>>;
}
/// Read the first code for the selected recipient. Never mark mail as read.
pub(super) trait CodeMailbox: Send {
    /// Refresh the baseline immediately before the phone sends its verification email.
    fn arm(&mut self) -> BoxFuture<'_, Result<(), String>>;
    fn poll(&mut self) -> BoxFuture<'_, Result<Option<String>, String>>;
    // Explicit user refresh reads the latest mail, including a previously fetched code.
    fn poll_latest(&mut self) -> BoxFuture<'_, Result<Option<String>, String>> {
        self.poll()
    }
}
fn provider_for(email: &str) -> Result<&'static dyn MailboxProvider, String> {
    let (local, domain) = email.rsplit_once('@').ok_or("邮箱地址格式错误")?;
    if local.is_empty() || local.contains(['@', '\r', '\n', '\x01']) {
        return Err("邮箱地址格式错误".into());
    }
    match domain.to_ascii_lowercase().as_str() {
        "outlook.com" | "hotmail.com" | "live.com" | "msn.com" | "outlook.cn" | "hotmail.co.uk"
        | "hotmail.fr" | "hotmail.de" | "live.co.uk" => Ok(&microsoft::Microsoft),
        _ => Err("此邮箱服务尚未接入自动取码；可只填写邮箱并手动验证".into()),
    }
}
pub(super) struct Router;
impl MailboxProvider for Router {
    fn validate(&self, credentials: &Credentials) -> Result<(), String> {
        provider_for(&credentials.email)?.validate(credentials)
    }
    fn prepare(
        &self,
        credentials: Credentials,
    ) -> BoxFuture<'_, Result<Box<dyn CodeMailbox>, String>> {
        Box::pin(async move { provider_for(&credentials.email)?.prepare(credentials).await })
    }
}
pub(crate) struct PreparedMailbox {
    reader: Box<dyn CodeMailbox>,
}
impl PreparedMailbox {
    pub async fn prepare(
        credentials: Credentials,
        provider: &dyn MailboxProvider,
    ) -> Result<Self, String> {
        tokio::time::timeout(Duration::from_secs(30), provider.prepare(credentials))
            .await
            .map_err(|_| "邮箱授权检查超时；尚未添加邮箱".to_string())?
            .map(|reader| Self { reader })
    }
    pub async fn arm(&mut self) -> Result<(), String> {
        tokio::time::timeout(Duration::from_secs(15), self.reader.arm())
            .await
            .map_err(|_| "邮箱检查超时；尚未添加邮箱".to_string())?
    }
    pub async fn read_latest(&mut self) -> Result<String, String> {
        tokio::time::timeout(Duration::from_secs(30), self.reader.poll_latest())
            .await
            .map_err(|_| "读取验证码超时，请稍后重新获取".to_string())?
            .and_then(|code| {
                code.ok_or_else(|| "最新一封邮件没有可用验证码，请稍后重新获取".into())
            })
    }
    pub async fn wait_code(&mut self) -> Result<String, String> {
        tokio::time::timeout(Duration::from_secs(150), async {
            // Apple has issued the challenge; allow initial email delivery to settle.
            tokio::time::sleep(Duration::from_secs(3)).await;
            loop {
                if let Some(code) = self.reader.poll().await? {
                    return Ok(code);
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        })
        .await
        .map_err(|_| "未收到新的邮箱验证码，可手动输入；请勿重复添加邮箱".to_string())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn first_poll_waits_for_mail_delivery() {
        struct Reader(tokio::time::Instant);
        impl CodeMailbox for Reader {
            fn arm(&mut self) -> BoxFuture<'_, Result<(), String>> {
                Box::pin(async { Ok(()) })
            }
            fn poll(&mut self) -> BoxFuture<'_, Result<Option<String>, String>> {
                Box::pin(async move {
                    assert!(self.0.elapsed() >= Duration::from_secs(3));
                    Ok(Some("012345".into()))
                })
            }
        }
        let mut mailbox = PreparedMailbox {
            reader: Box::new(Reader(tokio::time::Instant::now())),
        };
        assert_eq!(mailbox.wait_code().await.unwrap(), "012345");
    }
    #[test]
    fn routes_exact_domains_and_rejects_partial_credentials() {
        assert!(provider_for("a@HOTMAIL.COM").is_ok());
        assert!(provider_for("a@outlook.com.evil.test").is_err());
        assert!(provider_for("a@gmail.com").is_err());
        let mut p = serde_json::json!({"email":"a@outlook.com","password":"demo"})
            .as_object()
            .unwrap()
            .clone();
        assert!(Credentials::take_from(&mut p).is_err());
    }
}
