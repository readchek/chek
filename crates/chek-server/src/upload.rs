//! Uploads into folder libraries. The file is streamed to a hidden temporary
//! name next to its destination, checked, moved into place and recorded, so
//! a scan never sees a partial file and the upload shows up like any file
//! put in the folder by hand.

use std::path::{Component, Path as FsPath, PathBuf};
use std::time::Duration;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chek_formats::Format;
use http_body_util::BodyExt;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

use crate::api::{ApiError, AppState, PublicationDto, blocking};
use crate::auth::CurrentUser;

/// Longer than any real path in a library, short enough to stay well under
/// file system limits.
const MAX_PATH_BYTES: usize = 1024;

/// How long the request waits for the file to be recorded. A scan of the
/// same library holds recording until it finishes; past this the upload
/// answers 202 and is recorded in the background.
const RECORD_WAIT: Duration = Duration::from_secs(20);

/// `PUT /api/v1/libraries/{id}/files/{*path}`: stores the request body as
/// `path` in the library's folder. Admin only.
pub async fn upload(
    State(state): State<AppState>,
    current: CurrentUser,
    Path((id, path)): Path<(i64, String)>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    current.require_admin()?;
    let (relative_path, format) = checked_path(&path)?;
    // Refused before reading, so a client sending `Expect: 100-continue`
    // gets the answer instead of a dropped connection.
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|len| len > state.max_upload_bytes) {
        return Err(too_large(state.max_upload_bytes));
    }
    let library = blocking(&state, move |c| c.library(id))
        .await?
        .ok_or_else(|| ApiError::library_not_found(id))?;
    let Some(root) = library.root_path else {
        return Err(ApiError::bad_request(
            "uploads go into folder libraries; this library is a source",
        ));
    };
    let root = PathBuf::from(root);
    let destination = root.join(&relative_path);
    let parent = destination
        .parent()
        .expect("a checked path has a file name")
        .to_owned();
    prepare_folder(&root, &parent).await?;
    if tokio::fs::try_exists(&destination)
        .await
        .map_err(ApiError::internal)?
    {
        return Err(file_exists(&relative_path));
    }

    let temp = parent.join(temp_name(format));
    let written = receive(body, &temp, state.max_upload_bytes).await;
    if let Err(e) = written {
        remove(&temp).await;
        return Err(e);
    }
    let check = {
        let temp = temp.clone();
        tokio::task::spawn_blocking(move || chek_formats::detect(&temp))
            .await
            .map_err(ApiError::internal)?
    };
    if !matches!(check, Ok(Some(found)) if found == format) {
        remove(&temp).await;
        return Err(ApiError::bad_request(format!(
            "the body is not a {} file",
            format.as_str().to_uppercase()
        )));
    }
    // Checked again right before the move, which replaces an existing file,
    // so two uploads to one path can't both land.
    if tokio::fs::try_exists(&destination)
        .await
        .map_err(ApiError::internal)?
    {
        remove(&temp).await;
        return Err(file_exists(&relative_path));
    }
    if let Err(e) = tokio::fs::rename(&temp, &destination).await {
        remove(&temp).await;
        return Err(ApiError::internal(e));
    }
    info!(library_id = id, path = %relative_path, "file uploaded");

    let recording = tokio::spawn(chek_library::add_file(
        state.catalog.clone(),
        id,
        relative_path.clone(),
        state.scan.clone(),
    ));
    let publication_id = match tokio::time::timeout(RECORD_WAIT, recording).await {
        Err(_) => {
            return Ok((
                StatusCode::ACCEPTED,
                Json(serde_json::json!({ "path": relative_path, "status": "queued" })),
            )
                .into_response());
        }
        Ok(joined) => match joined.map_err(ApiError::internal)? {
            Ok(id) => id,
            // The bytes matched the format, so this is a damaged file; it
            // stays in the folder like any other unreadable file.
            Err(e) => {
                warn!(library_id = id, path = %relative_path, error = %e, "uploaded file unreadable");
                return Err(ApiError {
                    status: StatusCode::UNPROCESSABLE_ENTITY,
                    code: "unreadable_file",
                    message: format!("the file was stored but can't be read: {e}"),
                });
            }
        },
    };
    let publication = blocking(&state, move |c| c.publication(publication_id))
        .await?
        .ok_or_else(|| ApiError::internal("uploaded publication vanished"))?;
    Ok((StatusCode::CREATED, Json(PublicationDto::from(publication))).into_response())
}

/// Checks a path sent by the client and returns it with `/` separators and
/// the format its extension names. Every part must be a plain name: no
/// `.`, `..`, hidden names or empty parts, so the file lands inside the
/// library and a scan will see it.
fn checked_path(path: &str) -> Result<(String, Format), ApiError> {
    let invalid = |why: &str| ApiError::bad_request(format!("path {path:?} {why}"));
    if path.is_empty() || path.len() > MAX_PATH_BYTES {
        return Err(invalid("must be 1 to 1024 bytes"));
    }
    if path.contains(['\\', '\0']) {
        return Err(invalid("must use / between folders"));
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || p.starts_with('.') || p.trim() != *p)
    {
        return Err(invalid(
            "has an empty, hidden or space-padded part; use plain folder and file names",
        ));
    }
    let relative = FsPath::new(path);
    if !relative
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(invalid("must stay inside the library folder"));
    }
    let format = relative
        .extension()
        .and_then(|e| e.to_str())
        .and_then(Format::from_extension)
        .ok_or_else(|| invalid("must end in .epub, .pdf, .cbz or .cbt"))?;
    Ok((parts.join("/"), format))
}

/// Creates the destination's folders and checks they resolve inside the
/// library's folder, so an existing symlink can't send the file elsewhere.
async fn prepare_folder(root: &FsPath, parent: &FsPath) -> Result<(), ApiError> {
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(write_error)?;
    let root = tokio::fs::canonicalize(root)
        .await
        .map_err(ApiError::internal)?;
    let parent = tokio::fs::canonicalize(parent)
        .await
        .map_err(ApiError::internal)?;
    if parent.starts_with(&root) {
        Ok(())
    } else {
        Err(ApiError::bad_request(
            "the path leads outside the library folder through a link",
        ))
    }
}

/// Streams the body to `temp`, stopping at `limit` bytes.
async fn receive(body: Body, temp: &FsPath, limit: u64) -> Result<(), ApiError> {
    let mut file = tokio::fs::File::create_new(temp)
        .await
        .map_err(write_error)?;
    let mut body = body;
    let mut received: u64 = 0;
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|e| ApiError::bad_request(format!("upload interrupted: {e}")))?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        received += data.len() as u64;
        if received > limit {
            return Err(too_large(limit));
        }
        file.write_all(&data).await.map_err(ApiError::internal)?;
    }
    if received == 0 {
        return Err(ApiError::bad_request(
            "the body is empty; send the file's bytes",
        ));
    }
    file.sync_all().await.map_err(ApiError::internal)
}

/// Hidden, so a scan running meanwhile skips it, and random, so parallel
/// uploads don't collide.
fn temp_name(format: Format) -> String {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("the operating system provides randomness");
    let suffix: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(".chek-upload-{suffix}.{}", format.as_str())
}

async fn remove(path: &FsPath) {
    if let Err(e) = tokio::fs::remove_file(path).await {
        warn!(path = %path.display(), error = %e, "cannot remove partial upload");
    }
}

/// Libraries are often mounted read-only, which deserves a clearer answer
/// than a server error.
fn write_error(e: std::io::Error) -> ApiError {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem => ApiError {
            status: StatusCode::CONFLICT,
            code: "library_read_only",
            message:
                "the server can't write to this library's folder; mount it read-write to upload"
                    .to_owned(),
        },
        _ => ApiError::internal(e),
    }
}

fn too_large(limit: u64) -> ApiError {
    ApiError {
        status: StatusCode::PAYLOAD_TOO_LARGE,
        code: "too_large",
        message: format!(
            "uploads are limited to {limit} bytes; raise CHEK_MAX_UPLOAD_BYTES to allow more"
        ),
    }
}

fn file_exists(path: &str) -> ApiError {
    ApiError {
        status: StatusCode::CONFLICT,
        code: "file_exists",
        message: format!("{path} already exists; delete it or upload under another name"),
    }
}
