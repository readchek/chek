//! What the Chek admin panel needs beyond the rest of the API: whether the
//! server still needs setting up, the server's folders to pick a library
//! from, and renaming and deleting libraries. The panel itself is the web
//! app's `/admin` screens; see [`crate::app`].

use std::path::{Path as FsPath, PathBuf};

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chek_catalog::LibraryDeletion;
use serde::{Deserialize, Serialize};

use crate::api::{ApiError, AppState, LibraryDto, blocking};
use crate::auth::CurrentUser;

/// A folder listing shows at most this many folders, so a folder of
/// thousands of series still answers quickly.
const MAX_FOLDERS: usize = 1000;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfoDto {
    version: &'static str,
    /// True until the first admin is created with the setup token.
    setup_required: bool,
}

/// Says which server this is and whether it still needs its first admin,
/// before anyone has signed in.
pub async fn server_info(State(state): State<AppState>) -> Json<ServerInfoDto> {
    let setup_required = state
        .setup_token
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some();
    Json(ServerInfoDto {
        version: env!("CARGO_PKG_VERSION"),
        setup_required,
    })
}

#[derive(Deserialize)]
pub struct FolderQuery {
    path: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderListDto {
    path: String,
    /// The folder above, or none at the top.
    parent: Option<String>,
    folders: Vec<FolderDto>,
    /// Whether some folders were left out past the first thousand.
    truncated: bool,
}

#[derive(Serialize)]
pub struct FolderDto {
    name: String,
    path: String,
}

/// Lists the folders inside one of the server's folders, so an admin can
/// pick a library's folder instead of typing its path. Hidden folders are
/// left out.
pub async fn list_folders(
    current: CurrentUser,
    Query(query): Query<FolderQuery>,
) -> Result<Json<FolderListDto>, ApiError> {
    current.require_admin()?;
    let path = PathBuf::from(query.path.as_deref().unwrap_or("/"));
    if !path.is_absolute() {
        return Err(ApiError::bad_request(
            "path must be an absolute path on the server",
        ));
    }
    tokio::task::spawn_blocking(move || folders(&path))
        .await
        .map_err(ApiError::internal)?
        .map(Json)
}

fn folders(path: &FsPath) -> Result<FolderListDto, ApiError> {
    let path = std::fs::canonicalize(path)
        .map_err(|e| ApiError::bad_request(format!("cannot open {}: {e}", path.display())))?;
    let entries = std::fs::read_dir(&path)
        .map_err(|e| ApiError::bad_request(format!("cannot open {}: {e}", path.display())))?;
    let mut folders = Vec::new();
    let mut truncated = false;
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        // Follows links, so a linked folder is listed like any other.
        if name.starts_with('.') || !entry.path().is_dir() {
            continue;
        }
        if folders.len() == MAX_FOLDERS {
            truncated = true;
            break;
        }
        folders.push(FolderDto {
            path: entry.path().to_string_lossy().into_owned(),
            name,
        });
    }
    folders.sort_by_cached_key(|f| f.name.to_lowercase());
    Ok(FolderListDto {
        parent: path.parent().map(|p| p.to_string_lossy().into_owned()),
        path: path.to_string_lossy().into_owned(),
        folders,
        truncated,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryChanges {
    name: String,
}

pub async fn update_library(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
    body: Result<Json<LibraryChanges>, JsonRejection>,
) -> Result<Json<LibraryDto>, ApiError> {
    current.require_admin()?;
    let Json(body) = body.map_err(|e| ApiError::bad_request(e.body_text()))?;
    let name = body.name.trim().to_owned();
    if name.is_empty() {
        return Err(ApiError::bad_request("name must not be empty"));
    }
    blocking(&state, move |c| c.rename_library(id, &name))
        .await?
        .map(|l| Json(l.into()))
        .ok_or_else(|| ApiError::library_not_found(id))
}

/// Removes a library from Chek. Its files, or its source, are untouched.
pub async fn delete_library(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    current.require_admin()?;
    match blocking(&state, move |c| c.delete_library(id)).await? {
        LibraryDeletion::Deleted => Ok(StatusCode::NO_CONTENT),
        LibraryDeletion::NotFound => Err(ApiError::library_not_found(id)),
        LibraryDeletion::Scanning => Err(ApiError {
            status: StatusCode::CONFLICT,
            code: "scan_in_progress",
            message: format!("library {id} is being scanned; delete it once the scan finishes"),
        }),
    }
}
