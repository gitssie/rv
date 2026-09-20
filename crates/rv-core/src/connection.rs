use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable identity for a saved connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConnectionId(pub Uuid);

impl ConnectionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ConnectionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// How the viewer should negotiate RFB security / VeNCrypt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum EncryptionMode {
    /// Prefer plain authentication, accepting VeNCrypt when required by the server.
    #[default]
    LetServerChoose,
    /// Try VeNCrypt first, then fall back to None / VNC-Auth / ARD.
    PreferOn,
    /// Require VeNCrypt TLS.
    Always,
    /// Never use VeNCrypt.
    Off,
}

impl EncryptionMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::LetServerChoose => "Let server choose",
            Self::PreferOn => "Prefer on",
            Self::Always => "Always on",
            Self::Off => "Off",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::LetServerChoose => Self::PreferOn,
            Self::PreferOn => Self::Always,
            Self::Always => Self::Off,
            Self::Off => Self::LetServerChoose,
        }
    }
}

/// Advertised encoding order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum QualityPreset {
    #[default]
    Auto,
    Best,
    Fast,
}

impl QualityPreset {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automatic",
            Self::Best => "Best quality",
            Self::Fast => "Fast",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::Auto => Self::Best,
            Self::Best => Self::Fast,
            Self::Fast => Self::Auto,
        }
    }
}

/// Clipboard text encoding used for this connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ClipboardMode {
    /// UltraVNC Extended Clipboard with UTF-8 text.
    #[default]
    Utf8,
    /// Classic RFB ClientCutText / ServerCutText with ISO-8859-1 text.
    Latin1,
}

impl ClipboardMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Latin1 => "Latin-1",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::Utf8 => Self::Latin1,
            Self::Latin1 => Self::Utf8,
        }
    }
}

/// How the native pointer is shown over a remote session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LocalCursorMode {
    /// Prefer a visible native pointer when server cursor support is unknown.
    #[default]
    Automatic,
    /// Always keep the native pointer visible.
    Show,
    /// Hide the native pointer while it is over the active remote desktop.
    Hide,
}

impl LocalCursorMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Automatic => "Automatic",
            Self::Show => "Always show",
            Self::Hide => "Hide over desktop",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::Automatic => Self::Show,
            Self::Show => Self::Hide,
            Self::Hide => Self::Automatic,
        }
    }
}

/// How the remote desktop is fitted into the session window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ScaleMode {
    Fit,
    #[default]
    Actual,
    Stretch,
}

impl ScaleMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Fit => "Scale to fit",
            Self::Actual => "100%",
            Self::Stretch => "Stretch",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::Fit => Self::Actual,
            Self::Actual => Self::Stretch,
            Self::Stretch => Self::Fit,
        }
    }
}

/// A saved address-book entry. Passwords live in the OS keychain, never here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Connection {
    pub id: ConnectionId,
    pub name: String,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub remember_password: bool,
    #[serde(default)]
    pub encryption: EncryptionMode,
    #[serde(default)]
    pub quality: QualityPreset,
    #[serde(default)]
    pub clipboard: ClipboardMode,
    #[serde(default)]
    pub local_cursor: LocalCursorMode,
    #[serde(default)]
    pub view_only: bool,
    #[serde(default = "default_shared")]
    pub shared: bool,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub last_connected: Option<i64>,
}

fn default_shared() -> bool {
    true
}

impl Connection {
    pub fn new(name: impl Into<String>, host: impl Into<String>, port: u16) -> Self {
        let host = host.into();
        let name = {
            let n = name.into();
            if n.trim().is_empty() {
                format!("{host}:{port}")
            } else {
                n
            }
        };
        Self {
            id: ConnectionId::new(),
            name,
            host,
            port,
            username: None,
            remember_password: false,
            encryption: EncryptionMode::default(),
            quality: QualityPreset::default(),
            clipboard: ClipboardMode::default(),
            local_cursor: LocalCursorMode::default(),
            view_only: false,
            shared: true,
            labels: Vec::new(),
            last_connected: None,
        }
    }

    pub fn server_display(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Parameters for an in-flight connect attempt (saved or ad-hoc).
#[derive(Debug, Clone)]
pub struct ConnectRequest {
    pub connection_id: Option<ConnectionId>,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub encryption: EncryptionMode,
    pub quality: QualityPreset,
    pub clipboard: ClipboardMode,
    pub local_cursor: LocalCursorMode,
    pub view_only: bool,
    pub shared: bool,
}

impl ConnectRequest {
    pub fn from_connection(conn: &Connection, password: Option<String>) -> Self {
        Self {
            connection_id: Some(conn.id),
            name: conn.name.clone(),
            host: conn.host.clone(),
            port: conn.port,
            username: conn.username.clone(),
            password,
            encryption: conn.encryption,
            quality: conn.quality,
            clipboard: conn.clipboard,
            local_cursor: conn.local_cursor,
            view_only: conn.view_only,
            shared: conn.shared,
        }
    }

    pub fn display_name(&self) -> &str {
        if self.name.trim().is_empty() {
            &self.host
        } else {
            &self.name
        }
    }
}

/// Default RFB port; `host:N` with `N < 100` is display `N` on top of it.
pub const DEFAULT_PORT: u16 = 5900;

/// Parse a VNC server address the way classic viewers do.
///
/// * `host` → port 5900
/// * `host:N` with `N < 100` → display number, port `5900 + N`
/// * `host:port` (≥ 100) → that port
/// * `host::port` → that port, even if below 100
/// * `[ipv6]`, `[ipv6]:N`, `[ipv6]::port`, or a bare IPv6 literal
pub fn parse_server(input: &str) -> Result<(String, u16), String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Enter a VNC server host".into());
    }
    if let Some(rest) = trimmed.strip_prefix('[') {
        let (host, tail) = rest
            .split_once(']')
            .ok_or_else(|| "Invalid IPv6 address".to_string())?;
        if host.is_empty() {
            return Err("Invalid IPv6 address".into());
        }
        if tail.is_empty() {
            return Ok((host.to_string(), DEFAULT_PORT));
        }
        let port = tail
            .strip_prefix(':')
            .ok_or_else(|| "Invalid IPv6 address".to_string())
            .and_then(parse_port_suffix)?;
        return Ok((host.to_string(), port));
    }
    if let Some((host, suffix)) = trimmed.split_once(':') {
        if suffix.contains(':') && !suffix.starts_with(':') {
            // Bare IPv6 literal such as `2001:db8::1`.
            return Ok((trimmed.to_string(), DEFAULT_PORT));
        }
        if host.is_empty() {
            return Err("Enter a VNC server host".into());
        }
        return Ok((host.to_string(), parse_port_suffix(suffix)?));
    }
    Ok((trimmed.to_string(), DEFAULT_PORT))
}

/// `suffix` is everything after the first `:`: `N`, `port`, or `:port`.
fn parse_port_suffix(suffix: &str) -> Result<u16, String> {
    if let Some(explicit) = suffix.strip_prefix(':') {
        return parse_port(explicit);
    }
    let n = parse_port(suffix)?;
    if n < 100 { Ok(DEFAULT_PORT + n) } else { Ok(n) }
}

fn parse_port(s: &str) -> Result<u16, String> {
    const MSG: &str = "Port must be a number from 0 to 65535 (or a display number below 100)";
    s.parse::<u16>().map_err(|_| MSG.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_host_only() {
        assert_eq!(parse_server("10.0.0.8").unwrap(), ("10.0.0.8".into(), 5900));
    }

    #[test]
    fn parse_host_port() {
        assert_eq!(
            parse_server("pi.local:5901").unwrap(),
            ("pi.local".into(), 5901)
        );
    }

    #[test]
    fn parse_display_number() {
        assert_eq!(
            parse_server("pi.local:1").unwrap(),
            ("pi.local".into(), 5901)
        );
        assert_eq!(
            parse_server("pi.local:0").unwrap(),
            ("pi.local".into(), 5900)
        );
        assert_eq!(
            parse_server("pi.local:99").unwrap(),
            ("pi.local".into(), 5999)
        );
        assert_eq!(
            parse_server("pi.local:100").unwrap(),
            ("pi.local".into(), 100)
        );
    }

    #[test]
    fn parse_explicit_port() {
        assert_eq!(parse_server("pi.local::1").unwrap(), ("pi.local".into(), 1));
        assert_eq!(
            parse_server("pi.local::5900").unwrap(),
            ("pi.local".into(), 5900)
        );
    }

    #[test]
    fn parse_ipv6() {
        assert_eq!(
            parse_server("[2001:db8::1]:5902").unwrap(),
            ("2001:db8::1".into(), 5902)
        );
        assert_eq!(parse_server("[::1]").unwrap(), ("::1".into(), 5900));
        assert_eq!(parse_server("[::1]:2").unwrap(), ("::1".into(), 5902));
        assert_eq!(parse_server("[::1]::80").unwrap(), ("::1".into(), 80));
        assert_eq!(
            parse_server("2001:db8::1").unwrap(),
            ("2001:db8::1".into(), 5900)
        );
        assert!(parse_server("[::1").is_err());
        assert!(parse_server("[]:5900").is_err());
    }

    #[test]
    fn reject_bad_input() {
        assert!(parse_server("  ").is_err());
        assert!(parse_server(":5900").is_err());
        assert!(parse_server("host:").is_err());
        assert!(parse_server("host:abc").is_err());
        assert!(parse_server("host:70000").is_err());
    }

    #[test]
    fn connection_round_trip() {
        let mut c = Connection::new("office", "10.0.0.8", 5900);
        c.clipboard = ClipboardMode::Latin1;
        c.local_cursor = LocalCursorMode::Hide;
        let json = serde_json::to_string(&c).unwrap();
        let back: Connection = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "office");
        assert_eq!(back.port, 5900);
        assert!(back.shared);
        assert_eq!(back.clipboard, ClipboardMode::Latin1);
        assert_eq!(back.local_cursor, LocalCursorMode::Hide);
        assert_eq!(
            ConnectRequest::from_connection(&back, None).clipboard,
            ClipboardMode::Latin1
        );
        assert_eq!(
            ConnectRequest::from_connection(&back, None).local_cursor,
            LocalCursorMode::Hide
        );
    }

    #[test]
    fn legacy_connection_defaults_to_automatic_local_cursor() {
        let c = Connection::new("office", "10.0.0.8", 5900);
        let mut json = serde_json::to_value(&c).unwrap();
        json.as_object_mut().unwrap().remove("local_cursor");
        let back: Connection = serde_json::from_value(json).unwrap();
        assert_eq!(back.local_cursor, LocalCursorMode::Automatic);
    }

    #[test]
    fn legacy_connection_defaults_to_utf8_clipboard() {
        let c = Connection::new("office", "10.0.0.8", 5900);
        let mut json = serde_json::to_value(&c).unwrap();
        json.as_object_mut().unwrap().remove("clipboard");
        let back: Connection = serde_json::from_value(json).unwrap();
        assert_eq!(back.clipboard, ClipboardMode::Utf8);
    }
}
