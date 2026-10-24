//! Browsing a source through an extension the user installed: search, a
//! series and its chapters, a chapter's pages, and the images they name.
//!
//! Covers and pages come back as links to the image route here, so the app
//! never contacts the source's sites itself and the server fetches only
//! from the sites the extension declares.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chek_catalog::ExtensionInstall;
use chek_sources::{
    Chapter, ChapterOrder as SiteChapterOrder, Image, ReadingDirection, Source, SourceError, Status,
};
use serde::{Deserialize, Serialize};

use crate::api::{ApiError, AppState, blocking};
use crate::auth::CurrentUser;
use crate::installs::{not_installed, not_usable};

/// Longer queries are almost certainly not typed by a person.
const MAX_QUERY_CHARS: usize = 200;

/// How long clients may reuse an answer without asking again. A source's
/// series and chapters change a few times a day at most, and each fresh
/// answer costs a trip to its site, so clients keep them for a while;
/// `refresh` on the chapters route is how a client asks past its cache.
const SEARCH_MAX_AGE: &str = "private, max-age=600";
const SERIES_MAX_AGE: &str = "private, max-age=3600";
const CHAPTERS_MAX_AGE: &str = "private, max-age=600";
/// Short, since sources often hand out page links that expire.
const PAGES_MAX_AGE: &str = "private, max-age=600";
/// An image at a given address doesn't change.
const IMAGE_MAX_AGE: &str = "private, max-age=86400";

/// A JSON answer with the `Cache-Control` it may be kept for.
type Cached<T> = ([(header::HeaderName, HeaderValue); 1], Json<T>);

fn cached<T>(max_age: &'static str, body: T) -> Cached<T> {
    (
        [(header::CACHE_CONTROL, HeaderValue::from_static(max_age))],
        Json(body),
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSeriesItemDto {
    id: String,
    title: String,
    cover_url: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSeriesPageDto {
    items: Vec<RemoteSeriesItemDto>,
    has_more: bool,
}

#[derive(Serialize)]
struct ContributorDto {
    name: String,
    role: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSeriesDto {
    id: String,
    title: String,
    summary: Option<String>,
    cover_url: Option<String>,
    contributors: Vec<ContributorDto>,
    reading_direction: Option<&'static str>,
    status: Option<&'static str>,
    year: Option<u16>,
    tags: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteChapterDto {
    id: String,
    title: String,
    number: Option<String>,
    version: Option<String>,
    published: Option<String>,
}

impl From<Chapter> for RemoteChapterDto {
    fn from(chapter: Chapter) -> Self {
        Self {
            id: chapter.id,
            title: chapter.title,
            number: chapter.number,
            version: chapter.version,
            published: chapter.published,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteChapterPageDto {
    items: Vec<RemoteChapterDto>,
    offset: usize,
    total: usize,
}

/// Which end of a series a chapter page counts from.
#[derive(Deserialize, Default, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum ChapterOrder {
    /// Reading order, first chapter first.
    #[default]
    Asc,
    /// Newest first, as people usually look for what's new.
    Desc,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChaptersQuery {
    series_id: String,
    #[serde(default)]
    order: ChapterOrder,
    #[serde(default)]
    offset: usize,
    /// Starts the page just past this chapter instead of at `offset`, as
    /// a reader asks for the chapter after the one it's showing.
    after: Option<String>,
    limit: Option<usize>,
    /// Reads the list from the source again instead of the kept one.
    #[serde(default)]
    refresh: bool,
}

/// Chapters in one page unless the client asks for another size.
const DEFAULT_CHAPTER_LIMIT: usize = 100;
/// The most chapters one page holds.
const MAX_CHAPTER_LIMIT: usize = 500;
/// How long a complete chapter list is kept before the source is asked again.
const CHAPTER_LIST_TTL: Duration = Duration::from_secs(10 * 60);
/// How many series' lists are kept at once; the oldest goes first.
const CHAPTER_LISTS_KEPT: usize = 64;

/// Complete chapter lists read from sources lately, so paging through a
/// series, or reading it newest first, asks the source for the whole list
/// once rather than on every page. Sources hand chapters out in reading
/// order and, when paginated, only forwards through an opaque cursor, so a
/// page from the end or the middle needs the whole list anyway.
#[derive(Default)]
pub struct ChapterLists {
    entries: Mutex<HashMap<ChapterListKey, KeptChapters>>,
}

/// When a list was read, and the list.
type KeptChapters = (Instant, Arc<Vec<Chapter>>);

/// One user's install of an extension and a series in it. Sources are
/// loaded per user, with that user's cookies, so lists aren't shared.
#[derive(Hash, PartialEq, Eq, Clone)]
struct ChapterListKey {
    extension: String,
    user: i64,
    series: String,
}

impl ChapterLists {
    fn get(&self, key: &ChapterListKey) -> Option<Arc<Vec<Chapter>>> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries
            .get(key)
            .filter(|(read, _)| read.elapsed() < CHAPTER_LIST_TTL)
            .map(|(_, chapters)| Arc::clone(chapters))
    }

    fn insert(&self, key: ChapterListKey, chapters: Arc<Vec<Chapter>>) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|_, (read, _)| read.elapsed() < CHAPTER_LIST_TTL);
        if entries.len() >= CHAPTER_LISTS_KEPT
            && let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, (read, _))| *read)
                .map(|(key, _)| key.clone())
        {
            entries.remove(&oldest);
        }
        entries.insert(key, (Instant::now(), chapters));
    }
}

/// The chapters of `chapters` in one page: `limit` of them starting at
/// `offset`, counted from the end that `order` names.
fn chapter_page(
    chapters: &[Chapter],
    order: ChapterOrder,
    offset: usize,
    limit: usize,
) -> Vec<Chapter> {
    let start = offset.min(chapters.len());
    let end = offset.saturating_add(limit).min(chapters.len());
    match order {
        ChapterOrder::Asc => chapters[start..end].to_vec(),
        ChapterOrder::Desc => chapters[chapters.len() - end..chapters.len() - start]
            .iter()
            .rev()
            .cloned()
            .collect(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePageDto {
    index: usize,
    url: String,
}

#[derive(Serialize)]
pub struct List<T> {
    items: Vec<T>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    q: Option<String>,
    page: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesQuery {
    series_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterQuery {
    chapter_id: String,
}

#[derive(Deserialize)]
pub struct ImageQuery {
    r#ref: String,
}

/// What an image link carries: the request the extension asked for.
#[derive(Serialize, Deserialize)]
struct ImageRef {
    url: String,
    #[serde(default)]
    headers: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transform: Option<String>,
}

pub async fn search(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<SearchQuery>,
) -> Result<Cached<RemoteSeriesPageDto>, ApiError> {
    let q = query.q.unwrap_or_default().trim().to_owned();
    if q.chars().count() > MAX_QUERY_CHARS {
        return Err(ApiError::bad_request(format!(
            "q is longer than {MAX_QUERY_CHARS} characters"
        )));
    }
    let page = query.page.unwrap_or(1).max(1);
    let (_, source) = usable_source(&state, &current, &id).await?;
    let found = run(source, move |s| s.search(&q, page)).await?;
    Ok(cached(
        SEARCH_MAX_AGE,
        RemoteSeriesPageDto {
            items: found
                .items
                .into_iter()
                .map(|item| RemoteSeriesItemDto {
                    cover_url: item.cover.map(|c| image_url(&id, &c)),
                    id: item.id,
                    title: item.title,
                })
                .collect(),
            has_more: found.has_more,
        },
    ))
}

pub async fn series(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<SeriesQuery>,
) -> Result<Cached<RemoteSeriesDto>, ApiError> {
    let (_, source) = usable_source(&state, &current, &id).await?;
    let series = run(source, move |s| s.series(&query.series_id)).await?;
    Ok(cached(
        SERIES_MAX_AGE,
        RemoteSeriesDto {
            cover_url: series.cover.map(|c| image_url(&id, &c)),
            id: series.id,
            title: series.title,
            summary: series.summary,
            contributors: series
                .contributors
                .into_iter()
                .map(|c| ContributorDto {
                    name: c.name,
                    role: c.role,
                })
                .collect(),
            reading_direction: series.reading_direction.map(direction),
            status: series.status.map(status),
            year: series.year,
            tags: series.tags,
        },
    ))
}

/// One page of a series' chapters, from either end, and how many there
/// are. An extension that answers pages itself is asked for just that
/// page; otherwise the whole list is read from the source on the first
/// page and kept for the pages after it.
pub async fn chapters(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<ChaptersQuery>,
) -> Result<Cached<RemoteChapterPageDto>, ApiError> {
    let limit = query.limit.unwrap_or(DEFAULT_CHAPTER_LIMIT);
    if !(1..=MAX_CHAPTER_LIMIT).contains(&limit) {
        return Err(ApiError::bad_request(format!(
            "limit must be between 1 and {MAX_CHAPTER_LIMIT}"
        )));
    }
    let (_, source) = usable_source(&state, &current, &id).await?;
    if let Some(page) = extension_page(&source, &query, limit).await? {
        return Ok(cached(CHAPTERS_MAX_AGE, page));
    }
    let key = ChapterListKey {
        extension: id.clone(),
        user: current.user.id,
        series: query.series_id.clone(),
    };
    let kept = if query.refresh {
        None
    } else {
        state.chapter_lists.get(&key)
    };
    let chapters = match kept {
        Some(chapters) => chapters,
        None => {
            let series = query.series_id.clone();
            let chapters = Arc::new(run(source, move |s| s.chapters(&series)).await?);
            state.chapter_lists.insert(key, Arc::clone(&chapters));
            chapters
        }
    };
    let offset = match &query.after {
        None => query.offset,
        Some(after) => {
            let index = chapters
                .iter()
                .position(|chapter| &chapter.id == after)
                .ok_or_else(|| ApiError::not_found("the source has no such chapter".to_owned()))?;
            match query.order {
                ChapterOrder::Asc => index + 1,
                ChapterOrder::Desc => chapters.len() - index,
            }
        }
    };
    let items = chapter_page(&chapters, query.order, offset, limit);
    Ok(cached(
        CHAPTERS_MAX_AGE,
        RemoteChapterPageDto {
            items: items.into_iter().map(RemoteChapterDto::from).collect(),
            offset,
            total: chapters.len(),
        },
    ))
}

/// The page straight from an extension that answers pages itself, or
/// `None` to page through the complete list instead: when it doesn't, or
/// when it can't place the `after` chapter without reading everything.
async fn extension_page(
    source: &Arc<Source>,
    query: &ChaptersQuery,
    limit: usize,
) -> Result<Option<RemoteChapterPageDto>, ApiError> {
    let order = match query.order {
        ChapterOrder::Asc => SiteChapterOrder::OldestFirst,
        ChapterOrder::Desc => SiteChapterOrder::NewestFirst,
    };
    let offset =
        u32::try_from(query.offset).map_err(|_| ApiError::bad_request("offset is too large"))?;
    let series = query.series_id.clone();
    let after = query.after.clone();
    let limit = limit as u32;
    match run(Arc::clone(source), move |s| {
        s.chapter_page(&series, order, offset, limit, after.as_deref())
    })
    .await
    {
        Ok(page) => Ok(page.map(|page| RemoteChapterPageDto {
            items: page.items.into_iter().map(RemoteChapterDto::from).collect(),
            offset: page.offset as usize,
            total: page.total as usize,
        })),
        Err(error) if query.after.is_some() && error.status == StatusCode::NOT_FOUND => Ok(None),
        Err(error) => Err(error),
    }
}

pub async fn pages(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<ChapterQuery>,
) -> Result<Cached<List<RemotePageDto>>, ApiError> {
    let (_, source) = usable_source(&state, &current, &id).await?;
    let pages = run(source, move |s| s.pages(&query.chapter_id)).await?;
    Ok(cached(
        PAGES_MAX_AGE,
        List {
            items: pages
                .iter()
                .enumerate()
                .map(|(index, page)| RemotePageDto {
                    index,
                    url: image_url(&id, page),
                })
                .collect(),
        },
    ))
}

/// Fetches a cover or page named by a link from the routes above.
pub async fn image(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<ImageQuery>,
) -> Result<Response, ApiError> {
    let image = decode_image(&query.r#ref)?;
    let (_, source) = usable_source(&state, &current, &id).await?;
    let fetched = run(source, move |s| s.fetch_image(&image)).await?;
    let content_type = fetched
        .content_type
        .filter(|t| t.starts_with("image/"))
        .and_then(|t| HeaderValue::from_str(&t).ok())
        .unwrap_or(HeaderValue::from_static("application/octet-stream"));
    let mut response = Response::new(Body::from(fetched.bytes));
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, content_type);
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(IMAGE_MAX_AGE),
    );
    // Images come from sites the server doesn't control, so the browser
    // mustn't read them as anything but an image.
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

/// The user's install of an extension and its loaded source, refused while
/// the repository it came from isn't usable.
pub(crate) async fn usable_source(
    state: &AppState,
    current: &CurrentUser,
    id: &str,
) -> Result<(ExtensionInstall, Arc<Source>), ApiError> {
    let user_id = current.user.id;
    let lookup = id.to_owned();
    let (install, repository, allow_any) = blocking(state, move |c| {
        let install = c.extension_install(user_id, &lookup)?;
        let repository = match &install {
            Some(install) => c.extension_repository(install.repository_id)?,
            None => None,
        };
        Ok((install, repository, c.allow_any_repository()?))
    })
    .await?;
    let install = install.ok_or_else(|| not_installed(id))?;
    if repository.is_none_or(|r| r.owner_id.is_some() && !allow_any) {
        return Err(not_usable());
    }
    let extensions = Arc::clone(&state.extensions);
    let digest = install.sha256.clone();
    let source = tokio::task::spawn_blocking(move || extensions.source(&digest, user_id))
        .await
        .map_err(ApiError::internal)?
        .map_err(load_error)?;
    Ok((install, source))
}

/// Runs an extension call on a blocking thread, since extensions block.
pub(crate) async fn run<T, F>(source: Arc<Source>, call: F) -> Result<T, ApiError>
where
    F: FnOnce(&Source) -> Result<T, chek_sources::Error> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(move || call(&source))
        .await
        .map_err(ApiError::internal)?
        .map_err(source_error)
}

/// Why an installed extension couldn't be loaded. One built for another
/// contract version isn't the server's fault, and updating it fixes it.
pub(crate) fn load_error(error: chek_sources::Error) -> ApiError {
    match error {
        chek_sources::Error::Incompatible(_) => ApiError {
            status: StatusCode::BAD_GATEWAY,
            code: "source_unavailable",
            message: "this extension was built for another version of Chek; update it from its repository, or ask its author for a new build".to_owned(),
        },
        error => ApiError::internal(error),
    }
}

pub(crate) fn source_error(error: chek_sources::Error) -> ApiError {
    use chek_sources::Error;
    match error {
        Error::Source(SourceError::NotFound) => {
            ApiError::not_found("the source has no such item".to_owned())
        }
        Error::Source(_) | Error::Trap(_) | Error::Image(_) | Error::Pagination(_) => {
            tracing::warn!(error = %error, "source request failed");
            ApiError {
                status: StatusCode::BAD_GATEWAY,
                code: "source_failed",
                message: error.to_string(),
            }
        }
        _ => ApiError::internal(error),
    }
}

/// A link to the image route that fetches `image` through extension `id`.
pub(crate) fn image_url(id: &str, image: &Image) -> String {
    format!(
        "/api/v1/me/extensions/{id}/image?ref={}",
        URL_SAFE_NO_PAD.encode(image_json(image))
    )
}

fn decode_image(reference: &str) -> Result<Image, ApiError> {
    let invalid = || ApiError::bad_request("ref is not a valid image reference");
    let json = URL_SAFE_NO_PAD.decode(reference).map_err(|_| invalid())?;
    image_from_json(&json).ok_or_else(invalid)
}

/// An image request as JSON, the form links and stored pages keep it in.
pub(crate) fn image_json(image: &Image) -> String {
    let reference = ImageRef {
        url: image.url.clone(),
        headers: image.headers.clone(),
        transform: image.transform.clone(),
    };
    serde_json::to_string(&reference).unwrap_or_default()
}

pub(crate) fn image_from_json(json: &[u8]) -> Option<Image> {
    let reference: ImageRef = serde_json::from_slice(json).ok()?;
    Some(Image {
        url: reference.url,
        headers: reference.headers,
        transform: reference.transform,
    })
}

fn status(status: Status) -> &'static str {
    match status {
        Status::Ongoing => "ongoing",
        Status::Completed => "completed",
        Status::Hiatus => "hiatus",
        Status::Cancelled => "cancelled",
    }
}

fn direction(direction: ReadingDirection) -> &'static str {
    match direction {
        ReadingDirection::Ltr => "ltr",
        ReadingDirection::Rtl => "rtl",
        ReadingDirection::Ttb => "ttb",
    }
}
