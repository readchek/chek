//! Extension repositories: an `index.json` listing the extensions a
//! repository offers, and the component files next to it.
//!
//! A repository is named by its index's URL, by the folder that holds
//! `index.json`, or by a GitHub repository as `github:owner/repo`. Private
//! repositories take credentials, which are sent only to the index's own
//! origin.

use std::io::Read;
use std::time::Duration;

use reqwest::Url;
use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, HeaderName, HeaderValue};
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};

use crate::{MAX_FIELD, valid_id};

/// The largest index the server reads.
const MAX_INDEX: u64 = 1 << 20;
/// Indexes can't list more extensions than this.
const MAX_ENTRIES: usize = 1_000;
/// The largest component the server downloads.
const MAX_COMPONENT: u64 = 32 << 20;
const TIMEOUT: Duration = Duration::from_secs(30);

/// A repository's `index.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Index {
    /// What the repository calls itself.
    #[serde(default)]
    pub name: Option<String>,
    pub extensions: Vec<IndexEntry>,
}

/// One extension a repository offers. Its id, version and sites must match
/// the manifest inside the component.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    /// The component's URL, relative to the index.
    pub file: String,
    /// The component's SHA-256 digest in lowercase hex.
    pub sha256: String,
    #[serde(default)]
    pub sites: Vec<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    /// What the extension needs beyond plain requests, such as `browser`.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// How to sign in to a private repository.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Credentials {
    /// `Authorization: Bearer`, such as a GitHub access token.
    Bearer { token: String },
    /// HTTP Basic.
    Basic { username: String, password: String },
    /// Any other header, such as an API key.
    Header { name: String, value: String },
}

// Credentials are secrets, so they never reach a log.
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self {
            Credentials::Bearer { .. } => "Bearer",
            Credentials::Basic { .. } => "Basic",
            Credentials::Header { .. } => "Header",
        };
        write!(f, "Credentials::{kind}(..)")
    }
}

impl Credentials {
    /// Checks the credentials can be sent as a header.
    pub fn check(&self) -> Result<(), RepositoryError> {
        let invalid = |what: &str| Err(RepositoryError::Credentials(what.to_owned()));
        match self {
            Credentials::Bearer { token } if token.trim().is_empty() => {
                invalid("the token is empty")
            }
            Credentials::Basic { username, .. } if username.is_empty() => {
                invalid("the username is empty")
            }
            Credentials::Header { name, .. } if HeaderName::try_from(name.as_str()).is_err() => {
                invalid("the header name isn't valid")
            }
            _ => match self.header() {
                Some(_) => Ok(()),
                None => invalid("the value can't be sent as a header"),
            },
        }
    }

    fn header(&self) -> Option<(HeaderName, HeaderValue)> {
        use base64::Engine;
        let (name, value) = match self {
            Credentials::Bearer { token } => (AUTHORIZATION, format!("Bearer {}", token.trim())),
            Credentials::Basic { username, password } => (
                AUTHORIZATION,
                format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD
                        .encode(format!("{username}:{password}"))
                ),
            ),
            Credentials::Header { name, value } => {
                (HeaderName::try_from(name.as_str()).ok()?, value.clone())
            }
        };
        let mut value = HeaderValue::try_from(value).ok()?;
        value.set_sensitive(true);
        Some((name, value))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    #[error("`{0}` isn't an HTTPS URL")]
    NotHttps(String),
    #[error("couldn't fetch the index: {0}")]
    Fetch(String),
    #[error("the index is invalid: {0}")]
    Invalid(String),
    #[error("the credentials can't be used: {0}")]
    Credentials(String),
}

/// Turns what a user typed into the URL of the repository's index, which is
/// how the repository is stored. It accepts:
///
/// - the index itself, such as `https://example.org/chek/index.json`;
/// - the folder holding it, such as `https://chek.page/extensions`, with or
///   without `https://`;
/// - a GitHub repository as `github:owner/repo`, optionally with a branch
///   or tag as `@ref` and a folder as `/path`, read from the repository's
///   raw files.
pub fn repository_url(input: &str) -> Result<String, RepositoryError> {
    let input = input.trim();
    let not_https = || RepositoryError::NotHttps(input.to_owned());
    let url = if let Some(github) = input.strip_prefix("github:") {
        github_url(github).ok_or_else(not_https)?
    } else if input.contains("://") {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    let mut parsed = Url::parse(&url).map_err(|_| not_https())?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(not_https());
    }
    if !parsed.path().ends_with(".json") {
        if !parsed.path().ends_with('/') {
            let path = format!("{}/", parsed.path());
            parsed.set_path(&path);
        }
        parsed = parsed.join("index.json").map_err(|_| not_https())?;
    }
    Ok(parsed.to_string())
}

/// `owner/repo[@ref][/path]` as the raw URL of the folder holding the
/// index. Without a ref, GitHub serves the default branch as `HEAD`.
fn github_url(spec: &str) -> Option<String> {
    let mut parts = spec.trim_matches('/').splitn(3, '/');
    let owner = parts.next().filter(|s| !s.is_empty())?;
    let repo = parts.next().filter(|s| !s.is_empty())?;
    let path = parts.next().unwrap_or("");
    let (repo, reference) = match repo.split_once('@') {
        Some((repo, reference)) if !reference.is_empty() => (repo, reference),
        Some(_) => return None,
        None => (repo, "HEAD"),
    };
    let plain = |s: &str| {
        s.bytes().any(|b| b != b'.')
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    if !plain(owner) || !plain(repo) || reference.contains("..") {
        return None;
    }
    Some(format!(
        "https://raw.githubusercontent.com/{owner}/{repo}/{reference}/{path}"
    ))
}

/// Fetches and checks a repository's index.
pub fn fetch_index(url: &str, credentials: Option<&Credentials>) -> Result<Index, RepositoryError> {
    let bytes = get(url, url, credentials, MAX_INDEX)?;
    parse_index(&bytes)
}

/// Reads an index fetched earlier.
pub fn parse_index(bytes: &[u8]) -> Result<Index, RepositoryError> {
    let mut index: Index =
        serde_json::from_slice(bytes).map_err(|e| RepositoryError::Invalid(e.to_string()))?;
    if index.extensions.len() > MAX_ENTRIES {
        return Err(RepositoryError::Invalid(format!(
            "lists {} extensions; the limit is {MAX_ENTRIES}",
            index.extensions.len()
        )));
    }
    if let Some(name) = &mut index.name {
        crate::truncate(name, MAX_FIELD);
    }
    for entry in &index.extensions {
        if !valid_id(&entry.id) {
            return Err(RepositoryError::Invalid(format!(
                "extension id `{}` isn't in reverse-DNS form",
                entry.id
            )));
        }
        let digest_ok = entry.sha256.len() == 64
            && entry
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !digest_ok {
            return Err(RepositoryError::Invalid(format!(
                "`{}` has no lowercase hex SHA-256 digest",
                entry.id
            )));
        }
        if entry.name.trim().is_empty()
            || entry.version.trim().is_empty()
            || entry.name.len() > MAX_FIELD
            || entry.version.len() > MAX_FIELD
        {
            return Err(RepositoryError::Invalid(format!(
                "`{}` needs a name and version of at most {MAX_FIELD} bytes",
                entry.id
            )));
        }
    }
    Ok(index)
}

/// Downloads an entry's component from the repository at `index_url` and
/// checks it against the entry's digest.
pub fn download(
    index_url: &str,
    entry: &IndexEntry,
    credentials: Option<&Credentials>,
) -> Result<Vec<u8>, RepositoryError> {
    let base =
        Url::parse(index_url).map_err(|_| RepositoryError::NotHttps(index_url.to_owned()))?;
    let url = base
        .join(&entry.file)
        .map_err(|e| RepositoryError::Invalid(format!("`{}`: {e}", entry.file)))?;
    let bytes = get(url.as_str(), index_url, credentials, MAX_COMPONENT)?;
    if sha256_hex(&bytes) != entry.sha256 {
        return Err(RepositoryError::Invalid(format!(
            "{url} doesn't match the digest in the index"
        )));
    }
    Ok(bytes)
}

/// A component's SHA-256 digest in lowercase hex, as indexes write it.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Sends a GET over HTTPS and reads at most `max` bytes. Credentials go
/// only to the index's origin, and a request carrying them isn't followed
/// to another origin, so a repository can't send them elsewhere.
fn get(
    url: &str,
    index_url: &str,
    credentials: Option<&Credentials>,
    max: u64,
) -> Result<Vec<u8>, RepositoryError> {
    let url = Url::parse(url).map_err(|_| RepositoryError::NotHttps(url.to_owned()))?;
    if url.scheme() != "https" {
        return Err(RepositoryError::NotHttps(url.to_string()));
    }
    let origin = Url::parse(index_url)
        .map_err(|_| RepositoryError::NotHttps(index_url.to_owned()))?
        .origin();
    let credentials = credentials.filter(|_| url.origin() == origin);
    let header = match credentials {
        Some(credentials) => Some(credentials.header().ok_or_else(|| {
            RepositoryError::Credentials("the value can't be sent as a header".to_owned())
        })?),
        None => None,
    };
    let signed = header.is_some();
    let client = Client::builder()
        .user_agent(concat!("Chek/", env!("CARGO_PKG_VERSION")))
        .timeout(TIMEOUT)
        .https_only(true)
        .redirect(Policy::custom(move |attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("too many redirects")
            } else if signed && attempt.url().origin() != origin {
                attempt.error("redirected to another site, which isn't sent the credentials")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| RepositoryError::Fetch(e.to_string()))?;
    let mut request = client.get(url.clone());
    if let Some((name, value)) = header {
        request = request.header(name, value);
    }
    let response = request
        .send()
        .map_err(|e| RepositoryError::Fetch(crate::http::describe(&e)))?;
    if !response.status().is_success() {
        return Err(RepositoryError::Fetch(format!(
            "{url} answered {}",
            response.status()
        )));
    }
    if response.content_length().is_some_and(|length| length > max) {
        return Err(RepositoryError::Fetch(format!("larger than {max} bytes")));
    }
    let mut bytes = Vec::new();
    response
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| RepositoryError::Fetch(e.to_string()))?;
    if bytes.len() as u64 > max {
        return Err(RepositoryError::Fetch(format!("larger than {max} bytes")));
    }
    Ok(bytes)
}
