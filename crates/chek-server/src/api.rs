//! HTTP handlers. Routes, request and response shapes follow `openapi.yaml`
//! at the repository root; change both together.

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chek_catalog::{Catalog, CatalogError, Library, LibraryKind, PublicationSummary, ScanStatus};
use serde::{Deserialize, Serialize};
use tracing::error;

#[derive(Clone)]
pub struct AppState {
    pub catalog: Arc<Catalog>,
    pub scan_workers: usize,
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

fn start_scan(state: &AppState, library_id: i64) {
    let catalog = state.catalog.clone();
    let workers = state.scan_workers;
    tokio::spawn(async move {
        if let Err(e) = chek_library::scan(catalog, library_id, workers).await {
            error!(library_id, error = %e, "scan failed");
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

    fn library_not_found(id: i64) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: format!("library {id} does not exist"),
        }
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
