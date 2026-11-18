//! Server configuration: a TOML file, overridden by `HITO_*` environment variables.
//!
//! Secrets never live here: the server has none of its own (session tokens and invitation codes
//! are random and stored hashed), so the file can be committed to a deployment repository.

use std::net::SocketAddr;
use std::path::Path;

use hito_protocol::Registration;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Name shown to clients.
    pub name: String,
    /// Address and port to listen on. Loopback by default.
    pub bind: SocketAddr,
    /// SQLx SQLite URL, e.g. `sqlite://hito.db` or `sqlite::memory:` (tests).
    pub database: String,
    pub registration: RegistrationPolicy,
    /// Days a session lasts after logging in.
    pub session_days: u32,
    /// The server sits behind a reverse proxy that terminates TLS and sets `X-Forwarded-For`.
    /// Rate limits then key on that header's last address.
    pub trust_proxy: bool,
    /// Allow listening on a non-loopback address without a TLS proxy in front. Passwords and
    /// tokens would cross the network in clear text: only for an isolated lab network.
    pub allow_insecure_public_bind: bool,
    pub limits: Limits,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationPolicy {
    Open,
    InviteOnly,
    Closed,
}

impl From<RegistrationPolicy> for Registration {
    fn from(p: RegistrationPolicy) -> Self {
        match p {
            RegistrationPolicy::Open => Registration::Open,
            RegistrationPolicy::InviteOnly => Registration::InviteOnly,
            RegistrationPolicy::Closed => Registration::Closed,
        }
    }
}

/// Abuse limits. The defaults suit a team; raise them for very large workspaces.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    /// Login and registration attempts per address per minute.
    pub auth_per_minute: u32,
    /// Messages, edits and reactions per user per minute.
    pub writes_per_minute: u32,
    /// Other API requests per user per minute.
    pub reads_per_minute: u32,
    /// Simultaneous WebSocket connections per user (devices, tabs).
    pub connections_per_user: usize,
    /// Largest HTTP request body, in bytes.
    pub max_body_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            auth_per_minute: 10,
            writes_per_minute: 120,
            reads_per_minute: 1_200,
            connections_per_user: 16,
            max_body_bytes: 1024 * 1024,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            name: "Hito".into(),
            bind: SocketAddr::from(([127, 0, 0, 1], 7878)),
            database: "sqlite://hito.db".into(),
            registration: RegistrationPolicy::Open,
            session_days: 30,
            trust_proxy: false,
            allow_insecure_public_bind: false,
            limits: Limits::default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("{path} is not valid: {source}")]
    Parse {
        path: String,
        source: Box<toml::de::Error>,
    },
    #[error("{name}={value:?} is not valid: {reason}")]
    Env {
        name: &'static str,
        value: String,
        reason: String,
    },
    #[error(
        "refusing to listen on {0} without TLS: put a TLS reverse proxy in front and set \
         trust_proxy = true, or set allow_insecure_public_bind = true on an isolated network \
         (see docs/deployment.md)"
    )]
    InsecureBind(SocketAddr),
    #[error("session_days must be between 1 and 365")]
    SessionDays,
}

impl Config {
    /// Read `path` if given, then apply environment overrides, then check the result.
    pub fn load(path: Option<&Path>) -> Result<Config, ConfigError> {
        let mut config = match path {
            Some(path) => {
                let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
                    path: path.display().to_string(),
                    source,
                })?;
                toml::from_str(&text).map_err(|source| ConfigError::Parse {
                    path: path.display().to_string(),
                    source: Box::new(source),
                })?
            }
            None => Config::default(),
        };
        config.apply_env(|name| std::env::var(name).ok())?;
        config.check()?;
        Ok(config)
    }

    fn apply_env(&mut self, get: impl Fn(&str) -> Option<String>) -> Result<(), ConfigError> {
        if let Some(v) = get("HITO_BIND") {
            self.bind = v
                .parse()
                .map_err(|e: std::net::AddrParseError| ConfigError::Env {
                    name: "HITO_BIND",
                    value: v.clone(),
                    reason: e.to_string(),
                })?;
        }
        if let Some(v) = get("HITO_DATABASE") {
            self.database = v;
        }
        if let Some(v) = get("HITO_REGISTRATION") {
            self.registration = match v.as_str() {
                "open" => RegistrationPolicy::Open,
                "invite_only" => RegistrationPolicy::InviteOnly,
                "closed" => RegistrationPolicy::Closed,
                _ => {
                    return Err(ConfigError::Env {
                        name: "HITO_REGISTRATION",
                        value: v,
                        reason: "expected open, invite_only or closed".into(),
                    })
                }
            };
        }
        if let Some(v) = get("HITO_TRUST_PROXY") {
            self.trust_proxy = matches!(v.as_str(), "1" | "true" | "yes");
        }
        Ok(())
    }

    /// Refuse configurations that would be unsafe on a public network.
    pub fn check(&self) -> Result<(), ConfigError> {
        if !(1..=365).contains(&self.session_days) {
            return Err(ConfigError::SessionDays);
        }
        if !self.bind.ip().is_loopback() && !self.trust_proxy && !self.allow_insecure_public_bind {
            return Err(ConfigError::InsecureBind(self.bind));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_example_file_matches_the_defaults() {
        let text = include_str!("../../../hito.example.toml");
        let example: Config = toml::from_str(text).unwrap();
        assert_eq!(format!("{example:?}"), format!("{:?}", Config::default()));
    }

    #[test]
    fn defaults_are_local_and_safe() {
        let c = Config::default();
        assert!(c.bind.ip().is_loopback());
        assert!(c.check().is_ok());
    }

    #[test]
    fn public_bind_needs_a_tls_proxy_or_an_explicit_opt_out() {
        let mut c = Config {
            bind: "0.0.0.0:7878".parse().unwrap(),
            ..Config::default()
        };
        assert!(matches!(c.check(), Err(ConfigError::InsecureBind(_))));
        c.trust_proxy = true;
        assert!(c.check().is_ok());
    }

    #[test]
    fn environment_overrides_the_file() {
        let mut c: Config =
            toml::from_str("registration = \"open\"\nbind = \"127.0.0.1:9000\"").unwrap();
        c.apply_env(|n| match n {
            "HITO_REGISTRATION" => Some("invite_only".into()),
            "HITO_BIND" => Some("127.0.0.1:9100".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(c.registration, RegistrationPolicy::InviteOnly);
        assert_eq!(c.bind.port(), 9100);
        assert!(c.apply_env(|_| Some("bogus".into())).is_err());
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(toml::from_str::<Config>("bnid = \"x\"").is_err());
    }
}
