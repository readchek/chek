//! Browsing a source through an extension the user installed: search, a
//! series and its chapters, a chapter's pages or file, and the images and
//! files they name.
//!
//! Covers, pages and files come back as links to the image and file routes
//! here, so the app never contacts the source's sites itself and the server
//! fetches only from the sites the extension declares.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chek_catalog::ExtensionInstall;
use chek_sources::{
    Asset, Chapter, ChapterOrder as SiteChapterOrder, Content, ContentFormat, Fetched, Image,
    ReadingDirection, Rights, Source, SourceError, Status,
};
use serde::{Deserialize, Serialize};

use crate::api::{ApiError, AppState, blocking, content_disposition, matches_etag, not_modified};
use crate::auth::CurrentUser;
use crate::content_ratings::{self, content_ratings};
use crate::installs::{not_installed, not_usable};
use crate::releases::{Group, Pager, chapter_count, group_list, list_page, order_releases};

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
/// A file's id changes when the file does, so its bytes can be kept as
/// long as an image's.
const FILE_MAX_AGE: &str = "private, max-age=86400";

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
    content_rating: Option<&'static str>,
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
    content_rating: Option<&'static str>,
}

/// One release of a chapter, as one group put it out.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RemoteReleaseDto {
    id: String,
    title: String,
    number: Option<String>,
    version: Option<String>,
    published: Option<String>,
    language: Option<String>,
    release_group: Option<String>,
    official: bool,
}

impl From<Chapter> for RemoteReleaseDto {
    fn from(chapter: Chapter) -> Self {
        Self {
            id: chapter.id,
            title: chapter.title,
            number: chapter.number,
            version: chapter.version,
            published: chapter.published,
            language: chapter.language,
            release_group: chapter.release_group,
            official: chapter.official,
        }
    }
}

/// A chapter: its picked release's fields, and every release of it with
/// the picked one first.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteChapterDto {
    #[serde(flatten)]
    release: RemoteReleaseDto,
    releases: Vec<RemoteReleaseDto>,
}

impl RemoteChapterDto {
    fn new(mut group: Group, preferred: Option<&str>) -> Self {
        order_releases(&mut group.releases, preferred);
        let releases: Vec<RemoteReleaseDto> = group
            .releases
            .into_iter()
            .map(RemoteReleaseDto::from)
            .collect();
        Self {
            // A group always has a release.
            release: releases[0].clone(),
            releases,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteChapterPageDto {
    items: Vec<RemoteChapterDto>,
    /// The release position the page starts at.
    offset: usize,
    /// Where the next page starts, or `None` at the end.
    next_offset: Option<usize>,
    /// How many releases the series has.
    total: usize,
    /// How many chapters, when the whole list has been read.
    chapter_count: Option<usize>,
    preferred_group: Option<String>,
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
    /// Starts the page just past the chapter this release belongs to
    /// instead of at `offset`, as a reader asks for the chapter after the
    /// one it's showing.
    after: Option<String>,
    limit: Option<usize>,
    /// Only releases in this language, a BCP 47 tag, and those whose
    /// language the source doesn't give.
    language: Option<String>,
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
/// The most pages of releases one page of chapters asks an extension for.
/// It stops sooner once a chapter is complete, so this only matters for a
/// chapter with releases across many of the site's pages.
const MAX_EXTENSION_PAGES: usize = 8;
/// Longer than any BCP 47 tag a person would ask for.
const MAX_LANGUAGE_CHARS: usize = 35;
/// Bounds on what a release group preference stores.
const MAX_SERIES_ID_BYTES: usize = 1024;
const MAX_GROUP_BYTES: usize = 200;

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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePageDto {
    index: usize,
    url: String,
}

/// How to read a chapter: its pages, or one file such as a book.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterContentDto {
    /// `images`, `epub` or `pdf`.
    format: &'static str,
    /// Set for `images`.
    pages: Option<Vec<RemotePageDto>>,
    /// Set for a file.
    file: Option<RemoteFileDto>,
    rights: Option<RightsDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFileDto {
    id: String,
    media_type: &'static str,
    url: String,
    size: Option<u64>,
    file_name: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RightsDto {
    license: Option<String>,
    license_url: Option<String>,
    attribution: Option<String>,
    source_url: Option<String>,
}

impl From<Rights> for RightsDto {
    fn from(rights: Rights) -> Self {
        Self {
            license: rights.license,
            license_url: rights.license_url,
            attribution: rights.attribution,
            source_url: rights.source_url,
        }
    }
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

/// What a file link carries: the download the extension asked for.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetRef {
    id: String,
    format: String,
    url: String,
    #[serde(default)]
    headers: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file_name: Option<String>,
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
    let ratings = content_ratings(&state, &current).await?;
    let found = run(source, move |s| s.search(&q, page, &ratings)).await?;
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
                    content_rating: item.content_rating.map(content_ratings::name),
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
            content_rating: series.content_rating.map(content_ratings::name),
        },
    ))
}

/// One page of a series' chapters, from either end, each with its
/// releases and one of them picked. An extension that answers pages itself
/// is asked for just the releases the page needs; otherwise the whole list
/// is read from the source on the first page and kept for the pages after
/// it.
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
    if query
        .language
        .as_ref()
        .is_some_and(|tag| tag.is_empty() || tag.len() > MAX_LANGUAGE_CHARS)
    {
        return Err(ApiError::bad_request(
            "language must be a BCP 47 tag such as en or pt-BR",
        ));
    }
    let (_, source) = usable_source(&state, &current, &id).await?;
    let preferred = preferred_group(&state, &current, &id, &query.series_id).await?;
    let language = query.language.as_deref();
    if query.after.is_none()
        && let Some((groups, offset, next_offset, total)) =
            extension_page(&source, &query, limit).await?
    {
        let items = groups
            .into_iter()
            .map(|group| RemoteChapterDto::new(group, preferred.as_deref()))
            .collect();
        return Ok(cached(
            CHAPTERS_MAX_AGE,
            RemoteChapterPageDto {
                items,
                offset,
                next_offset,
                total,
                chapter_count: None,
                preferred_group: preferred,
            },
        ));
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
    let groups = group_list(&chapters, query.order == ChapterOrder::Desc);
    let count = chapter_count(&groups, language);
    let offset = match &query.after {
        None => query.offset,
        Some(after) => groups
            .iter()
            .find(|group| group.releases.iter().any(|r| &r.id == after))
            .map(|group| group.start + group.releases.len())
            .ok_or_else(|| ApiError::not_found("the source has no such chapter".to_owned()))?,
    };
    let (page, next_offset) = list_page(groups, offset, limit, language);
    Ok(cached(
        CHAPTERS_MAX_AGE,
        RemoteChapterPageDto {
            offset: page.first().map_or(offset, |group| group.start),
            items: page
                .into_iter()
                .map(|group| RemoteChapterDto::new(group, preferred.as_deref()))
                .collect(),
            next_offset,
            total: chapters.len(),
            chapter_count: Some(count),
            preferred_group: preferred,
        },
    ))
}

/// A page straight from an extension that answers pages itself, or `None`
/// to page through the complete list instead. The extension pages by
/// release, so this asks for pages until at least one chapter is known to
/// be complete, or the page is full, or the list ends, and leaves a chapter
/// that may continue past what was read for the next page. Returns the
/// chapters, where they start, where the next page starts and how many
/// releases the series has.
async fn extension_page(
    source: &Arc<Source>,
    query: &ChaptersQuery,
    limit: usize,
) -> Result<Option<(Vec<Group>, usize, Option<usize>, usize)>, ApiError> {
    let order = match query.order {
        ChapterOrder::Asc => SiteChapterOrder::OldestFirst,
        ChapterOrder::Desc => SiteChapterOrder::NewestFirst,
    };
    let mut pager = Pager::new(limit, query.language.as_deref());
    let mut position = query.offset;
    let mut total = None;
    let mut end = false;
    for _ in 0..MAX_EXTENSION_PAGES {
        let offset =
            u32::try_from(position).map_err(|_| ApiError::bad_request("offset is too large"))?;
        let series = query.series_id.clone();
        let Some(page) = run(Arc::clone(source), move |s| {
            s.chapter_page(&series, order, offset, limit as u32, None)
        })
        .await?
        else {
            return Ok(None);
        };
        let total = *total.get_or_insert(page.total as usize);
        let start = page.offset as usize;
        let read = page.items.len();
        let mut full = false;
        for (index, chapter) in page.items.into_iter().enumerate() {
            if !pager.push(start + index, chapter) {
                full = true;
                break;
            }
        }
        position = start + read;
        end = read == 0 || position >= total;
        if full || end || pager.has_complete() {
            break;
        }
    }
    let (groups, next) = pager.finish(position, end);
    let offset = groups.first().map_or(query.offset, |group| group.start);
    Ok(Some((groups, offset, next, total.unwrap_or(0))))
}

/// The release group the user prefers for a series. Demo accounts keep
/// none; see [`set_release_group`].
async fn preferred_group(
    state: &AppState,
    current: &CurrentUser,
    extension: &str,
    series: &str,
) -> Result<Option<String>, ApiError> {
    if current.demo {
        return Ok(None);
    }
    let user = current.user.id;
    let (extension, series) = (extension.to_owned(), series.to_owned());
    blocking(state, move |c| {
        c.release_group_preference(user, &extension, &series)
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseGroupBody {
    series_id: String,
    /// `None` goes back to the default pick.
    release_group: Option<String>,
}

/// Sets the release group picked first in a series' chapters, or clears it.
pub async fn set_release_group(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<ReleaseGroupBody>,
) -> Result<StatusCode, ApiError> {
    if body.series_id.is_empty() || body.series_id.len() > MAX_SERIES_ID_BYTES {
        return Err(ApiError::bad_request(format!(
            "seriesId must be between 1 and {MAX_SERIES_ID_BYTES} bytes"
        )));
    }
    let group = body
        .release_group
        .map(|group| group.trim().to_owned())
        .filter(|group| !group.is_empty());
    if group
        .as_ref()
        .is_some_and(|group| group.len() > MAX_GROUP_BYTES)
    {
        return Err(ApiError::bad_request(format!(
            "releaseGroup is longer than {MAX_GROUP_BYTES} bytes"
        )));
    }
    let user = current.user.id;
    let installed = {
        let id = id.clone();
        blocking(&state, move |c| c.extension_install(user, &id)).await?
    };
    if installed.is_none() {
        return Err(not_installed(&id));
    }
    if current.demo {
        // Everyone trying the demo shares its account, so a kept choice
        // would be a stranger's. It's answered as kept, and the app keeps
        // it on the device.
        return Ok(StatusCode::NO_CONTENT);
    }
    blocking(&state, move |c| {
        c.set_release_group_preference(user, &id, &body.series_id, group.as_deref())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn pages(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<ChapterQuery>,
) -> Result<Cached<List<RemotePageDto>>, ApiError> {
    let (_, source) = usable_source(&state, &current, &id).await?;
    let pages = run(Arc::clone(&source), move |s| s.pages(&query.chapter_id)).await?;
    source.read_ahead(pages.clone());
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

/// How to read a chapter: its pages, as the pages route lists them, or a
/// link to the file route for a book or other file.
pub async fn content(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<ChapterQuery>,
) -> Result<Cached<ChapterContentDto>, ApiError> {
    let (_, source) = usable_source(&state, &current, &id).await?;
    let reading = run(source, move |s| s.content(&query.chapter_id)).await?;
    let rights = reading.rights.map(RightsDto::from);
    let dto = match reading.content {
        Content::Pages(pages) => ChapterContentDto {
            format: chek_sources::format_name(ContentFormat::Images),
            pages: Some(
                pages
                    .iter()
                    .enumerate()
                    .map(|(index, page)| RemotePageDto {
                        index,
                        url: image_url(&id, page),
                    })
                    .collect(),
            ),
            file: None,
            rights,
        },
        Content::File(asset) => ChapterContentDto {
            format: chek_sources::format_name(asset.format),
            pages: None,
            file: Some(RemoteFileDto {
                url: format!(
                    "/api/v1/me/extensions/{id}/file?ref={}",
                    URL_SAFE_NO_PAD.encode(asset_json(&asset))
                ),
                media_type: chek_sources::media_type(asset.format),
                id: asset.id,
                size: asset.size,
                file_name: asset.file_name,
            }),
            rights,
        },
    };
    // Page links expire; a file link is the same as long as the file is.
    Ok(cached(PAGES_MAX_AGE, dto))
}

/// Downloads a file named by a link from the content route.
pub async fn file(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<ImageQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let asset = decode_asset(&query.r#ref)?;
    let etag = file_etag(&asset);
    if matches_etag(&headers, &etag) {
        return Ok(not_modified(&etag));
    }
    let (_, source) = usable_source(&state, &current, &id).await?;
    let response_asset = asset.clone();
    let fetched = run(source, move |s| s.fetch_asset(&asset)).await?;
    Ok(file_response(fetched, &response_asset, FILE_MAX_AGE))
}

/// A downloaded file as a response: the media type of the format the
/// extension named, which the host checked the bytes against, its name,
/// and an ETag from its id.
pub(crate) fn file_response(fetched: Fetched, asset: &Asset, max_age: &'static str) -> Response {
    let mut response = Response::new(Body::from(fetched.bytes));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(chek_sources::media_type(asset.format)),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(max_age));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if let Ok(etag) = HeaderValue::from_str(&file_etag(asset)) {
        headers.insert(header::ETAG, etag);
    }
    let name = asset
        .file_name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| format!("{}.{}", asset.id, chek_sources::format_name(asset.format)));
    if let Ok(disposition) = HeaderValue::from_str(&content_disposition(&name)) {
        headers.insert(header::CONTENT_DISPOSITION, disposition);
    }
    response
}

/// Names a file by the extension's id for it and where it's fetched from,
/// so a client holding it can ask whether it changed without a download.
fn file_etag(asset: &Asset) -> String {
    let hash = blake3::hash(asset_json(asset).as_bytes());
    format!("\"{}\"", &hash.to_hex()[..32])
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
        Error::Source(_)
        | Error::Trap(_)
        | Error::Image(_)
        | Error::Pagination(_)
        | Error::Content(_)
        | Error::Asset(_) => {
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

fn decode_asset(reference: &str) -> Result<Asset, ApiError> {
    let invalid = || ApiError::bad_request("ref is not a valid file reference");
    let json = URL_SAFE_NO_PAD.decode(reference).map_err(|_| invalid())?;
    let reference: AssetRef = serde_json::from_slice(&json).map_err(|_| invalid())?;
    let format = match reference.format.as_str() {
        "epub" => ContentFormat::Epub,
        "pdf" => ContentFormat::Pdf,
        _ => return Err(invalid()),
    };
    Ok(Asset {
        id: reference.id,
        format,
        url: reference.url,
        headers: reference.headers,
        size: reference.size,
        file_name: reference.file_name,
    })
}

/// A file request as JSON, the form file links carry it in.
fn asset_json(asset: &Asset) -> String {
    let reference = AssetRef {
        id: asset.id.clone(),
        format: chek_sources::format_name(asset.format).to_owned(),
        url: asset.url.clone(),
        headers: asset.headers.clone(),
        size: asset.size,
        file_name: asset.file_name.clone(),
    };
    serde_json::to_string(&reference).unwrap_or_default()
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
