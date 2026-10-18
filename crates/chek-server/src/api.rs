//! HTTP handlers. Routes, request and response shapes follow `openapi.yaml`
//! at the repository root; change both together.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use chek_catalog::{
    Catalog, CatalogError, Library, LibraryKind, PageRecord, PublicationFile, PublicationSummary,
    ScanStatus, SeriesSummary,
};
use chek_formats::{Compression, PageEntry};
use chek_library::ScanOptions;
use serde::{Deserialize, Serialize};
use tower::ServiceExt;

use crate::auth::{self, CurrentUser};
use crate::{annotations, extensions, installs, progress};
use tower_http::services::ServeFile;
use tracing::error;

#[derive(Clone)]
pub struct AppState {
    pub catalog: Arc<Catalog>,
    pub scan: ScanOptions,
    /// Present until the first admin is created; see [`crate::auth::setup`].
    pub setup_token: Arc<std::sync::Mutex<Option<String>>>,
    /// Basic auth checks that passed; see [`crate::auth`].
    pub basic_auth_cache: Arc<std::sync::Mutex<std::collections::HashSet<[u8; 32]>>>,
    pub password_limiter: Arc<crate::limit::PasswordLimiter>,
    /// Reverse proxies whose `X-Forwarded-For` is believed.
    pub trusted_proxies: Arc<Vec<crate::limit::IpRange>>,
    /// A header those proxies set to the client's address, believed before
    /// `X-Forwarded-For`.
    pub client_ip_header: Option<axum::http::HeaderName>,
    /// The largest file an upload may send.
    pub max_upload_bytes: u64,
    /// Installed extension components and the sources loaded from them.
    pub extensions: Arc<chek_sources::Extensions>,
}

/// Starts a background scan of every folder library.
pub async fn scan_all_folders(state: &AppState) -> Result<(), CatalogError> {
    let catalog = state.catalog.clone();
    let libraries = tokio::task::spawn_blocking(move || catalog.libraries())
        .await
        .expect("listing libraries does not panic")?;
    for library in libraries.iter().filter(|l| l.kind == LibraryKind::Folder) {
        start_scan(state, library.id);
    }
    Ok(())
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/v1/libraries", get(list_libraries).post(add_library))
        .route("/api/v1/libraries/{id}", get(get_library))
        .route("/api/v1/libraries/{id}/scan", post(scan_library))
        .route(
            "/api/v1/libraries/{id}/files/{*path}",
            put(crate::upload::upload),
        )
        .route(
            "/api/v1/libraries/{id}/publications",
            get(list_publications),
        )
        .route("/api/v1/libraries/{id}/series", get(list_series))
        .route("/api/v1/series/{id}", get(get_series))
        .route(
            "/api/v1/series/{id}/publications",
            get(list_series_publications),
        )
        .route("/api/v1/search", get(search))
        .route("/api/v1/setup", post(auth::setup))
        .route("/api/v1/auth/sign-in", post(auth::sign_in))
        .route("/api/v1/auth/sign-out", post(auth::sign_out))
        .route("/api/v1/me", get(auth::me))
        .route("/api/v1/me/password", put(auth::change_password))
        .route("/api/v1/me/sessions", get(auth::list_sessions))
        .route("/api/v1/me/continue-reading", get(continue_reading))
        .route("/api/v1/recently-added", get(recently_added))
        .route(
            "/api/v1/me/sessions/{id}",
            axum::routing::delete(auth::delete_session),
        )
        .route(
            "/api/v1/users",
            get(auth::list_users).post(auth::create_user),
        )
        .route(
            "/api/v1/users/{id}",
            patch(auth::update_user).delete(auth::delete_user),
        )
        .route(
            "/api/v1/publications/{id}",
            get(get_publication).patch(crate::edits::edit_publication),
        )
        .route(
            "/api/v1/publications/{id}/edits",
            axum::routing::delete(crate::edits::clear_edits),
        )
        .route("/api/v1/publications/{id}/file", get(get_file))
        .route("/api/v1/publications/{id}/cover", get(get_cover))
        .route("/api/v1/publications/{id}/pages", get(list_pages))
        .route("/api/v1/publications/{id}/pages/{index}", get(get_page))
        .route(
            "/api/v1/publications/{id}/progress",
            get(progress::get_progress).put(progress::save_progress),
        )
        .route(
            "/api/v1/publications/{id}/annotations",
            get(annotations::list_annotations).post(annotations::add_annotation),
        )
        .route(
            "/api/v1/annotations/{id}",
            patch(annotations::update_annotation).delete(annotations::delete_annotation),
        )
        .route(
            "/api/v1/extension-repositories",
            get(extensions::list_repositories).post(extensions::add_repository),
        )
        .route(
            "/api/v1/extension-repositories/{id}",
            axum::routing::delete(extensions::delete_repository),
        )
        .route(
            "/api/v1/extension-repositories/{id}/refresh",
            post(extensions::refresh_repository),
        )
        .route("/api/v1/extensions", get(extensions::list_extensions))
        .route("/api/v1/me/extensions", get(installs::list_installed))
        .route(
            "/api/v1/me/extensions/{id}",
            put(installs::install).delete(installs::uninstall),
        )
        .route(
            "/api/v1/settings",
            get(extensions::get_settings).put(extensions::update_settings),
        )
        .merge(crate::opds::router())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::telemetry::trace_request,
        ))
        .with_state(state)
}

const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 200;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LibraryDto {
    id: i64,
    name: String,
    kind: &'static str,
    path: Option<String>,
    scan: ScanDto,
    created_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanDto {
    status: &'static str,
    error: Option<String>,
    finished_at: Option<String>,
}

impl From<Library> for LibraryDto {
    fn from(l: Library) -> Self {
        Self {
            id: l.id,
            name: l.name,
            kind: match l.kind {
                LibraryKind::Folder => "folder",
                LibraryKind::Source => "source",
            },
            path: l.root_path,
            scan: ScanDto {
                status: match l.scan_status {
                    ScanStatus::Idle => "idle",
                    ScanStatus::Scanning => "scanning",
                    ScanStatus::Failed => "failed",
                },
                error: l.scan_error,
                finished_at: l.scanned_at,
            },
            created_at: l.created_at,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PublicationDto {
    id: i64,
    series: SeriesRefDto,
    title: String,
    number_label: Option<String>,
    format: String,
    page_count: Option<i64>,
    available: bool,
    has_cover: bool,
}

/// A publication with the metadata shown when it's opened.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PublicationDetailDto {
    #[serde(flatten)]
    summary: PublicationDto,
    /// Fields an admin changed by hand, which rescans keep.
    edited_fields: Vec<&'static str>,
    contributors: Vec<ContributorDto>,
    description: Option<String>,
    publisher: Option<String>,
    language: Option<String>,
    published: Option<String>,
    reading_direction: Option<String>,
}

#[derive(Serialize)]
struct ContributorDto {
    name: String,
    role: String,
}

#[derive(Serialize)]
struct SeriesRefDto {
    id: i64,
    title: String,
}

impl From<PublicationSummary> for PublicationDto {
    fn from(p: PublicationSummary) -> Self {
        Self {
            id: p.id,
            series: SeriesRefDto {
                id: p.series_id,
                title: p.series_title,
            },
            title: p.title,
            number_label: p.number_label,
            format: p.format,
            page_count: p.page_count,
            available: p.available,
            has_cover: p.has_cover,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Page<T> {
    items: Vec<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn list_libraries(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<Page<LibraryDto>>, ApiError> {
    let libraries = blocking(&state, |c| c.libraries()).await?;
    Ok(Json(Page {
        items: libraries
            .into_iter()
            .filter(|l| current.user.can_read(l.id))
            .map(Into::into)
            .collect(),
        next_cursor: None,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewLibrary {
    name: String,
    path: String,
}

async fn add_library(
    State(state): State<AppState>,
    current: CurrentUser,
    body: Result<Json<NewLibrary>, JsonRejection>,
) -> Result<(StatusCode, Json<LibraryDto>), ApiError> {
    current.require_admin()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let name = body.name.trim().to_owned();
    if name.is_empty() {
        return Err(ApiError::bad_request("name must not be empty"));
    }
    let path = PathBuf::from(&body.path);
    if !path.is_absolute() {
        return Err(ApiError::bad_request(
            "path must be an absolute path on the server",
        ));
    }
    let path = tokio::fs::canonicalize(&path)
        .await
        .map_err(|e| ApiError::bad_request(format!("cannot open {}: {e}", body.path)))?;
    if !path.is_dir() {
        return Err(ApiError::bad_request(format!(
            "{} is not a folder",
            path.display()
        )));
    }
    let Some(path) = path.to_str().map(str::to_owned) else {
        return Err(ApiError::bad_request("path must be valid UTF-8"));
    };

    let library = blocking(&state, move |c| c.add_folder_library(&name, &path)).await?;
    start_scan(&state, library.id);
    Ok((StatusCode::CREATED, Json(library.into())))
}

async fn get_library(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Json<LibraryDto>, ApiError> {
    let library = blocking(&state, move |c| c.library(id)).await?;
    library
        .filter(|l| current.user.can_read(l.id))
        .map(|l| Json(l.into()))
        .ok_or_else(|| ApiError::library_not_found(id))
}

async fn scan_library(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    current.require_admin()?;
    let library = blocking(&state, move |c| c.library(id))
        .await?
        .ok_or_else(|| ApiError::library_not_found(id))?;
    if library.scan_status == ScanStatus::Scanning {
        return Err(ApiError {
            status: StatusCode::CONFLICT,
            code: "scan_in_progress",
            message: format!("library {id} is already being scanned"),
        });
    }
    start_scan(&state, id);
    Ok(StatusCode::ACCEPTED)
}

#[derive(Deserialize)]
struct PageQuery {
    cursor: Option<String>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecentQuery {
    library_id: Option<i64>,
    cursor: Option<String>,
    limit: Option<u32>,
}

/// Newest publications first, across every readable library or one.
async fn recently_added(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(query): Query<RecentQuery>,
) -> Result<Json<Page<PublicationDto>>, ApiError> {
    let after = parse_cursor(query.cursor.as_deref())?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);
    let items = recent_publications(&state, &current, query.library_id, after, limit).await?;
    let next_cursor = (items.len() == limit as usize)
        .then(|| items.last().map(|p| p.id.to_string()))
        .flatten();
    Ok(Json(Page {
        items: items.into_iter().map(Into::into).collect(),
        next_cursor,
    }))
}

/// One page of recently added publications for the API and OPDS.
pub(crate) async fn recent_publications(
    state: &AppState,
    current: &CurrentUser,
    library_id: Option<i64>,
    after: Option<i64>,
    limit: u32,
) -> Result<Vec<PublicationSummary>, ApiError> {
    if let Some(id) = library_id
        && !current.user.can_read(id)
    {
        return Err(ApiError::library_not_found(id));
    }
    let user = current.user.clone();
    let found = blocking(state, move |c| {
        if let Some(id) = library_id
            && c.library(id)?.is_none()
        {
            return Ok(None);
        }
        Ok(Some(c.recently_added(&user, library_id, after, limit)?))
    })
    .await?;
    found.ok_or_else(|| ApiError::library_not_found(library_id.unwrap_or_default()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadingDto {
    publication: PublicationDto,
    progress: progress::ProgressDto,
}

/// Publications the user has started and not finished, most recently read
/// first.
async fn continue_reading(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(query): Query<PageQuery>,
) -> Result<Json<Page<ReadingDto>>, ApiError> {
    let after = parse_cursor(query.cursor.as_deref())?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);
    let user = current.user.clone();
    let items = blocking(&state, move |c| c.continue_reading(&user, after, limit)).await?;
    let next_cursor = (items.len() == limit as usize)
        .then(|| items.last().map(|(p, _)| p.id.to_string()))
        .flatten();
    let items = items
        .into_iter()
        .map(|(publication, progress)| {
            Ok(ReadingDto {
                publication: publication.into(),
                progress: progress.try_into()?,
            })
        })
        .collect::<Result<_, ApiError>>()?;
    Ok(Json(Page { items, next_cursor }))
}

async fn list_publications(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
    Query(query): Query<PageQuery>,
) -> Result<Json<Page<PublicationDto>>, ApiError> {
    if !current.user.can_read(id) {
        return Err(ApiError::library_not_found(id));
    }
    let after = match query.cursor.as_deref() {
        None => None,
        Some(c) => Some(
            c.parse::<i64>()
                .map_err(|_| ApiError::bad_request("cursor is not valid"))?,
        ),
    };
    let limit = query
        .limit
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);
    let (library, items) = blocking(&state, move |c| {
        let library = c.library(id)?;
        let items = match library {
            Some(_) => c.publications(id, after, limit)?,
            None => Vec::new(),
        };
        Ok((library, items))
    })
    .await?;
    if library.is_none() {
        return Err(ApiError::library_not_found(id));
    }

    let next_cursor = (items.len() == limit as usize)
        .then(|| items.last().map(|p| p.id.to_string()))
        .flatten();
    Ok(Json(Page {
        items: items.into_iter().map(Into::into).collect(),
        next_cursor,
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SeriesDto {
    id: i64,
    library_id: i64,
    title: String,
    publication_count: i64,
    cover_publication_id: Option<i64>,
}

impl From<SeriesSummary> for SeriesDto {
    fn from(s: SeriesSummary) -> Self {
        Self {
            id: s.id,
            library_id: s.library_id,
            title: s.title,
            publication_count: s.publication_count,
            cover_publication_id: s.cover_publication_id,
        }
    }
}

/// Reads a cursor, which is always the id of the last item on the previous
/// page.
pub(crate) fn parse_cursor(cursor: Option<&str>) -> Result<Option<i64>, ApiError> {
    cursor
        .map(|c| {
            c.parse::<i64>()
                .map_err(|_| ApiError::bad_request("cursor is not valid"))
        })
        .transpose()
}

/// Longer queries are almost certainly not typed by a person.
const MAX_QUERY_CHARS: usize = 200;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchQuery {
    q: Option<String>,
    library_id: Option<i64>,
    cursor: Option<String>,
    limit: Option<u32>,
}

async fn search(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Page<PublicationDto>>, ApiError> {
    let offset = parse_cursor(query.cursor.as_deref())?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);
    let items = search_publications(
        &state,
        &current,
        query.q.as_deref().unwrap_or_default(),
        query.library_id,
        offset,
        limit,
    )
    .await?;
    let next_cursor = (items.len() == limit as usize)
        .then(|| (offset.unwrap_or(0) + i64::from(limit)).to_string());
    Ok(Json(Page {
        items: items.into_iter().map(Into::into).collect(),
        next_cursor,
    }))
}

/// One page of search results for the API and OPDS. The cursor is the
/// number of results already returned.
pub(crate) async fn search_publications(
    state: &AppState,
    current: &CurrentUser,
    query: &str,
    library_id: Option<i64>,
    offset: Option<i64>,
    limit: u32,
) -> Result<Vec<PublicationSummary>, ApiError> {
    let query = query.trim().to_owned();
    if query.is_empty() {
        return Err(ApiError::bad_request("q is required"));
    }
    if query.chars().count() > MAX_QUERY_CHARS {
        return Err(ApiError::bad_request(format!(
            "q is longer than {MAX_QUERY_CHARS} characters"
        )));
    }
    let offset = u32::try_from(offset.unwrap_or(0))
        .map_err(|_| ApiError::bad_request("cursor is not valid"))?;
    if let Some(id) = library_id
        && !current.user.can_read(id)
    {
        return Err(ApiError::library_not_found(id));
    }
    let user = current.user.clone();
    let (library_found, items) = blocking(state, move |c| {
        if let Some(id) = library_id
            && c.library(id)?.is_none()
        {
            return Ok((false, Vec::new()));
        }
        Ok((true, c.search(&user, library_id, &query, offset, limit)?))
    })
    .await?;
    match (library_found, library_id) {
        (false, Some(id)) => Err(ApiError::library_not_found(id)),
        _ => Ok(items),
    }
}

async fn list_series(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
    Query(query): Query<PageQuery>,
) -> Result<Json<Page<SeriesDto>>, ApiError> {
    if !current.user.can_read(id) {
        return Err(ApiError::library_not_found(id));
    }
    let after = parse_cursor(query.cursor.as_deref())?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);
    let (library, items) = blocking(&state, move |c| {
        let library = c.library(id)?;
        let items = match library {
            Some(_) => c.series_list(id, after, limit)?,
            None => Vec::new(),
        };
        Ok((library, items))
    })
    .await?;
    if library.is_none() {
        return Err(ApiError::library_not_found(id));
    }
    let next_cursor = (items.len() == limit as usize)
        .then(|| items.last().map(|s| s.id.to_string()))
        .flatten();
    Ok(Json(Page {
        items: items.into_iter().map(Into::into).collect(),
        next_cursor,
    }))
}

/// A series the user may open; others answer as not found.
pub(crate) async fn readable_series(
    state: &AppState,
    current: &CurrentUser,
    id: i64,
) -> Result<SeriesSummary, ApiError> {
    blocking(state, move |c| c.series(id))
        .await?
        .filter(|s| current.user.can_read(s.library_id))
        .ok_or_else(|| ApiError::series_not_found(id))
}

async fn get_series(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Json<SeriesDto>, ApiError> {
    Ok(Json(readable_series(&state, &current, id).await?.into()))
}

async fn list_series_publications(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
    Query(query): Query<PageQuery>,
) -> Result<Json<Page<PublicationDto>>, ApiError> {
    readable_series(&state, &current, id).await?;
    let after = parse_cursor(query.cursor.as_deref())?;
    let limit = query
        .limit
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);
    let items = blocking(&state, move |c| c.series_publications(id, after, limit)).await?;
    let next_cursor = (items.len() == limit as usize)
        .then(|| items.last().map(|p| p.id.to_string()))
        .flatten();
    Ok(Json(Page {
        items: items.into_iter().map(Into::into).collect(),
        next_cursor,
    }))
}

async fn get_publication(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Json<PublicationDetailDto>, ApiError> {
    check_publication(&state, &current, id).await?;
    Ok(Json(publication_detail(&state, id).await?))
}

/// A publication with its metadata and which fields an admin edited.
pub(crate) async fn publication_detail(
    state: &AppState,
    id: i64,
) -> Result<PublicationDetailDto, ApiError> {
    let found = blocking(state, move |c| {
        let edited = c.edited_fields(id)?;
        Ok(c.publication(id)?
            .zip(c.publication_details(id)?)
            .map(|found| (found, edited)))
    })
    .await?;
    let ((publication, details), edited_fields) =
        found.ok_or_else(|| ApiError::publication_not_found(id))?;
    Ok(PublicationDetailDto {
        edited_fields,
        summary: publication.into(),
        contributors: details
            .contributors
            .into_iter()
            .map(|c| ContributorDto {
                name: c.name,
                role: c.role,
            })
            .collect(),
        description: details.summary,
        publisher: details.publisher,
        language: details.language,
        published: details.published,
        reading_direction: details.reading_direction,
    })
}

/// The media type of a publication's file.
pub(crate) fn file_media_type(format: &str) -> &'static str {
    match format {
        "epub" => "application/epub+zip",
        "pdf" => "application/pdf",
        "cbz" => "application/vnd.comicbook+zip",
        "cbt" => "application/x-cbt",
        "cbr" => "application/vnd.comicbook-rar",
        "cb7" => "application/x-cb7",
        _ => "application/octet-stream",
    }
}

/// The original file, with byte ranges and conditional requests.
pub(crate) async fn get_file(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
    request: Request,
) -> Result<Response, ApiError> {
    check_publication(&state, &current, id).await?;
    let (file, path) = current_file(&state, id).await?;
    let mut response = ServeFile::new(&path)
        .oneshot(request)
        .await
        .map_err(ApiError::internal)?
        .map(Body::new);
    let headers = response.headers_mut();
    // ServeFile guesses from the extension and calls a CBZ a RAR comic, so
    // the type comes from the detected format instead.
    let guessed = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());
    if guessed.is_some_and(|t| !t.starts_with("multipart/")) {
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(file_media_type(&file.format)),
        );
    }
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-cache"),
    );
    if let Ok(etag) = HeaderValue::from_str(&format!("\"{}\"", file.hash)) {
        headers.insert(header::ETAG, etag);
    }
    let name = file
        .relative_path
        .rsplit('/')
        .next()
        .unwrap_or(&file.relative_path);
    if let Ok(disposition) = HeaderValue::from_str(&content_disposition(name)) {
        headers.insert(header::CONTENT_DISPOSITION, disposition);
    }
    Ok(response)
}

pub(crate) async fn get_cover(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    check_publication(&state, &current, id).await?;
    let file = blocking(&state, move |c| c.publication_file(id))
        .await?
        .ok_or_else(|| ApiError::publication_not_found(id))?;
    let Some(cover) = file.cover else {
        return Err(ApiError::not_found(format!(
            "publication {id} has no cover"
        )));
    };
    let etag = format!("\"{cover}\"");
    if matches_etag(&headers, &etag) {
        return Ok(not_modified(&etag));
    }
    let path = state.scan.covers_dir.join(format!("{cover}.jpg"));
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // The covers folder was cleared; the next scan of a changed file
            // remakes it, so report it as missing for now.
            return Err(ApiError::not_found(format!(
                "publication {id} has no cover"
            )));
        }
        Err(e) => return Err(ApiError::internal(e)),
    };
    Ok(image_response(bytes, "image/jpeg", &etag))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PageDto {
    index: usize,
    media_type: &'static str,
    size: u64,
}

async fn list_pages(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Json<Page<PageDto>>, ApiError> {
    check_publication(&state, &current, id).await?;
    let pages = blocking(&state, move |c| c.pages(id)).await?;
    Ok(Json(Page {
        items: pages
            .into_iter()
            .enumerate()
            .map(|(index, p)| PageDto {
                index,
                media_type: chek_formats::page_media_type(&p.entry_name),
                size: p.size,
            })
            .collect(),
        next_cursor: None,
    }))
}

/// One comic page, read from its offset in the archive using the page index
/// stored at scan time.
async fn get_page(
    State(state): State<AppState>,
    current: CurrentUser,
    Path((id, index)): Path<(i64, u32)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    check_publication(&state, &current, id).await?;
    let page = blocking(&state, move |c| c.page(id, index))
        .await?
        .ok_or_else(|| ApiError::not_found(format!("publication {id} has no page {index}")))?;
    let (file, path) = current_file(&state, id).await?;
    let etag = format!("\"{}-{index}\"", file.hash);
    if matches_etag(&headers, &etag) {
        return Ok(not_modified(&etag));
    }

    let media_type = chek_formats::page_media_type(&page.entry_name);
    let entry = page_entry(page);
    let bytes = tokio::task::spawn_blocking(move || chek_formats::read_page(&path, &entry))
        .await
        .map_err(ApiError::internal)?;
    match bytes {
        Ok(bytes) => Ok(image_response(bytes, media_type, &etag)),
        // The file changed between the check and the read.
        Err(chek_formats::FormatError::Truncated) => Err(content_changed(&state, file.library_id)),
        Err(e) => Err(ApiError::internal(e)),
    }
}

/// Fails with not found unless the publication exists in a library the user
/// may read. Publications in other libraries look the same as missing ones.
pub async fn check_publication(
    state: &AppState,
    current: &CurrentUser,
    id: i64,
) -> Result<(), ApiError> {
    match blocking(state, move |c| c.publication_library(id)).await? {
        Some(library_id) if current.user.can_read(library_id) => Ok(()),
        _ => Err(ApiError::publication_not_found(id)),
    }
}

/// Loads a publication's file and checks it is still the file the last scan
/// read. If it changed, a rescan starts and the request gets a conflict, since
/// stored offsets and hashes no longer describe it.
async fn current_file(state: &AppState, id: i64) -> Result<(PublicationFile, PathBuf), ApiError> {
    let file = blocking(state, move |c| c.publication_file(id))
        .await?
        .ok_or_else(|| ApiError::publication_not_found(id))?;
    if !file.available {
        return Err(ApiError {
            status: StatusCode::NOT_FOUND,
            code: "unavailable",
            message: format!("publication {id}'s file is missing from its library folder"),
        });
    }
    let path = PathBuf::from(&file.root_path).join(&file.relative_path);
    let stamp = {
        let path = path.clone();
        tokio::task::spawn_blocking(move || chek_library::file_stamp(&path))
            .await
            .map_err(ApiError::internal)?
    };
    match (stamp, file.stamp) {
        (Ok(on_disk), Some(scanned)) if on_disk == scanned => Ok((file, path)),
        _ => Err(content_changed(state, file.library_id)),
    }
}

fn content_changed(state: &AppState, library_id: i64) -> ApiError {
    start_scan(state, library_id);
    ApiError {
        status: StatusCode::CONFLICT,
        code: "content_changed",
        message:
            "the file changed since it was scanned; a rescan has started, so try again shortly"
                .to_owned(),
    }
}

fn page_entry(page: PageRecord) -> PageEntry {
    PageEntry {
        name: page.entry_name,
        data_offset: page.data_offset,
        compressed_size: page.compressed_size,
        size: page.size,
        compression: match page.compression {
            "deflated" => Compression::Deflated,
            "archive" => Compression::Archive,
            _ => Compression::Stored,
        },
    }
}

fn image_response(bytes: Vec<u8>, media_type: &'static str, etag: &str) -> Response {
    let mut response = Response::new(Body::from(bytes));
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(media_type));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    if let Ok(etag) = HeaderValue::from_str(etag) {
        headers.insert(header::ETAG, etag);
    }
    response
}

fn not_modified(etag: &str) -> Response {
    let mut response = StatusCode::NOT_MODIFIED.into_response();
    if let Ok(etag) = HeaderValue::from_str(etag) {
        response.headers_mut().insert(header::ETAG, etag);
    }
    response
}

fn matches_etag(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|tag| tag.trim().trim_start_matches("W/"))
        .any(|tag| tag == etag || tag == "*")
}

/// `attachment` with the file's name, encoded per RFC 6266 so names outside
/// ASCII survive.
fn content_disposition(name: &str) -> String {
    let mut encoded = String::with_capacity(name.len());
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    let fallback: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

pub fn start_scan(state: &AppState, library_id: i64) {
    let catalog = state.catalog.clone();
    let options = state.scan.clone();
    tokio::spawn(async move {
        match chek_library::scan(catalog, library_id, options).await {
            Ok(_) | Err(chek_library::ScanError::AlreadyScanning(_)) => {}
            Err(e) => error!(library_id, error = %e, "scan failed"),
        }
    });
}

/// Runs a catalog call on a blocking thread, since SQLite calls block.
pub async fn blocking<T, F>(state: &AppState, f: F) -> Result<T, ApiError>
where
    F: FnOnce(&Catalog) -> Result<T, CatalogError> + Send + 'static,
    T: Send + 'static,
{
    let catalog = state.catalog.clone();
    match tokio::task::spawn_blocking(move || f(&catalog)).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(CatalogError::NameTaken(name))) => Err(ApiError {
            status: StatusCode::CONFLICT,
            code: "name_taken",
            message: format!("a user named {name:?} already exists"),
        }),
        Ok(Err(CatalogError::RepositoryExists(url))) => Err(ApiError {
            status: StatusCode::CONFLICT,
            code: "repository_exists",
            message: format!("the repository {url} has already been added"),
        }),
        Ok(Err(e)) => Err(ApiError::internal(e)),
        Err(e) => Err(ApiError::internal(e)),
    }
}

/// An error response: `{ "code": "...", "message": "..." }`.
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "bad_request",
            message: message.into(),
        }
    }

    pub fn not_found(message: String) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message,
        }
    }

    pub fn library_not_found(id: i64) -> Self {
        Self::not_found(format!("library {id} does not exist"))
    }

    pub fn series_not_found(id: i64) -> Self {
        Self::not_found(format!("series {id} does not exist"))
    }

    pub fn publication_not_found(id: i64) -> Self {
        Self::not_found(format!("publication {id} does not exist"))
    }

    pub fn internal(e: impl std::fmt::Display) -> Self {
        error!(error = %e, "request failed");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal",
            message: "the server could not complete the request; see its log".to_owned(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({ "code": self.code, "message": self.message });
        (self.status, Json(body)).into_response()
    }
}
