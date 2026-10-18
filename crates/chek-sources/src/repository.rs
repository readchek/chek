//! Extension repositories: an `index.json` listing the extensions a
//! repository offers, and the component files next to it.

use std::io::Read;
use std::time::Duration;

use reqwest::Url;
use reqwest::blocking::Client;
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
}

#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    #[error("`{0}` isn't an HTTPS URL")]
    NotHttps(String),
    #[error("couldn't fetch the index: {0}")]
    Fetch(String),
    #[error("the index is invalid: {0}")]
    Invalid(String),
}

/// Checks that a repository URL is something the server will fetch, and
/// returns it in normal form.
pub fn repository_url(url: &str) -> Result<String, RepositoryError> {
    let parsed = Url::parse(url.trim()).map_err(|_| RepositoryError::NotHttps(url.to_owned()))?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(RepositoryError::NotHttps(url.to_owned()));
    }
    Ok(parsed.to_string())
}

/// Fetches and checks a repository's index.
pub fn fetch_index(url: &str) -> Result<Index, RepositoryError> {
    let bytes = get(url, MAX_INDEX)?;
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
pub fn download(index_url: &str, entry: &IndexEntry) -> Result<Vec<u8>, RepositoryError> {
    let base =
        Url::parse(index_url).map_err(|_| RepositoryError::NotHttps(index_url.to_owned()))?;
    let url = base
        .join(&entry.file)
        .map_err(|e| RepositoryError::Invalid(format!("`{}`: {e}", entry.file)))?;
    let bytes = get(url.as_str(), MAX_COMPONENT)?;
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

/// Sends a GET over HTTPS and reads at most `max` bytes.
pub(crate) fn get(url: &str, max: u64) -> Result<Vec<u8>, RepositoryError> {
    let url = repository_url(url)?;
    let client = Client::builder()
        .user_agent(concat!("Chek/", env!("CARGO_PKG_VERSION")))
        .timeout(TIMEOUT)
        .https_only(true)
        .redirect(Policy::limited(5))
        .build()
        .map_err(|e| RepositoryError::Fetch(e.to_string()))?;
    let response = client
        .get(&url)
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
