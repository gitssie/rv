//! Microsoft OAuth transport. Graph and IMAP implement the same read-only mailbox contract.
use super::{CodeMailbox, Credentials, MailboxProvider, message};
use chrono::{DateTime, Utc};
use futures_util::{TryStreamExt, future::BoxFuture};
use mailparse::MailHeaderMap;
use serde_json::Value;
use std::{collections::HashSet, sync::Arc, time::Duration};
use tokio::net::TcpStream;
use zeroize::Zeroizing;

pub(super) struct Microsoft;
impl MailboxProvider for Microsoft {
    fn validate(&self, credentials: &Credentials) -> Result<(), String> {
        uuid::Uuid::parse_str(&credentials.client_id)
            .map(|_| ())
            .map_err(|_| "Microsoft OAuth 客户端ID格式错误".into())
    }
    fn prepare(
        &self,
        credentials: Credentials,
    ) -> BoxFuture<'_, Result<Box<dyn CodeMailbox>, String>> {
        Box::pin(async move {
            let http = reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| "无法初始化邮箱连接")?;
            // Tokens cannot gain scopes the application/user has not already authorized.
            if let Ok(token) = refresh(
                &http,
                &credentials,
                "https://graph.microsoft.com/Mail.Read https://graph.microsoft.com/User.Read",
            )
            .await
                && let Ok(reader) =
                    Graph::prepare(http.clone(), token, credentials.email.clone()).await
            {
                return Ok(Box::new(reader) as Box<dyn CodeMailbox>);
            }
            let token = refresh(
                &http,
                &credentials,
                "https://outlook.office.com/IMAP.AccessAsUser.All",
            )
            .await?;
            let reader = Imap::prepare(token, credentials.email).await?;
            Ok(Box::new(reader) as Box<dyn CodeMailbox>)
        })
    }
}
async fn bounded_json(mut response: reqwest::Response) -> Result<Value, String> {
    if !response.status().is_success() {
        return Err("邮箱服务拒绝请求；请检查 OAuth 授权及读取邮件权限".into());
    }
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response.chunk().await.map_err(|_| "邮箱回复读取失败")? {
        if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err("邮箱回复超过限制".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "邮箱回复格式错误".into())
}
async fn refresh(
    http: &reqwest::Client,
    credentials: &Credentials,
    scope: &str,
) -> Result<Zeroizing<String>, String> {
    refresh_at(
        http,
        credentials,
        scope,
        "https://login.microsoftonline.com/consumers/oauth2/v2.0/token",
    )
    .await
}
async fn refresh_at(
    http: &reqwest::Client,
    credentials: &Credentials,
    scope: &str,
    endpoint: &str,
) -> Result<Zeroizing<String>, String> {
    let response = http
        .post(endpoint)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", credentials.client_id.as_str()),
            ("refresh_token", credentials.refresh_token.as_str()),
            ("scope", scope),
        ])
        .send()
        .await
        .map_err(|_| "Microsoft OAuth 连接失败")?;
    let mut value = bounded_json(response).await?;
    let token = value
        .get_mut("access_token")
        .and_then(|v| v.as_str().map(str::to_owned))
        .filter(|s| !s.is_empty())
        .ok_or("OAuth 回复缺少访问令牌")?;
    Ok(Zeroizing::new(token))
}
struct Graph {
    http: reqwest::Client,
    token: Zeroizing<String>,
    email: String,
    since: DateTime<Utc>,
    baseline: HashSet<String>,
}
impl Graph {
    async fn prepare(
        http: reqwest::Client,
        token: Zeroizing<String>,
        email: String,
    ) -> Result<Self, String> {
        let profile = bounded_json(
            http.get("https://graph.microsoft.com/v1.0/me?$select=mail,userPrincipalName")
                .bearer_auth(token.as_str())
                .send()
                .await
                .map_err(|_| "邮箱账户查询失败")?,
        )
        .await?;
        if !["mail", "userPrincipalName"].iter().any(|k| {
            profile[*k]
                .as_str()
                .is_some_and(|s| s.eq_ignore_ascii_case(&email))
        }) {
            return Err("OAuth 授权账户与填写的邮箱不一致".into());
        }
        let mut reader = Self {
            http,
            token,
            email,
            since: Utc::now(),
            baseline: HashSet::new(),
        };
        let messages = reader.messages().await?;
        reader.baseline = messages
            .iter()
            .filter_map(|m| m["id"].as_str().map(str::to_owned))
            .collect();
        reader.since = Utc::now();
        Ok(reader)
    }
    async fn messages(&self) -> Result<Vec<Value>, String> {
        let response = self
            .http
            .get("https://graph.microsoft.com/v1.0/me/messages")
            .query(&[
                ("$top", "1"),
                ("$orderby", "receivedDateTime desc"),
                (
                    "$select",
                    "id,from,toRecipients,receivedDateTime,subject,body",
                ),
            ])
            .header("Prefer", "outlook.body-content-type=\"text\"")
            .bearer_auth(self.token.as_str())
            .send()
            .await
            .map_err(|_| "邮箱读取失败")?;
        bounded_json(response).await?["value"]
            .as_array()
            .cloned()
            .ok_or("邮箱回复缺少邮件列表".into())
    }
}
fn graph_code(
    m: &Value,
    email: &str,
    since: DateTime<Utc>,
    baseline: &HashSet<String>,
) -> Result<Option<String>, String> {
    graph_code_with_sender(m, email, since, baseline, true)
}
fn graph_code_with_sender(
    m: &Value,
    email: &str,
    since: DateTime<Utc>,
    baseline: &HashSet<String>,
    apple_only: bool,
) -> Result<Option<String>, String> {
    let candidate = (|| {
        let id = m["id"].as_str()?;
        let received = DateTime::parse_from_rfc3339(m["receivedDateTime"].as_str()?).ok()?;
        // Graph dates can be truncated to seconds. Baseline IDs exclude existing mail;
        // this short window tolerates timestamp precision and modest server clock skew.
        if baseline.contains(id)
            || received < since - chrono::Duration::seconds(5)
            || received > Utc::now() + chrono::Duration::seconds(30)
        {
            return None;
        }
        if (apple_only && !message::apple_sender(m["from"]["emailAddress"]["address"].as_str()?))
            || !m["toRecipients"].as_array()?.iter().any(|r| {
                r["emailAddress"]["address"]
                    .as_str()
                    .is_some_and(|s| s.eq_ignore_ascii_case(email))
            })
        {
            return None;
        }
        Some((m["subject"].as_str()?, m["body"]["content"].as_str()?))
    })();
    match candidate {
        Some((subject, body)) => message::code(subject, body),
        None => Ok(None),
    }
}
fn graph_first_code(
    messages: &[Value],
    email: &str,
    since: DateTime<Utc>,
    baseline: &HashSet<String>,
) -> Result<Option<String>, String> {
    // The request is newest-first with $top=1. Never search older messages.
    match messages.first() {
        Some(message) => graph_code(message, email, since, baseline),
        None => Ok(None),
    }
}
impl CodeMailbox for Graph {
    fn poll_latest_any(&mut self) -> BoxFuture<'_, Result<Option<String>, String>> {
        Box::pin(async move {
            match self.messages().await?.first() {
                Some(message) => graph_code_with_sender(
                    message,
                    &self.email,
                    chrono::DateTime::<Utc>::UNIX_EPOCH,
                    &HashSet::new(),
                    false,
                ),
                None => Ok(None),
            }
        })
    }

    fn poll_latest(&mut self) -> BoxFuture<'_, Result<Option<String>, String>> {
        Box::pin(async move {
            graph_first_code(
                &self.messages().await?,
                &self.email,
                chrono::DateTime::<Utc>::UNIX_EPOCH,
                &HashSet::new(),
            )
        })
    }
    fn arm(&mut self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.baseline = self
                .messages()
                .await?
                .iter()
                .filter_map(|m| m["id"].as_str().map(str::to_owned))
                .collect();
            self.since = Utc::now();
            Ok(())
        })
    }
    fn poll(&mut self) -> BoxFuture<'_, Result<Option<String>, String>> {
        Box::pin(async move {
            graph_first_code(
                &self.messages().await?,
                &self.email,
                self.since,
                &self.baseline,
            )
        })
    }
}

type Session = async_imap::Session<tokio_rustls::client::TlsStream<TcpStream>>;
struct Imap {
    session: Session,
    email: String,
    uid_next: u32,
    validity: u32,
}
struct XOAuth {
    payload: Zeroizing<String>,
    sent: bool,
}
impl async_imap::Authenticator for XOAuth {
    type Response = Vec<u8>;
    fn process(&mut self, _: &[u8]) -> Self::Response {
        // A further continuation indicates authentication rejection; don't resend the secret.
        if self.sent {
            Vec::new()
        } else {
            self.sent = true;
            self.payload.as_bytes().to_vec()
        }
    }
}
impl Imap {
    async fn prepare(token: Zeroizing<String>, email: String) -> Result<Self, String> {
        let host = "outlook.office365.com";
        let stream = TcpStream::connect((host, 993))
            .await
            .map_err(|_| "Microsoft IMAP 连接失败")?;
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let tls = tokio_rustls::TlsConnector::from(Arc::new(config))
            .connect(
                rustls::pki_types::ServerName::try_from(host).unwrap(),
                stream,
            )
            .await
            .map_err(|_| "邮箱 TLS 验证失败")?;
        let mut client = async_imap::Client::new(tls);
        client
            .read_response()
            .await
            .map_err(|_| "邮箱握手失败")?
            .ok_or("邮箱关闭连接")?;
        let auth = XOAuth {
            payload: Zeroizing::new(format!(
                "user={email}\x01auth=Bearer {}\x01\x01",
                token.as_str()
            )),
            sent: false,
        };
        let mut session = client
            .authenticate("XOAUTH2", auth)
            .await
            .map_err(|_| "邮箱 OAuth 登录失败；请检查令牌和 IMAP 权限")?;
        let inbox = session
            .examine("INBOX")
            .await
            .map_err(|_| "无法读取收件箱")?;
        Ok(Self {
            session,
            email,
            uid_next: inbox.uid_next.ok_or("邮箱缺少新邮件标识")?,
            validity: inbox.uid_validity.ok_or("邮箱缺少有效性标识")?,
        })
    }
}
fn addresses(headers: &[mailparse::MailHeader<'_>], name: &str) -> Vec<String> {
    headers
        .get_first_header(name)
        .and_then(|h| mailparse::addrparse_header(h).ok())
        .map(|list| {
            list.iter()
                .flat_map(|addr| match addr {
                    mailparse::MailAddr::Single(s) => vec![s.addr.clone()],
                    mailparse::MailAddr::Group(g) => {
                        g.addrs.iter().map(|s| s.addr.clone()).collect()
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}
fn mime_code(bytes: &[u8], email: &str) -> Result<Option<String>, String> {
    mime_code_with_sender(bytes, email, true)
}
fn mime_code_with_sender(
    bytes: &[u8],
    email: &str,
    apple_only: bool,
) -> Result<Option<String>, String> {
    let Ok(mail) = mailparse::parse_mail(bytes) else {
        return Ok(None);
    };
    let from = addresses(&mail.headers, "From");
    if from.len() != 1
        || (apple_only && !message::apple_sender(&from[0]))
        || !addresses(&mail.headers, "To")
            .iter()
            .any(|s| s.eq_ignore_ascii_case(email))
    {
        return Ok(None);
    }
    let subject = mail.headers.get_first_value("Subject").unwrap_or_default();
    for part in mail.parts().filter(|p| {
        matches!(p.ctype.mimetype.as_str(), "text/plain" | "text/html")
            && p.get_content_disposition().disposition != mailparse::DispositionType::Attachment
    }) {
        if let Ok(body) = part.get_body()
            && let Some(code) = message::code(&subject, &body)?
        {
            return Ok(Some(code));
        }
    }
    Ok(None)
}
fn imap_first_code(
    messages: Vec<(u32, Zeroizing<Vec<u8>>)>,
    email: &str,
) -> Result<Option<String>, String> {
    // UID FETCH * requests only the newest inbox message.
    match messages.first() {
        Some((_, body)) => mime_code(body, email),
        None => Ok(None),
    }
}
impl CodeMailbox for Imap {
    fn poll_latest(&mut self) -> BoxFuture<'_, Result<Option<String>, String>> {
        Box::pin(async move {
            let previous = self.uid_next;
            self.uid_next = 1;
            let code = self.poll().await;
            self.uid_next = previous;
            code
        })
    }
    fn arm(&mut self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let inbox = self
                .session
                .examine("INBOX")
                .await
                .map_err(|_| "无法读取收件箱；尚未添加邮箱")?;
            self.uid_next = inbox.uid_next.ok_or("邮箱缺少新邮件标识")?;
            self.validity = inbox.uid_validity.ok_or("邮箱缺少有效性标识")?;
            Ok(())
        })
    }
    fn poll(&mut self) -> BoxFuture<'_, Result<Option<String>, String>> {
        self.poll_with_sender(true)
    }
    fn poll_latest_any(&mut self) -> BoxFuture<'_, Result<Option<String>, String>> {
        Box::pin(async move {
            let previous = self.uid_next;
            self.uid_next = 1;
            let result = self.poll_with_sender(false).await;
            self.uid_next = previous;
            result
        })
    }
}
impl Imap {
    fn poll_with_sender(
        &mut self,
        apple_only: bool,
    ) -> BoxFuture<'_, Result<Option<String>, String>> {
        Box::pin(async move {
            let inbox = self
                .session
                .examine("INBOX")
                .await
                .map_err(|_| "邮箱连接中断，可手动输入验证码")?;
            if inbox.uid_validity != Some(self.validity) {
                return Err("收件箱标识已改变，请手动验证".into());
            }
            let upper = inbox
                .uid_next
                .ok_or("邮箱缺少新邮件标识")?
                .saturating_sub(1);
            if upper < self.uid_next {
                return Ok(None);
            }
            // PEEK and read-only EXAMINE preserve unread status; a partial fetch bounds each message.
            let mut messages = self
                .session
                .uid_fetch("*", "(UID RFC822.SIZE BODY.PEEK[]<0.262144>)")
                .await
                .map_err(|_| "邮箱读取失败")?;
            let mut candidates = Vec::new();
            while let Some(m) = messages.try_next().await.map_err(|_| "邮件读取失败")? {
                if let Some(uid) = m.uid
                    && uid >= self.uid_next
                    && uid <= upper
                    && m.size.is_some_and(|size| size <= 262144)
                    && let Some(body) = m.body()
                {
                    candidates.push((uid, Zeroizing::new(body.to_vec())));
                }
            }
            if apple_only {
                imap_first_code(candidates, &self.email)
            } else {
                match candidates.first() {
                    Some((_, body)) => mime_code_with_sender(body, &self.email, false),
                    None => Ok(None),
                }
            }
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_reads_accept_other_senders_but_apple_binding_does_not() {
        let email = "demo@outlook.com";
        let now = Utc::now();
        let mail = serde_json::json!({"id":"latest","receivedDateTime":now.to_rfc3339(),
            "from":{"emailAddress":{"address":"verify@example.com"}},
            "toRecipients":[{"emailAddress":{"address":email}}],"subject":"Verify", "body":{"content":"012345"}});
        assert_eq!(graph_code(&mail, email, now, &HashSet::new()), Ok(None));
        assert_eq!(
            graph_code_with_sender(&mail, email, now, &HashSet::new(), false),
            Ok(Some("012345".into()))
        );
        let mime = b"From: verify@example.com\r\nTo: demo@outlook.com\r\nSubject: Verify\r\nContent-Type: text/plain\r\n\r\n012345";
        assert_eq!(mime_code(mime, email), Ok(None));
        assert_eq!(
            mime_code_with_sender(mime, email, false),
            Ok(Some("012345".into()))
        );
    }

    #[test]
    fn graph_reads_first_message_without_searching_older_mail() {
        let now = Utc::now();
        let make = |id: &str, body: &str| serde_json::json!({"id":id,"receivedDateTime":now.to_rfc3339(),"from":{"emailAddress":{"address":"appleid@apple.com"}},"toRecipients":[{"emailAddress":{"address":"demo@outlook.com"}}],"subject":"Apple","body":{"content":body}});
        let messages = vec![make("newest", "012345"), make("older", "654321 or 123456")];
        assert_eq!(
            graph_first_code(&messages, "demo@outlook.com", now, &HashSet::new()),
            Ok(Some("012345".into()))
        );
        assert_eq!(
            graph_first_code(
                &messages,
                "demo@outlook.com",
                now,
                &HashSet::from(["newest".into()])
            ),
            Ok(None)
        );
        let mut unrelated = make("unrelated", "999999");
        unrelated["from"]["emailAddress"]["address"] = serde_json::json!("a@example.com");
        assert_eq!(
            graph_first_code(
                &[unrelated, make("older", "012345")],
                "demo@outlook.com",
                now,
                &HashSet::new()
            ),
            Ok(None)
        );
        assert_eq!(
            graph_first_code(
                &[make("empty", "No code"), make("older", "012345")],
                "demo@outlook.com",
                now,
                &HashSet::new()
            ),
            Ok(None)
        );
        assert_eq!(
            graph_first_code(
                &[make("newest", "111111 or 222222"), make("older", "012345")],
                "demo@outlook.com",
                now,
                &HashSet::new()
            ),
            Ok(Some("111111".into()))
        );
    }
    #[test]
    fn imap_reads_first_message_without_searching_older_mail() {
        let make = |uid, recipient: &str, code: &str| {
            (uid, Zeroizing::new(format!("From: Apple <appleid@apple.com>\r\nTo: {recipient}\r\nSubject: Apple\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{code}").into_bytes()))
        };
        assert_eq!(
            imap_first_code(
                vec![
                    make(12, "demo@outlook.com", "012345"),
                    make(11, "demo@outlook.com", "654321 or 123456")
                ],
                "demo@outlook.com"
            ),
            Ok(Some("012345".into()))
        );
        assert_eq!(
            imap_first_code(
                vec![
                    make(12, "other@outlook.com", "999999"),
                    make(11, "demo@outlook.com", "012345")
                ],
                "demo@outlook.com"
            ),
            Ok(None)
        );
        assert_eq!(
            imap_first_code(
                vec![
                    make(12, "demo@outlook.com", "No code"),
                    make(11, "demo@outlook.com", "012345")
                ],
                "demo@outlook.com"
            ),
            Ok(None)
        );
    }
    #[tokio::test]
    async fn refresh_uses_oauth_form_and_never_sends_mailbox_password() {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = BufReader::new(socket);
            let mut length = 0;
            loop {
                let mut line = String::new();
                socket.read_line(&mut line).await.unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_lowercase().strip_prefix("content-length: ") {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            socket.read_exact(&mut body).await.unwrap();
            let fields: std::collections::HashMap<_, _> = reqwest::Url::parse(&format!(
                "http://test/?{}",
                String::from_utf8(body).unwrap()
            ))
            .unwrap()
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
            assert_eq!(fields["grant_type"], "refresh_token");
            assert_eq!(fields["refresh_token"], "test-refresh");
            assert_eq!(fields["scope"], "test-scope");
            assert!(!fields.contains_key("password"));
            assert_eq!(fields.len(), 4);
            let body = r#"{"access_token":"test-access"}"#;
            socket
                .get_mut()
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let credentials = Credentials {
            email: "demo@outlook.com".into(),
            client_id: "00000000-0000-4000-8000-000000000001".into(),
            refresh_token: Zeroizing::new("test-refresh".into()),
        };
        let token = refresh_at(
            &reqwest::Client::new(),
            &credentials,
            "test-scope",
            &endpoint,
        )
        .await
        .unwrap();
        assert_eq!(token.as_str(), "test-access");
        server.await.unwrap();
    }
    #[test]
    fn graph_requires_new_recipient_matched_apple_mail() {
        let now = Utc::now();
        let mut m = serde_json::json!({"id":"new","receivedDateTime":now.to_rfc3339(),"from":{"emailAddress":{"address":"appleid@id.apple.com"}},"toRecipients":[{"emailAddress":{"address":"demo@outlook.com"}}],"subject":"Verify your email","body":{"content":"Your verification code is 012345"}});
        assert_eq!(
            graph_code(&m, "demo@outlook.com", now, &HashSet::new()),
            Ok(Some("012345".into()))
        );
        assert_eq!(
            graph_code(&m, "other@outlook.com", now, &HashSet::new()),
            Ok(None)
        );
        assert_eq!(
            graph_code(&m, "demo@outlook.com", now, &HashSet::from(["new".into()])),
            Ok(None)
        );
        m["receivedDateTime"] =
            serde_json::json!(now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        assert_eq!(
            graph_code(&m, "demo@outlook.com", now, &HashSet::new()),
            Ok(Some("012345".into()))
        );
        m["body"]["content"] = serde_json::json!("Verify your email: 012345 or 123456");
        assert_eq!(
            graph_code(&m, "demo@outlook.com", now, &HashSet::new()),
            Ok(Some("012345".into()))
        );
        m["receivedDateTime"] =
            serde_json::json!((now - chrono::Duration::minutes(1)).to_rfc3339());
        assert_eq!(
            graph_code(&m, "demo@outlook.com", now, &HashSet::new()),
            Ok(None)
        );
    }
    #[test]
    fn mime_decodes_transfer_encoding_and_checks_addresses() {
        let mail = b"From: Apple <appleid@id.apple.com>\r\nTo: demo@outlook.com\r\nSubject: Verify email\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\nVmVyaWZpY2F0aW9uIGNvZGU6IDAxMjM0NQ==";
        assert_eq!(
            mime_code(mail, "demo@outlook.com"),
            Ok(Some("012345".into()))
        );
        assert_eq!(mime_code(mail, "other@outlook.com"), Ok(None));
    }
    #[test]
    fn mime_uses_first_code_without_merging_alternative_parts() {
        let mail = b"From: Apple <appleid@apple.com>\r\nTo: demo@outlook.com\r\nSubject: Apple\r\nContent-Type: multipart/alternative; boundary=parts\r\n\r\n--parts\r\nContent-Type: text/plain\r\n\r\n012345 then 654321\r\n--parts\r\nContent-Type: text/html\r\n\r\n<b>987654</b>\r\n--parts--\r\n";
        assert_eq!(
            mime_code(mail, "demo@outlook.com"),
            Ok(Some("012345".into()))
        );
    }
}
