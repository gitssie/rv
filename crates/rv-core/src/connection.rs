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
    /// Prefer unencrypted None / VNC-Auth; VeNCrypt-only servers fail with a hint.
    #[default]
    LetServerChoose,
    /// Try VeNCrypt first, then fall back to None / VNC-Auth.
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

/// How the remote desktop is fitted into the session window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ScaleMode {
    #[default]
    Fit,
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
    pub password: Option<String>,
    pub encryption: EncryptionMode,
    pub quality: QualityPreset,
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
            password,
            encryption: conn.encryption,
            quality: conn.quality,
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

/// Parse `host`, `host:port`, or `[ipv6]:port`. Default port is 5900.
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
        let port = match tail.strip_prefix(':') {
            Some(p) if !p.is_empty() => parse_port(p)?,
            Some(_) => return Err("Invalid port".into()),
            None if tail.is_empty() => 5900,
            None => return Err("Invalid IPv6 address".into()),
        };
        return Ok((host.to_string(), port));
    }
    if let Some((host, port)) = trimmed.rsplit_once(':') {
        if host.contains(':') {
            // Bare IPv6 without brackets.
            return Ok((trimmed.to_string(), 5900));
        }
        if host.is_empty() {
            return Err("Enter a VNC server host".into());
        }
        return Ok((host.to_string(), parse_port(port)?));
    }
    Ok((trimmed.to_string(), 5900))
}

fn parse_port(s: &str) -> Result<u16, String> {
    s.parse::<u16>()
        .map_err(|_| "Port must be a number from 1 to 65535".into())
        .and_then(|p| {
            if p == 0 {
                Err("Port must be a number from 1 to 65535".into())
            } else {
                Ok(p)
            }
        })
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
    fn parse_ipv6() {
        assert_eq!(
            parse_server("[2001:db8::1]:5902").unwrap(),
            ("2001:db8::1".into(), 5902)
        );
    }

    #[test]
    fn reject_empty() {
        assert!(parse_server("  ").is_err());
    }

    #[test]
    fn connection_round_trip() {
        let c = Connection::new("office", "10.0.0.8", 5900);
        let json = serde_json::to_string(&c).unwrap();
        let back: Connection = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "office");
        assert_eq!(back.port, 5900);
        assert!(back.shared);
    }
}
