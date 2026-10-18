//! A client for the parts of Komga's REST API that a Chek source library
//! reads: series and books to sync, and pages, files and thumbnails to serve.
//!
//! Komga is reached with one account's API key or name and password, chosen
//! by the admin who adds the library. Redirects aren't followed, so the
//! credentials only ever go to the address the admin gave.

use std::time::Duration;

pub use reqwest::header::HeaderValue;
pub use reqwest::{Client, Response};

use reqwest::header::{ACCEPT, RANGE};
use reqwest::{Method, RequestBuilder, StatusCode, Url};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Books and series are listed this many at a time.
const PAGE_SIZE: u32 = 500;
/// Stops a server that never reports its last page: two million items.
const MAX_LIST_PAGES: u32 = 4_000;
const MAX_JSON_BYTES: usize = 64 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a response may go quiet. Files stream for as long as they take.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// How a library reaches its Komga server, as stored with the library.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KomgaConfig {
    /// Komga's address, such as `http://komga:25600/`, including any path it
    /// is served under.
    pub url: String,
    #[serde(flatten)]
    pub auth: KomgaAuth,
    /// Komga libraries to read; empty reads every library the account sees.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub library_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, rename_all = "camelCase")]
pub enum KomgaAuth {
    ApiKey {
        #[serde(rename = "apiKey")]
        api_key: String,
    },
    Basic {
        username: String,
        password: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum KomgaError {
    #[error("Komga's address must be an http:// or https:// URL, such as http://komga:25600")]
    BadUrl,
    #[error("couldn't reach Komga: {0}")]
    Unreachable(String),
    #[error("Komga refused the credentials; check the API key, or the name and password")]
    Unauthorized,
    #[error("the Komga account isn't allowed to {0}; give it that role in Komga")]
    Forbidden(&'static str),
    #[error("Komga has no {0}")]
    NotFound(String),
    #[error("Komga answered {0}")]
    Status(StatusCode),
    #[error("the answer isn't Komga's; check the address")]
    NotKomga,
    #[error("Komga's answer is larger than {0} bytes")]
    TooLarge(usize),
    #[error("Komga listed more items than Chek reads from one server")]
    TooManyItems,
}

/// A shared HTTP client for talking to Komga servers.
pub fn http_client() -> reqwest::Result<Client> {
    Client::builder()
        .user_agent(concat!("Chek/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
}

/// A Komga series, with the metadata Chek keeps.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Series {
    pub id: String,
    pub name: String,
    pub last_modified: String,
    #[serde(default)]
    pub deleted: bool,
    pub metadata: SeriesMetadata,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesMetadata {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub title_sort: String,
    #[serde(default)]
    pub publisher: String,
    #[serde(default)]
    pub language: String,
    /// `LEFT_TO_RIGHT`, `RIGHT_TO_LEFT`, `VERTICAL`, `WEBTOON` or empty.
    #[serde(default)]
    pub reading_direction: String,
}

/// A Komga book, with the metadata Chek keeps.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Book {
    pub id: String,
    pub series_id: String,
    pub series_title: String,
    pub name: String,
    /// The file's path on the Komga server.
    pub url: String,
    pub size_bytes: u64,
    pub last_modified: String,
    #[serde(default)]
    pub deleted: bool,
    pub media: Media,
    pub metadata: BookMetadata,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    /// `READY` once Komga has analyzed the file.
    pub status: String,
    pub media_type: String,
    pub pages_count: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookMetadata {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub number: String,
    pub number_sort: Option<f64>,
    pub release_date: Option<String>,
    #[serde(default)]
    pub authors: Vec<Author>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Author {
    pub name: String,
    pub role: String,
}

/// One page of a book, as Komga lists it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    pub file_name: String,
    pub media_type: String,
    pub size_bytes: Option<u64>,
}

impl Book {
    /// The file's name, from its path on the Komga server.
    pub fn file_name(&self) -> &str {
        self.url.rsplit(['/', '\\']).next().unwrap_or(&self.url)
    }
}

#[derive(Deserialize)]
struct ListPage<T> {
    content: Vec<T>,
    last: bool,
}

/// One Komga server, as one library reaches it.
#[derive(Clone)]
pub struct Komga {
    client: Client,
    base: Url,
    auth: KomgaAuth,
    library_ids: Vec<String>,
}

impl Komga {
    pub fn new(client: Client, config: &KomgaConfig) -> Result<Self, KomgaError> {
        let mut base = Url::parse(config.url.trim()).map_err(|_| KomgaError::BadUrl)?;
        if !matches!(base.scheme(), "http" | "https") || base.cannot_be_a_base() {
            return Err(KomgaError::BadUrl);
        }
        // Joining relative paths keeps a prefix, such as Komga behind a
        // proxy at /komga, only when the base ends in a slash.
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        base.set_query(None);
        base.set_fragment(None);
        Ok(Self {
            client,
            base,
            auth: config.auth.clone(),
            library_ids: config.library_ids.clone(),
        })
    }

    /// Checks the address and credentials, and that the chosen libraries
    /// exist for this account.
    pub async fn check(&self) -> Result<(), KomgaError> {
        #[derive(Deserialize)]
        struct Library {
            id: String,
        }
        let libraries: Vec<Library> = self
            .json(self.get(&["api", "v1", "libraries"])?, "list libraries")
            .await?;
        for id in &self.library_ids {
            if !libraries.iter().any(|l| &l.id == id) {
                return Err(KomgaError::NotFound(format!(
                    "library {id:?} that this account can see"
                )));
            }
        }
        Ok(())
    }

    /// Every series in the libraries this source reads.
    pub async fn series(&self) -> Result<Vec<Series>, KomgaError> {
        let condition = self.library_condition(Vec::new());
        let series: Vec<Series> = self
            .list(&["api", "v1", "series", "list"], condition)
            .await?;
        Ok(series.into_iter().filter(|s| !s.deleted).collect())
    }

    /// Every book Komga has finished analyzing in those libraries.
    pub async fn books(&self) -> Result<Vec<Book>, KomgaError> {
        let ready = json!({ "mediaStatus": { "operator": "is", "value": "READY" } });
        let condition = self.library_condition(vec![ready]);
        let books: Vec<Book> = self
            .list(&["api", "v1", "books", "list"], condition)
            .await?;
        Ok(books
            .into_iter()
            .filter(|b| !b.deleted && b.media.status == "READY")
            .collect())
    }

    /// A book's pages in reading order.
    pub async fn pages(&self, book_id: &str) -> Result<Vec<PageInfo>, KomgaError> {
        self.json(
            self.get(&["api", "v1", "books", book_id, "pages"])?,
            "read books",
        )
        .await
    }

    /// The book's thumbnail image.
    pub async fn thumbnail(&self, book_id: &str) -> Result<Vec<u8>, KomgaError> {
        let request = self.get(&["api", "v1", "books", book_id, "thumbnail"])?;
        let response = self.send(request, "read books").await?;
        read_capped(response, MAX_IMAGE_BYTES).await
    }

    /// One page's image as stored in the book. `index` is zero-based.
    pub async fn page(&self, book_id: &str, index: u32) -> Result<Vec<u8>, KomgaError> {
        let number = (index + 1).to_string();
        let request = self
            .get(&["api", "v1", "books", book_id, "pages", &number])?
            // Without this, Komga may convert pages to formats it thinks the
            // client prefers.
            .query(&[("contentNegotiation", "false")])
            .header(ACCEPT, "*/*");
        let response = self.send(request, "stream pages").await?;
        read_capped(response, MAX_IMAGE_BYTES).await
    }

    /// Starts downloading a book's file. A `range` header is passed on, so the
    /// response may be 206 Partial Content or 416; any other answer than
    /// those and 200 is an error.
    pub async fn file(
        &self,
        book_id: &str,
        range: Option<&HeaderValue>,
    ) -> Result<Response, KomgaError> {
        let mut request = self.get(&["api", "v1", "books", book_id, "file"])?;
        if let Some(range) = range {
            request = request.header(RANGE, range.clone());
        }
        let response = self.authorize(request).send().await.map_err(unreachable)?;
        match response.status() {
            StatusCode::RANGE_NOT_SATISFIABLE => Ok(response),
            _ => check_status(response, "download files"),
        }
    }

    /// The condition Komga's search takes to keep to this source's
    /// libraries, joined with `and`.
    fn library_condition(&self, mut and: Vec<Value>) -> Value {
        if !self.library_ids.is_empty() {
            let libraries: Vec<Value> = self
                .library_ids
                .iter()
                .map(|id| json!({ "libraryId": { "operator": "is", "value": id } }))
                .collect();
            and.push(json!({ "anyOf": libraries }));
        }
        and.push(json!({ "deleted": { "operator": "isFalse" } }));
        json!({ "condition": { "allOf": and } })
    }

    /// Every item of a paged list search.
    async fn list<T: DeserializeOwned>(
        &self,
        path: &[&str],
        body: Value,
    ) -> Result<Vec<T>, KomgaError> {
        let mut items = Vec::new();
        for page in 0..MAX_LIST_PAGES {
            let request = self
                .request(Method::POST, path)?
                .query(&[("page", page), ("size", PAGE_SIZE)])
                .json(&body);
            let batch: ListPage<T> = self.json(request, "browse libraries").await?;
            let last = batch.last || batch.content.is_empty();
            items.extend(batch.content);
            if last {
                return Ok(items);
            }
        }
        Err(KomgaError::TooManyItems)
    }

    fn get(&self, path: &[&str]) -> Result<RequestBuilder, KomgaError> {
        self.request(Method::GET, path)
    }

    fn request(&self, method: Method, path: &[&str]) -> Result<RequestBuilder, KomgaError> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|()| KomgaError::BadUrl)?
            .pop_if_empty()
            .extend(path);
        Ok(self.client.request(method, url))
    }

    fn authorize(&self, request: RequestBuilder) -> RequestBuilder {
        match &self.auth {
            KomgaAuth::ApiKey { api_key } => request.header("X-API-Key", api_key),
            KomgaAuth::Basic { username, password } => request.basic_auth(username, Some(password)),
        }
    }

    async fn send(
        &self,
        request: RequestBuilder,
        action: &'static str,
    ) -> Result<Response, KomgaError> {
        let response = self.authorize(request).send().await.map_err(unreachable)?;
        check_status(response, action)
    }

    async fn json<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
        action: &'static str,
    ) -> Result<T, KomgaError> {
        let request = request.header(ACCEPT, "application/json");
        let response = self.send(request, action).await?;
        let body = read_capped(response, MAX_JSON_BYTES).await?;
        serde_json::from_slice(&body).map_err(|_| KomgaError::NotKomga)
    }
}

fn check_status(response: Response, action: &'static str) -> Result<Response, KomgaError> {
    match response.status() {
        status if status.is_success() => Ok(response),
        StatusCode::UNAUTHORIZED => Err(KomgaError::Unauthorized),
        StatusCode::FORBIDDEN => Err(KomgaError::Forbidden(action)),
        StatusCode::NOT_FOUND => Err(KomgaError::NotFound(
            "such item; it may have been removed".to_owned(),
        )),
        status => Err(KomgaError::Status(status)),
    }
}

async fn read_capped(mut response: Response, limit: usize) -> Result<Vec<u8>, KomgaError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(KomgaError::TooLarge(limit));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(unreachable)? {
        if body.len() + chunk.len() > limit {
            return Err(KomgaError::TooLarge(limit));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// reqwest's message repeats the URL, which the admin already knows; the
/// cause is what helps.
fn unreachable(e: reqwest::Error) -> KomgaError {
    let e = e.without_url();
    let mut message = e.to_string();
    let mut source = std::error::Error::source(&e);
    while let Some(cause) = source {
        message = format!("{message}: {cause}");
        source = cause.source();
    }
    KomgaError::Unreachable(message)
}
