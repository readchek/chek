//! A client for the parts of Kavita's REST API that a Chek source library
//! reads: series, volumes and chapters to sync, and pages, files and covers
//! to serve.
//!
//! Kavita is reached with one account's API key, copied from that account's
//! settings page in Kavita. The key is exchanged for a session token, which
//! is kept in memory and exchanged again when Kavita stops accepting it.
//! Redirects aren't followed, so the key only ever goes to the address the
//! admin gave.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub use reqwest::header::HeaderValue;
pub use reqwest::{Client, Response};

use reqwest::header::{ACCEPT, CONTENT_TYPE, RANGE};
use reqwest::{Method, RequestBuilder, StatusCode, Url};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Series are listed this many at a time.
const PAGE_SIZE: u32 = 200;
/// Stops a server that never runs out of pages: two million series.
const MAX_LIST_PAGES: u32 = 10_000;
const MAX_JSON_BYTES: usize = 64 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a response may go quiet. Files stream for as long as they take.
const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// What Chek calls itself when it signs in, shown in Kavita's logs.
const PLUGIN_NAME: &str = "Chek";

/// How a library reaches its Kavita server, as stored with the library.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KavitaConfig {
    /// Kavita's address, such as `http://kavita:5000/`, including any path
    /// it is served under.
    pub url: String,
    /// The API key of the Kavita account to read with.
    pub api_key: String,
    /// Kavita libraries to read; empty reads every library the account sees.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub library_ids: Vec<i64>,
    /// The Chek user whose reading progress is written back to this Kavita
    /// account, so Kavita's own readers stay in step. `None` writes nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_user_id: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum KavitaError {
    #[error("Kavita's address must be an http:// or https:// URL, such as http://kavita:5000")]
    BadUrl,
    #[error("couldn't reach Kavita: {0}")]
    Unreachable(String),
    #[error("Kavita refused the API key; copy it from the account's settings page in Kavita")]
    Unauthorized,
    #[error("the Kavita account isn't allowed to {0}; give it that role in Kavita")]
    Forbidden(&'static str),
    #[error("Kavita has no {0}")]
    NotFound(String),
    #[error("Kavita answered {0}")]
    Status(StatusCode),
    #[error("the answer isn't Kavita's; check the address")]
    NotKavita,
    #[error("Kavita's answer is larger than {0} bytes")]
    TooLarge(usize),
    #[error("Kavita listed more series than Chek reads from one server")]
    TooManyItems,
}

/// A shared HTTP client for talking to Kavita servers.
pub fn http_client() -> reqwest::Result<Client> {
    Client::builder()
        .user_agent(concat!("Chek/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
}

/// What Kavita found a series, chapter or file to be, from its `MangaFormat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(from = "u8")]
pub enum Format {
    /// A folder of loose images, read page by page.
    Image,
    /// A comic archive: CBZ, CBR, CB7 or CBT.
    Archive,
    Epub,
    Pdf,
    #[default]
    Unknown,
}

impl From<u8> for Format {
    fn from(value: u8) -> Self {
        match value {
            0 => Format::Image,
            1 => Format::Archive,
            3 => Format::Epub,
            4 => Format::Pdf,
            _ => Format::Unknown,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub id: i64,
    #[serde(default)]
    pub name: String,
}

/// A Kavita series, with the fields Chek keeps.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Series {
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub sort_name: String,
    pub library_id: i64,
    #[serde(default)]
    pub format: Format,
}

/// Metadata Kavita keeps for a whole series.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesMetadata {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub release_year: i32,
    #[serde(default)]
    pub publishers: Vec<Person>,
    #[serde(flatten)]
    pub people: People,
}

/// A Kavita volume and its chapters. A file that is a whole volume is a
/// chapter numbered `0` inside it; loose chapters sit in a volume numbered
/// [`LOOSE_LEAF_VOLUME`], and specials in one numbered [`SPECIAL_VOLUME`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Volume {
    pub id: i64,
    /// The volume's number as text, such as `1` or `1-2`.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub min_number: f64,
    #[serde(default)]
    pub chapters: Vec<Chapter>,
}

/// The number of the volume Kavita puts chapters that belong to no volume in.
pub const LOOSE_LEAF_VOLUME: f64 = -100_000.0;
/// The number of the volume Kavita puts specials in.
pub const SPECIAL_VOLUME: f64 = 100_000.0;

/// A Kavita chapter: one file, or a folder of images, that Chek serves as
/// one publication.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chapter {
    pub id: i64,
    /// The chapter's number or range as text, such as `12` or `12-13`.
    #[serde(default)]
    pub range: String,
    #[serde(default)]
    pub number: String,
    #[serde(default)]
    pub min_number: f64,
    #[serde(default)]
    pub sort_order: f64,
    #[serde(default)]
    pub pages: u32,
    #[serde(default)]
    pub is_special: bool,
    /// The file's name, for specials.
    #[serde(default)]
    pub title: String,
    /// The title from the file's own metadata.
    #[serde(default)]
    pub title_name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub language: String,
    /// A date-time; Kavita sends year 1 when there is none.
    #[serde(default)]
    pub release_date: String,
    #[serde(default)]
    pub last_modified_utc: String,
    #[serde(default)]
    pub format: Format,
    #[serde(default)]
    pub files: Vec<MangaFile>,
    #[serde(flatten)]
    pub people: People,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MangaFile {
    #[serde(default)]
    pub file_path: String,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub format: Format,
}

/// The people credited on a chapter or series, by Kavita's roles.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct People {
    #[serde(default)]
    pub writers: Vec<Person>,
    #[serde(default)]
    pub pencillers: Vec<Person>,
    #[serde(default)]
    pub inkers: Vec<Person>,
    #[serde(default)]
    pub colorists: Vec<Person>,
    #[serde(default)]
    pub letterers: Vec<Person>,
    #[serde(default)]
    pub cover_artists: Vec<Person>,
    #[serde(default)]
    pub editors: Vec<Person>,
    #[serde(default)]
    pub translators: Vec<Person>,
}

impl People {
    pub fn is_empty(&self) -> bool {
        [
            &self.writers,
            &self.pencillers,
            &self.inkers,
            &self.colorists,
            &self.letterers,
            &self.cover_artists,
            &self.editors,
            &self.translators,
        ]
        .iter()
        .all(|people| people.is_empty())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Person {
    #[serde(default)]
    pub name: String,
}

/// Where a chapter sits in Kavita, which a progress write has to name.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterInfo {
    pub volume_id: i64,
    pub series_id: i64,
    pub library_id: i64,
    #[serde(default)]
    pub pages: u32,
}

/// A page's bytes and the media type Kavita sent them with.
pub struct Page {
    pub bytes: Vec<u8>,
    pub media_type: Option<String>,
}

#[derive(Deserialize)]
struct Signed {
    token: String,
}

/// Session tokens by server and key, shared by every client since one is
/// made per request.
fn tokens() -> &'static Mutex<HashMap<String, String>> {
    static TOKENS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    TOKENS.get_or_init(Mutex::default)
}

/// One Kavita server, as one library reaches it.
#[derive(Clone)]
pub struct Kavita {
    client: Client,
    base: Url,
    api_key: String,
    library_ids: Vec<i64>,
}

impl Kavita {
    pub fn new(client: Client, config: &KavitaConfig) -> Result<Self, KavitaError> {
        let mut base = Url::parse(config.url.trim()).map_err(|_| KavitaError::BadUrl)?;
        if !matches!(base.scheme(), "http" | "https") || base.cannot_be_a_base() {
            return Err(KavitaError::BadUrl);
        }
        // Joining relative paths keeps a prefix, such as Kavita behind a
        // proxy at /kavita, only when the base ends in a slash.
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        base.set_query(None);
        base.set_fragment(None);
        Ok(Self {
            client,
            base,
            api_key: config.api_key.clone(),
            library_ids: config.library_ids.clone(),
        })
    }

    /// Checks the address and key, and that the chosen libraries exist for
    /// this account.
    pub async fn check(&self) -> Result<(), KavitaError> {
        self.sign_in().await?;
        let libraries = self.libraries().await?;
        for id in &self.library_ids {
            if !libraries.iter().any(|l| l.id == *id) {
                return Err(KavitaError::NotFound(format!(
                    "library {id} that this account can see"
                )));
            }
        }
        Ok(())
    }

    /// The libraries the account can see.
    pub async fn libraries(&self) -> Result<Vec<Library>, KavitaError> {
        self.json(
            self.request(Method::GET, &["api", "Library", "libraries"])?,
            "list libraries",
        )
        .await
    }

    /// Every series in the libraries this source reads.
    pub async fn series(&self) -> Result<Vec<Series>, KavitaError> {
        // An empty filter lists everything the account sees; Kavita's
        // filter statements change between versions, so libraries are
        // picked here instead.
        let filter = json!({ "statements": [], "combination": 1, "limitTo": 0 });
        let mut items: Vec<Series> = Vec::new();
        for page in 1..=MAX_LIST_PAGES {
            let request = self
                .request(Method::POST, &["api", "Series", "all-v2"])?
                .query(&[("PageNumber", page), ("PageSize", PAGE_SIZE)])
                .json(&filter);
            let batch: Vec<Series> = self.json(request, "browse libraries").await?;
            let last = (batch.len() as u32) < PAGE_SIZE;
            items.extend(batch);
            if last {
                return Ok(items
                    .into_iter()
                    .filter(|s| {
                        self.library_ids.is_empty() || self.library_ids.contains(&s.library_id)
                    })
                    .collect());
            }
        }
        Err(KavitaError::TooManyItems)
    }

    /// The metadata of one series.
    pub async fn series_metadata(&self, series_id: i64) -> Result<SeriesMetadata, KavitaError> {
        let request = self
            .request(Method::GET, &["api", "Series", "metadata"])?
            .query(&[("seriesId", series_id)]);
        self.json(request, "browse libraries").await
    }

    /// A series' volumes, each with its chapters.
    pub async fn volumes(&self, series_id: i64) -> Result<Vec<Volume>, KavitaError> {
        let request = self
            .request(Method::GET, &["api", "Series", "volumes"])?
            .query(&[("seriesId", series_id)]);
        self.json(request, "browse libraries").await
    }

    /// The chapter's cover image, as Kavita made it.
    pub async fn cover(&self, chapter_id: i64) -> Result<Vec<u8>, KavitaError> {
        let request = self
            .request(Method::GET, &["api", "Image", "chapter-cover"])?
            .query(&[("chapterId", chapter_id)]);
        let response = self.send(request, "read covers").await?;
        read_capped(response, MAX_IMAGE_BYTES).await
    }

    /// One page's image as stored in the chapter. `index` is zero-based.
    pub async fn page(&self, chapter_id: i64, index: u32) -> Result<Page, KavitaError> {
        let request = self
            .request(Method::GET, &["api", "Reader", "image"])?
            .query(&[("chapterId", chapter_id), ("page", i64::from(index))])
            .header(ACCEPT, "image/*");
        let response = self.send(request, "read pages").await?;
        let media_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        Ok(Page {
            bytes: read_capped(response, MAX_IMAGE_BYTES).await?,
            media_type,
        })
    }

    /// Starts downloading a chapter's file. A `range` header is passed on,
    /// so the response may be 206 Partial Content or 416; any other answer
    /// than those and 200 is an error.
    pub async fn file(
        &self,
        chapter_id: i64,
        range: Option<&HeaderValue>,
    ) -> Result<Response, KavitaError> {
        let mut request = self
            .request(Method::GET, &["api", "Download", "chapter"])?
            .query(&[("chapterId", chapter_id)]);
        if let Some(range) = range {
            request = request.header(RANGE, range.clone());
        }
        let response = self.send_unchecked(request).await?;
        match response.status() {
            StatusCode::RANGE_NOT_SATISFIABLE => Ok(response),
            _ => check_status(response, "download files"),
        }
    }

    /// Where a chapter sits, for a progress write. Kavita extracts the
    /// chapter's images when asked, as reading it does.
    pub async fn chapter_info(&self, chapter_id: i64) -> Result<ChapterInfo, KavitaError> {
        let request = self
            .request(Method::GET, &["api", "Reader", "chapter-info"])?
            .query(&[("chapterId", chapter_id)]);
        self.json(request, "read pages").await
    }

    /// Records the page the account is on in a chapter, counted from zero.
    /// Sending the chapter's page count marks it read, as Kavita's own
    /// reader does on the last page.
    pub async fn set_progress(
        &self,
        chapter_id: i64,
        info: &ChapterInfo,
        page_num: u32,
    ) -> Result<(), KavitaError> {
        let body = json!({
            "volumeId": info.volume_id,
            "chapterId": chapter_id,
            "pageNum": page_num,
            "seriesId": info.series_id,
            "libraryId": info.library_id,
            "bookScrollId": null,
        });
        let request = self
            .request(Method::POST, &["api", "Reader", "progress"])?
            .json(&body);
        self.send(request, "save read progress").await?;
        Ok(())
    }

    /// Exchanges the API key for a session token and remembers it.
    async fn sign_in(&self) -> Result<String, KavitaError> {
        let request = self
            .request(Method::POST, &["api", "Plugin", "authenticate"])?
            .query(&[
                ("apiKey", self.api_key.as_str()),
                ("pluginName", PLUGIN_NAME),
            ])
            .header(ACCEPT, "application/json");
        let response = self
            .client
            .execute(build(request)?)
            .await
            .map_err(unreachable)?;
        let response = check_status(response, "sign in")?;
        let body = read_capped(response, MAX_JSON_BYTES).await?;
        let signed: Signed = serde_json::from_slice(&body).map_err(|_| KavitaError::NotKavita)?;
        tokens()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(self.token_key(), signed.token.clone());
        Ok(signed.token)
    }

    fn token_key(&self) -> String {
        format!("{}\n{}", self.base, self.api_key)
    }

    fn request(&self, method: Method, path: &[&str]) -> Result<RequestBuilder, KavitaError> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|()| KavitaError::BadUrl)?
            .pop_if_empty()
            .extend(path);
        Ok(self.client.request(method, url))
    }

    /// Sends with the session token, signing in first when there is none
    /// and again when Kavita no longer accepts it.
    async fn send_unchecked(&self, request: RequestBuilder) -> Result<Response, KavitaError> {
        let retry = request.try_clone();
        let cached = tokens()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&self.token_key())
            .cloned();
        let token = match cached {
            Some(token) => token,
            None => self.sign_in().await?,
        };
        let response = self
            .client
            .execute(build(request.bearer_auth(&token))?)
            .await
            .map_err(unreachable)?;
        if response.status() != StatusCode::UNAUTHORIZED {
            return Ok(response);
        }
        let Some(retry) = retry else {
            return Ok(response);
        };
        let token = self.sign_in().await?;
        self.client
            .execute(build(retry.bearer_auth(&token))?)
            .await
            .map_err(unreachable)
    }

    async fn send(
        &self,
        request: RequestBuilder,
        action: &'static str,
    ) -> Result<Response, KavitaError> {
        let response = self.send_unchecked(request).await?;
        check_status(response, action)
    }

    async fn json<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
        action: &'static str,
    ) -> Result<T, KavitaError> {
        let request = request.header(ACCEPT, "application/json");
        let response = self.send(request, action).await?;
        let body = read_capped(response, MAX_JSON_BYTES).await?;
        serde_json::from_slice(&body).map_err(|_| KavitaError::NotKavita)
    }
}

fn build(request: RequestBuilder) -> Result<reqwest::Request, KavitaError> {
    request.build().map_err(unreachable)
}

fn check_status(response: Response, action: &'static str) -> Result<Response, KavitaError> {
    match response.status() {
        status if status.is_success() => Ok(response),
        StatusCode::UNAUTHORIZED => Err(KavitaError::Unauthorized),
        StatusCode::FORBIDDEN => Err(KavitaError::Forbidden(action)),
        StatusCode::NOT_FOUND => Err(KavitaError::NotFound(
            "such item; it may have been removed".to_owned(),
        )),
        status => Err(KavitaError::Status(status)),
    }
}

async fn read_capped(mut response: Response, limit: usize) -> Result<Vec<u8>, KavitaError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(KavitaError::TooLarge(limit));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(unreachable)? {
        if body.len() + chunk.len() > limit {
            return Err(KavitaError::TooLarge(limit));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// reqwest's message repeats the URL, which the admin already knows; the
/// cause is what helps.
fn unreachable(e: reqwest::Error) -> KavitaError {
    let e = e.without_url();
    let mut message = e.to_string();
    let mut source = std::error::Error::source(&e);
    while let Some(cause) = source {
        message = format!("{message}: {cause}");
        source = cause.source();
    }
    KavitaError::Unreachable(message)
}
