//! Serving publications from source libraries, such as Komga. Pages and files
//! are fetched from the source on each request and passed through, so the
//! source stays the only copy.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use chek_catalog::{PageRecord, RemoteContent};
use chek_komga::{Komga, KomgaConfig, KomgaError};
use chek_library::SourceConfig;

use serde_json::Value;
use tracing::warn;

use crate::api::{
    ApiError, AppState, blocking, content_disposition, file_media_type, image_response,
    matches_etag, not_modified, start_scan,
};

/// The publication's file, streamed from the source. Byte ranges are passed
/// on, so readers can open large files without downloading them whole.
pub(crate) async fn file(
    state: &AppState,
    remote: RemoteContent,
    headers: &HeaderMap,
) -> Result<Response, ApiError> {
    let komga = available(state, &remote)?;
    let etag = format!("\"{}\"", remote.hash);
    if matches_etag(headers, &etag) {
        return Ok(not_modified(&etag));
    }
    // A range of an older version would splice two files together.
    let range = headers.get(header::RANGE).filter(|_| {
        headers
            .get(header::IF_RANGE)
            .is_none_or(|v| v.as_bytes() == etag.as_bytes())
    });
    let upstream = komga
        .file(&remote.remote_id, range)
        .await
        .map_err(|e| source_error(state, &remote, e))?;

    let mut response = Response::builder().status(upstream.status().as_u16());
    for name in [
        header::CONTENT_LENGTH,
        header::CONTENT_RANGE,
        header::ACCEPT_RANGES,
    ] {
        if let Some(value) = upstream.headers().get(name.as_str())
            && let Ok(value) = HeaderValue::from_bytes(value.as_bytes())
        {
            response = response.header(name, value);
        }
    }
    let name = format!("{}.{}", file_stem(&remote.title), remote.format);
    let mut response = response
        .header(header::CONTENT_TYPE, file_media_type(&remote.format))
        .header(header::CACHE_CONTROL, "private, no-cache")
        .header(header::ETAG, etag)
        .body(Body::from_stream(upstream.bytes_stream()))
        .map_err(ApiError::internal)?;
    if let Ok(disposition) = HeaderValue::from_str(&content_disposition(&name)) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, disposition);
    }
    Ok(response)
}

/// One comic page, fetched from the source as stored in the book.
pub(crate) async fn page(
    state: &AppState,
    remote: RemoteContent,
    page: PageRecord,
    index: u32,
    headers: &HeaderMap,
) -> Result<Response, ApiError> {
    let komga = available(state, &remote)?;
    let etag = format!("\"{}-{index}\"", remote.hash);
    if matches_etag(headers, &etag) {
        return Ok(not_modified(&etag));
    }
    let bytes = komga
        .page(&remote.remote_id, index)
        .await
        .map_err(|e| source_error(state, &remote, e))?;
    Ok(image_response(
        bytes,
        chek_formats::page_media_type(&page.entry_name),
        &etag,
    ))
}

fn available(state: &AppState, remote: &RemoteContent) -> Result<Komga, ApiError> {
    if !remote.available {
        return Err(ApiError {
            status: StatusCode::NOT_FOUND,
            code: "unavailable",
            message: format!(
                "publication {} is no longer in its source",
                remote.publication_id
            ),
        });
    }
    // Callers send an extension's chapters to `source_libraries` first.
    let SourceConfig::Komga(config) =
        serde_json::from_str(&remote.source_config).map_err(ApiError::internal)?
    else {
        return Err(ApiError::internal("not a Komga library"));
    };
    Komga::new(state.scan.http.clone(), &config).map_err(ApiError::internal)
}

fn source_error(state: &AppState, remote: &RemoteContent, e: KomgaError) -> ApiError {
    match e {
        // The book left Komga since the last sync; a sync marks it.
        KomgaError::NotFound(_) => {
            start_scan(state, remote.library_id);
            ApiError {
                status: StatusCode::NOT_FOUND,
                code: "unavailable",
                message: format!(
                    "publication {} is no longer in its source; a sync has started",
                    remote.publication_id
                ),
            }
        }
        e => ApiError {
            status: StatusCode::BAD_GATEWAY,
            code: "source_unavailable",
            message: format!("couldn't read from the library's Komga server: {e}"),
        },
    }
}

/// A title made safe to use as a file name.
fn file_stem(title: &str) -> String {
    let stem: String = title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .take(150)
        .collect();
    let stem = stem.trim().trim_matches('.');
    if stem.is_empty() {
        "publication".to_owned()
    } else {
        stem.to_owned()
    }
}

/// Pending writes of reading progress to Komga, in save order.
pub type ProgressWrites = tokio::sync::mpsc::Sender<ProgressWrite>;

/// Saves waiting at once; more are dropped, since a later save replaces them.
const MAX_PENDING_WRITES: usize = 1024;

pub struct ProgressWrite {
    config: KomgaConfig,
    book_id: String,
    position: KomgaPosition,
}

enum KomgaPosition {
    Page { page: u32, completed: bool },
    Locator { locator: Value, modified: String },
}

/// Starts the task that writes progress to Komga. Writes go one at a time,
/// so Komga ends at the newest position, and saves for the same book that
/// queue up while Komga is slow are reduced to the last one.
pub(crate) fn progress_writer(http: chek_library::HttpClient) -> ProgressWrites {
    let (sender, mut receiver) = tokio::sync::mpsc::channel::<ProgressWrite>(MAX_PENDING_WRITES);
    tokio::spawn(async move {
        while let Some(first) = receiver.recv().await {
            let mut batch = vec![first];
            while let Ok(next) = receiver.try_recv() {
                batch.retain(|w| w.book_id != next.book_id);
                batch.push(next);
            }
            for write in batch {
                let result = match Komga::new(http.clone(), &write.config) {
                    Ok(komga) => match &write.position {
                        KomgaPosition::Page { page, completed } => {
                            komga
                                .set_read_progress(&write.book_id, *page, *completed)
                                .await
                        }
                        KomgaPosition::Locator { locator, modified } => {
                            komga
                                .set_progression(&write.book_id, locator, modified)
                                .await
                        }
                    },
                    Err(e) => Err(e),
                };
                if let Err(e) = result {
                    warn!(book = %write.book_id, error = %e, "couldn't write reading progress to Komga");
                }
            }
        }
    });
    sender
}

/// Queues a saved position for Komga when the publication comes from a Komga
/// library that writes this user's progress back.
pub(crate) async fn write_progress(
    state: &AppState,
    user_id: i64,
    publication_id: i64,
    locator: &Value,
    modified: Option<String>,
) {
    let remote = match blocking(state, move |c| c.publication_remote(publication_id)).await {
        Ok(Some(remote)) => remote,
        Ok(None) => return,
        Err(e) => {
            warn!(error = %e.message, "couldn't look up a publication's source");
            return;
        }
    };
    let Ok(SourceConfig::Komga(config)) = serde_json::from_str(&remote.source_config) else {
        return;
    };
    if config.progress_user_id != Some(user_id) {
        return;
    }
    let Some(position) = komga_position(&remote, locator, modified) else {
        return;
    };
    let write = ProgressWrite {
        config,
        book_id: remote.remote_id,
        position,
    };
    if state.progress_writes.try_send(write).is_err() {
        warn!("too many progress writes waiting for Komga; dropping one");
    }
}

/// Komga keeps EPUB positions as Readium locators and other books as a page.
fn komga_position(
    remote: &RemoteContent,
    locator: &Value,
    modified: Option<String>,
) -> Option<KomgaPosition> {
    let locations = locator.get("locations");
    let total = locations
        .and_then(|l| l.get("totalProgression"))
        .and_then(Value::as_f64);
    if remote.format == "epub" {
        return Some(KomgaPosition::Locator {
            locator: locator.clone(),
            modified: modified?,
        });
    }
    let pages = remote.page_count.and_then(|n| u32::try_from(n).ok());
    let page = locations
        .and_then(|l| l.get("position"))
        .and_then(Value::as_u64)
        .and_then(|p| u32::try_from(p).ok())
        .or_else(|| {
            // Without a position, the page is worked out from how far along
            // the reader is, when the page count is known.
            let pages = pages?;
            Some(((total? * f64::from(pages)).ceil() as u32).clamp(1, pages))
        })?;
    let page = pages.map_or(page, |pages| page.min(pages));
    let completed = pages.is_some_and(|pages| page >= pages) || total.is_some_and(|t| t >= 1.0);
    Some(KomgaPosition::Page { page, completed })
}
