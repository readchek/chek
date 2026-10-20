use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use axum::http::HeaderName;
use reqwest::Url;

use crate::limit::IpRange;
use crate::oidc::ProviderConfig;

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
    /// `CHEK_MAX_UPLOAD_BYTES`, the largest file an upload may send. Default
    /// 2 GiB.
    pub max_upload_bytes: u64,
    /// `CHEK_SOURCE_SYNC_MINUTES`, how often source libraries such as Komga
    /// are synced. Default 60; 0 syncs them only at startup and on request.
    pub source_sync_minutes: u64,
    /// `CHEK_EXTENSION_IMAGE_CACHE_MB`, how much of the covers and pages
    /// fetched through extensions to keep on disk. Default 2048; 0 keeps
    /// none.
    pub extension_image_cache_mb: u64,
    /// `CHEK_BROWSER_SOLVER_URL`, a FlareSolverr-compatible endpoint such as
    /// `http://flaresolverr:8191/v1`, which extensions with the `browser`
    /// capability use to get past challenges. Default none.
    pub browser_solver: Option<String>,
    /// `CHEK_BROWSER_EXECUTABLE`, absolute path to a Chromium executable.
    pub browser_executable: Option<PathBuf>,
    /// `CHEK_PUBLIC_URL`, the address people reach the server at, such as
    /// `https://chek.example.com`. Needed for sign-in providers, which send
    /// people back to it.
    pub public_url: Option<Url>,
    /// `CHEK_OIDC_PROVIDERS`, comma-separated provider ids, each set up with
    /// `CHEK_OIDC_<ID>_*` variables. Default none.
    pub oidc_providers: Vec<ProviderConfig>,
    /// `CHEK_OIDC_REDIRECTS`, comma-separated prefixes of the addresses apps
    /// may be sent back to after signing in with a provider, besides the
    /// public URL. Default `chek://`.
    pub oidc_redirects: Vec<String>,
    /// `CHEK_DEMO_USERS`, comma-separated names of shared demo accounts,
    /// whose password is public. Their account, extensions and annotations
    /// can't be changed, and their reading places aren't kept, so one
    /// visitor can't lock the others out or see where they are. Default
    /// none.
    pub demo_users: Vec<String>,
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
            max_upload_bytes: parse("CHEK_MAX_UPLOAD_BYTES", "2147483648")?,
            source_sync_minutes: parse("CHEK_SOURCE_SYNC_MINUTES", "60")?,
            extension_image_cache_mb: parse("CHEK_EXTENSION_IMAGE_CACHE_MB", "2048")?,
            browser_solver: match env::var("CHEK_BROWSER_SOLVER_URL") {
                Ok(url) if !url.trim().is_empty() => {
                    let parsed = parse::<Url>("CHEK_BROWSER_SOLVER_URL", "")?;
                    if !matches!(parsed.scheme(), "http" | "https") {
                        return Err(ConfigError {
                            name: "CHEK_BROWSER_SOLVER_URL",
                            reason: "use an http or https address".to_owned(),
                        });
                    }
                    Some(url.trim().to_owned())
                }
                _ => None,
            },
            public_url: match env::var("CHEK_PUBLIC_URL") {
                Ok(url) if !url.trim().is_empty() => {
                    let url = parse::<Url>("CHEK_PUBLIC_URL", "")?;
                    if !matches!(url.scheme(), "http" | "https") {
                        return Err(ConfigError {
                            name: "CHEK_PUBLIC_URL",
                            reason: "use an http or https address".to_owned(),
                        });
                    }
                    Some(url)
                }
                _ => None,
            },
            browser_executable: env::var_os("CHEK_BROWSER_EXECUTABLE")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            oidc_providers: list("CHEK_OIDC_PROVIDERS", "")
                .iter()
                .map(|id| provider(id))
                .collect::<Result<_, _>>()?,
            oidc_redirects: list("CHEK_OIDC_REDIRECTS", "chek://"),
            demo_users: list("CHEK_DEMO_USERS", ""),
        })
        .and_then(|config| {
            if !config.oidc_providers.is_empty() && config.public_url.is_none() {
                return Err(ConfigError {
                    name: "CHEK_PUBLIC_URL",
                    reason: "sign-in providers need it, to send people back to the server"
                        .to_owned(),
                });
            }
            Ok(config)
        })
    }
}

/// Reads one sign-in provider from `CHEK_OIDC_<ID>_*`.
fn provider(id: &str) -> Result<ProviderConfig, ConfigError> {
    if id.is_empty()
        || id.len() > 32
        || !id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(ConfigError {
            name: "CHEK_OIDC_PROVIDERS",
            reason: format!(
                "{id:?} must be 1 to 32 lowercase letters, digits and hyphens, such as authentik"
            ),
        });
    }
    let prefix = format!("CHEK_OIDC_{}_", id.to_ascii_uppercase().replace('-', "_"));
    let var = |suffix: &str| {
        env::var(format!("{prefix}{suffix}"))
            .ok()
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    let required = |suffix: &'static str| {
        var(suffix).ok_or_else(|| ConfigError {
            name: "CHEK_OIDC_PROVIDERS",
            reason: format!("provider {id:?} needs {prefix}{suffix}"),
        })
    };
    let issuer = required("ISSUER")?;
    let invalid_issuer = |reason: &str| ConfigError {
        name: "CHEK_OIDC_PROVIDERS",
        reason: format!("{prefix}ISSUER {issuer:?} {reason}"),
    };
    let url = Url::parse(&issuer).map_err(|e| invalid_issuer(&e.to_string()))?;
    // Sign-in trusts what the issuer says about who someone is, so it must
    // be reached over TLS, except on this machine for testing.
    let loopback = match url.host_str() {
        Some("localhost") => true,
        Some(host) => host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback()),
        None => false,
    };
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err(invalid_issuer("must be an https address"));
    }
    let create_users = match var("CREATE_USERS").as_deref() {
        None | Some("false") => false,
        Some("true") => true,
        Some(other) => {
            return Err(ConfigError {
                name: "CHEK_OIDC_PROVIDERS",
                reason: format!("{prefix}CREATE_USERS is {other:?}; use true or false"),
            });
        }
    };
    Ok(ProviderConfig {
        id: id.to_owned(),
        name: var("NAME").unwrap_or_else(|| id.to_owned()),
        issuer,
        client_id: required("CLIENT_ID")?,
        client_secret: var("CLIENT_SECRET"),
        scopes: var("SCOPES").unwrap_or_else(|| "openid profile email".to_owned()),
        create_users,
        admin_groups: list(&format!("{prefix}ADMIN_GROUPS"), ""),
        groups_claim: var("GROUPS_CLAIM").unwrap_or_else(|| "groups".to_owned()),
    })
}

/// A comma-separated variable, without blanks.
fn list(name: &str, default: &str) -> Vec<String> {
    env::var(name)
        .unwrap_or_else(|_| default.to_owned())
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
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
