//! Browsing a source through an extension the user installed: search, a
//! series and its chapters, a chapter's pages, and the images they name.
//!
//! Covers and pages come back as links to the image route here, so the app
//! never contacts the source's sites itself and the server fetches only
//! from the sites the extension declares.

use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chek_catalog::ExtensionInstall;
use chek_sources::{Chapter, Image, ReadingDirection, Source, SourceError};
use serde::{Deserialize, Serialize};

use crate::api::{ApiError, AppState, blocking};
use crate::auth::CurrentUser;
use crate::installs::{not_installed, not_usable};

/// Longer queries are almost certainly not typed by a person.
const MAX_QUERY_CHARS: usize = 200;

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
pub struct RemoteChapterBatchDto {
    items: Vec<RemoteChapterDto>,
    next_cursor: Option<String>,
    total: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterBatchQuery {
    series_id: String,
    cursor: Option<String>,
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
) -> Result<Json<RemoteSeriesPageDto>, ApiError> {
    let q = query.q.unwrap_or_default().trim().to_owned();
    if q.chars().count() > MAX_QUERY_CHARS {
        return Err(ApiError::bad_request(format!(
            "q is longer than {MAX_QUERY_CHARS} characters"
        )));
    }
    let page = query.page.unwrap_or(1).max(1);
    let (_, source) = usable_source(&state, &current, &id).await?;
    let found = run(source, move |s| s.search(&q, page)).await?;
    Ok(Json(RemoteSeriesPageDto {
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
    }))
}

pub async fn series(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<SeriesQuery>,
) -> Result<Json<RemoteSeriesDto>, ApiError> {
    let (_, source) = usable_source(&state, &current, &id).await?;
    let series = run(source, move |s| s.series(&query.series_id)).await?;
    Ok(Json(RemoteSeriesDto {
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
    }))
}

pub async fn chapters(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<SeriesQuery>,
) -> Result<Json<List<RemoteChapterDto>>, ApiError> {
    let (_, source) = usable_source(&state, &current, &id).await?;
    let chapters = run(source, move |s| s.chapters(&query.series_id)).await?;
    Ok(Json(List {
        items: chapters.into_iter().map(RemoteChapterDto::from).collect(),
    }))
}

pub async fn chapter_batch(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<ChapterBatchQuery>,
) -> Result<Json<RemoteChapterBatchDto>, ApiError> {
    if query
        .cursor
        .as_ref()
        .is_some_and(|cursor| cursor.is_empty() || cursor.len() > 4096)
    {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_cursor",
            message: "chapter cursor must contain 1–4096 bytes".into(),
        });
    }
    let (_, source) = usable_source(&state, &current, &id).await?;
    let batch = run(source, move |source| {
        source.chapter_batch(&query.series_id, query.cursor.as_deref())
    })
    .await?;
    Ok(Json(RemoteChapterBatchDto {
        items: batch
            .items
            .into_iter()
            .map(RemoteChapterDto::from)
            .collect(),
        next_cursor: batch.next,
        total: batch.total,
    }))
}

pub async fn pages(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<ChapterQuery>,
) -> Result<Json<List<RemotePageDto>>, ApiError> {
    let (_, source) = usable_source(&state, &current, &id).await?;
    let pages = run(source, move |s| s.pages(&query.chapter_id)).await?;
    Ok(Json(List {
        items: pages
            .iter()
            .enumerate()
            .map(|(index, page)| RemotePageDto {
                index,
                url: image_url(&id, page),
            })
            .collect(),
    }))
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
        HeaderValue::from_static("private, max-age=3600"),
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
        .map_err(ApiError::internal)?;
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

pub(crate) fn source_error(error: chek_sources::Error) -> ApiError {
    use chek_sources::Error;
    match error {
        Error::Source(SourceError::NotFound) => {
            ApiError::not_found("the source has no such item".to_owned())
        }
        Error::Source(_) | Error::Trap(_) | Error::Image(_) | Error::Pagination(_) => ApiError {
            status: StatusCode::BAD_GATEWAY,
            code: "source_failed",
            message: error.to_string(),
        },
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

fn direction(direction: ReadingDirection) -> &'static str {
    match direction {
        ReadingDirection::Ltr => "ltr",
        ReadingDirection::Rtl => "rtl",
        ReadingDirection::Ttb => "ttb",
    }
}
