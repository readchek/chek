use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use axum::http::HeaderName;

use crate::limit::IpRange;

/// Server settings, read from `CHEK_*` environment variables.
pub struct Config {
    /// `CHEK_BIND`, default `127.0.0.1:8080`. Loopback by default because the
    /// first milestone has no sign-in yet.
    pub bind: SocketAddr,
    /// `CHEK_DATA_DIR`, default `./data`. Holds the SQLite database.
    pub data_dir: PathBuf,
    /// `CHEK_SCAN_WORKERS`, default one fewer than the cores.
    pub scan_workers: usize,
    /// `CHEK_TRUSTED_PROXIES`, comma-separated addresses or CIDR ranges of
    /// reverse proxies whose `X-Forwarded-For` header names the real client.
    /// Default none.
    pub trusted_proxies: Vec<IpRange>,
    /// `CHEK_CLIENT_IP_HEADER`, a header such as `CF-Connecting-IP` that a
    /// CDN in front of the trusted proxies sets to the client's address.
    /// Believed only from trusted proxies, before `X-Forwarded-For`. Default
    /// none.
    pub client_ip_header: Option<HeaderName>,
    /// `CHEK_SETUP_TOKEN`, the token that creates the first admin, for
    /// deployments that set up the server without reading its log. Default
    /// a random token, logged at startup.
    pub setup_token: Option<String>,
}

/// Long enough that a chosen token can't be guessed like a password.
const MIN_SETUP_TOKEN_LEN: usize = 32;

#[derive(Debug, thiserror::Error)]
#[error("{name} is invalid: {reason}")]
pub struct ConfigError {
    name: &'static str,
    reason: String,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            bind: parse("CHEK_BIND", "127.0.0.1:8080")?,
            data_dir: env::var_os("CHEK_DATA_DIR")
                .map_or_else(|| PathBuf::from("data"), PathBuf::from),
            scan_workers: match env::var("CHEK_SCAN_WORKERS") {
                Ok(_) => parse::<usize>("CHEK_SCAN_WORKERS", "")?.max(1),
                Err(_) => chek_library::default_scan_workers(),
            },
            trusted_proxies: env::var("CHEK_TRUSTED_PROXIES")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| {
                    s.parse::<IpRange>().map_err(|e| ConfigError {
                        name: "CHEK_TRUSTED_PROXIES",
                        reason: format!("{s:?}: {e}"),
                    })
                })
                .collect::<Result<_, _>>()?,
            client_ip_header: match env::var("CHEK_CLIENT_IP_HEADER") {
                Ok(name) if !name.trim().is_empty() => {
                    Some(parse::<HeaderName>("CHEK_CLIENT_IP_HEADER", "")?)
                }
                _ => None,
            },
            setup_token: match env::var("CHEK_SETUP_TOKEN") {
                Ok(token) if token.len() < MIN_SETUP_TOKEN_LEN => {
                    return Err(ConfigError {
                        name: "CHEK_SETUP_TOKEN",
                        reason: format!("use at least {MIN_SETUP_TOKEN_LEN} characters"),
                    });
                }
                Ok(token) => Some(token),
                Err(_) => None,
            },
        })
    }
}

fn parse<T>(name: &'static str, default: &str) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let value = env::var(name).unwrap_or_else(|_| default.to_owned());
    value.parse().map_err(|e: T::Err| ConfigError {
        name,
        reason: format!("{value:?}: {e}"),
    })
}
