//! Serving publications from source libraries, such as Komga. Pages and files
//! are fetched from the source on each request and passed through, so the
//! source stays the only copy.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use chek_catalog::{PageRecord, RemoteContent};
use chek_komga::{Komga, KomgaError};
use chek_library::SourceConfig;

use crate::api::{
    ApiError, AppState, content_disposition, file_media_type, image_response, matches_etag,
    not_modified, start_scan,
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
