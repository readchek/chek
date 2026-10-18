//! HTTP handlers. Routes, request and response shapes follow `openapi.yaml`
//! at the repository root; change both together.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chek_catalog::{
    Catalog, CatalogError, Library, LibraryKind, PageRecord, PublicationFile, PublicationSummary,
    ScanStatus,
};
use chek_formats::{Compression, PageEntry};
use chek_library::ScanOptions;
use serde::{Deserialize, Serialize};
use tower::ServiceExt;
use tower_http::services::ServeFile;
use tracing::error;

#[derive(Clone)]
pub struct AppState {
    pub catalog: Arc<Catalog>,
    pub scan: ScanOptions,
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
            "/api/v1/libraries/{id}/publications",
            get(list_publications),
        )
        .route("/api/v1/publications/{id}", get(get_publication))
        .route("/api/v1/publications/{id}/file", get(get_file))
        .route("/api/v1/publications/{id}/cover", get(get_cover))
        .route("/api/v1/publications/{id}/pages", get(list_pages))
        .route("/api/v1/publications/{id}/pages/{index}", get(get_page))
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
struct PublicationDto {
    id: i64,
    series: SeriesRefDto,
    title: String,
    number_label: Option<String>,
    format: String,
    page_count: Option<i64>,
    available: bool,
    has_cover: bool,
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

async fn list_libraries(State(state): State<AppState>) -> Result<Json<Page<LibraryDto>>, ApiError> {
    let libraries = blocking(&state, |c| c.libraries()).await?;
    Ok(Json(Page {
        items: libraries.into_iter().map(Into::into).collect(),
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
    body: Result<Json<NewLibrary>, JsonRejection>,
) -> Result<(StatusCode, Json<LibraryDto>), ApiError> {
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
    Path(id): Path<i64>,
) -> Result<Json<LibraryDto>, ApiError> {
    let library = blocking(&state, move |c| c.library(id)).await?;
    library
        .map(|l| Json(l.into()))
        .ok_or_else(|| ApiError::library_not_found(id))
}

async fn scan_library(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
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

async fn list_publications(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<PageQuery>,
) -> Result<Json<Page<PublicationDto>>, ApiError> {
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

async fn get_publication(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<PublicationDto>, ApiError> {
    let publication = blocking(&state, move |c| c.publication(id)).await?;
    publication
        .map(|p| Json(p.into()))
        .ok_or_else(|| ApiError::publication_not_found(id))
}

/// The original file, with byte ranges and conditional requests.
async fn get_file(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    request: Request,
) -> Result<Response, ApiError> {
    let (file, path) = current_file(&state, id).await?;
    let mut response = ServeFile::new(&path)
        .oneshot(request)
        .await
        .map_err(ApiError::internal)?
        .map(Body::new);
    let headers = response.headers_mut();
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

async fn get_cover(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
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
    Path(id): Path<i64>,
) -> Result<Json<Page<PageDto>>, ApiError> {
    let (publication, pages) =
        blocking(&state, move |c| Ok((c.publication(id)?, c.pages(id)?))).await?;
    if publication.is_none() {
        return Err(ApiError::publication_not_found(id));
    }
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
    Path((id, index)): Path<(i64, u32)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
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
        compression: if page.compression == "deflated" {
            Compression::Deflated
        } else {
            Compression::Stored
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
async fn blocking<T, F>(state: &AppState, f: F) -> Result<T, ApiError>
where
    F: FnOnce(&Catalog) -> Result<T, CatalogError> + Send + 'static,
    T: Send + 'static,
{
    let catalog = state.catalog.clone();
    match tokio::task::spawn_blocking(move || f(&catalog)).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(ApiError::internal(e)),
        Err(e) => Err(ApiError::internal(e)),
    }
}

/// An error response: `{ "code": "...", "message": "..." }`.
struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "bad_request",
            message: message.into(),
        }
    }

    fn not_found(message: String) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message,
        }
    }

    fn library_not_found(id: i64) -> Self {
        Self::not_found(format!("library {id} does not exist"))
    }

    fn publication_not_found(id: i64) -> Self {
        Self::not_found(format!("publication {id} does not exist"))
    }

    fn internal(e: impl std::fmt::Display) -> Self {
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
