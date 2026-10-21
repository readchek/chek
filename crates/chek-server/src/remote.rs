//! Serving publications from source libraries, such as Komga and Kavita.
//! Pages and files are fetched from the source on each request and passed
//! through, so the source stays the only copy.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use chek_catalog::{PageRecord, RemoteContent};
use chek_kavita::{Kavita, KavitaConfig};
use chek_komga::{Komga, KomgaConfig};
use chek_library::{SourceConfig, SourceError};

use serde_json::Value;
use tracing::{debug, warn};

use crate::api::{
    ApiError, AppState, blocking, content_disposition, file_media_type, image_response,
    matches_etag, not_modified, start_scan,
};

/// The server a publication's bytes come from.
enum Source {
    Komga(Komga),
    Kavita(Kavita),
}

impl Source {
    fn open(state: &AppState, remote: &RemoteContent) -> Result<Self, ApiError> {
        // Callers send an extension's chapters to `source_libraries` first.
        match serde_json::from_str(&remote.source_config).map_err(ApiError::internal)? {
            SourceConfig::Komga(config) => Komga::new(state.scan.http.clone(), &config)
                .map(Source::Komga)
                .map_err(ApiError::internal),
            SourceConfig::Kavita(config) => Kavita::new(state.scan.http.clone(), &config)
                .map(Source::Kavita)
                .map_err(ApiError::internal),
            SourceConfig::Extension(_) => Err(ApiError::internal("not a Komga or Kavita library")),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Source::Komga(_) => "Komga",
            Source::Kavita(_) => "Kavita",
        }
    }

    async fn file(
        &self,
        remote_id: &str,
        range: Option<&HeaderValue>,
    ) -> Result<reqwest::Response, SourceError> {
        match self {
            Source::Komga(komga) => Ok(komga.file(remote_id, range).await?),
            Source::Kavita(kavita) => Ok(kavita.file(chapter_id(remote_id)?, range).await?),
        }
    }

    /// A page's bytes, and its media type when the source names one.
    async fn page(
        &self,
        remote_id: &str,
        index: u32,
    ) -> Result<(Vec<u8>, Option<String>), SourceError> {
        match self {
            Source::Komga(komga) => Ok((komga.page(remote_id, index).await?, None)),
            Source::Kavita(kavita) => {
                let page = kavita.page(chapter_id(remote_id)?, index).await?;
                Ok((page.bytes, page.media_type))
            }
        }
    }
}

/// Kavita's ids are numbers; a remote id that isn't one was never Kavita's.
fn chapter_id(remote_id: &str) -> Result<i64, SourceError> {
    remote_id
        .parse()
        .map_err(|_| chek_kavita::KavitaError::NotFound(format!("chapter {remote_id}")).into())
}

/// The publication's file, streamed from the source. Byte ranges are passed
/// on, so readers can open large files without downloading them whole.
pub(crate) async fn file(
    state: &AppState,
    remote: RemoteContent,
    headers: &HeaderMap,
) -> Result<Response, ApiError> {
    let source = available(state, &remote)?;
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
    let upstream = source
        .file(&remote.remote_id, range)
        .await
        .map_err(|e| source_error(state, &remote, source.name(), e))?;

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
    let source = available(state, &remote)?;
    let etag = format!("\"{}-{index}\"", remote.hash);
    if matches_etag(headers, &etag) {
        return Ok(not_modified(&etag));
    }
    let (bytes, media_type) = source
        .page(&remote.remote_id, index)
        .await
        .map_err(|e| source_error(state, &remote, source.name(), e))?;
    let media_type = media_type
        .filter(|t| t.starts_with("image/"))
        .unwrap_or_else(|| chek_formats::page_media_type(&page.entry_name).to_owned());
    Ok(image_response(bytes, &media_type, &etag))
}

fn available(state: &AppState, remote: &RemoteContent) -> Result<Source, ApiError> {
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
    Source::open(state, remote)
}

fn source_error(
    state: &AppState,
    remote: &RemoteContent,
    source: &str,
    e: SourceError,
) -> ApiError {
    if e.is_not_found() {
        // The item left the source since the last sync; a sync marks it.
        start_scan(state, remote.library_id);
        return ApiError {
            status: StatusCode::NOT_FOUND,
            code: "unavailable",
            message: format!(
                "publication {} is no longer in its source; a sync has started",
                remote.publication_id
            ),
        };
    }
    ApiError {
        status: StatusCode::BAD_GATEWAY,
        code: "source_unavailable",
        message: format!("couldn't read from the library's {source} server: {e}"),
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

/// Pending writes of reading progress to sources, in save order.
pub type ProgressWrites = tokio::sync::mpsc::Sender<ProgressWrite>;

/// Saves waiting at once; more are dropped, since a later save replaces them.
const MAX_PENDING_WRITES: usize = 1024;

pub struct ProgressWrite {
    library_id: i64,
    remote_id: String,
    target: WriteTarget,
}

enum WriteTarget {
    Komga {
        config: KomgaConfig,
        position: KomgaPosition,
    },
    Kavita {
        config: KavitaConfig,
        position: PagePosition,
    },
}

enum KomgaPosition {
    Page(PagePosition),
    Locator { locator: Value, modified: String },
}

/// A place in a comic or PDF: the page the reader is on, counted from one,
/// and whether that finishes the book.
struct PagePosition {
    page: u32,
    completed: bool,
}

/// Starts the task that writes progress to sources. Writes go one at a
/// time, so a source ends at the newest position, and saves for the same
/// item that queue up while a source is slow are reduced to the last one.
pub(crate) fn progress_writer(http: chek_library::HttpClient) -> ProgressWrites {
    let (sender, mut receiver) = tokio::sync::mpsc::channel::<ProgressWrite>(MAX_PENDING_WRITES);
    tokio::spawn(async move {
        while let Some(first) = receiver.recv().await {
            let mut batch = vec![first];
            while let Ok(next) = receiver.try_recv() {
                batch
                    .retain(|w| (w.library_id, &w.remote_id) != (next.library_id, &next.remote_id));
                batch.push(next);
            }
            for write in batch {
                if let Err(e) = send_write(&http, &write).await {
                    warn!(item = %write.remote_id, error = %e, "couldn't write reading progress to the source");
                }
            }
        }
    });
    sender
}

async fn send_write(
    http: &chek_library::HttpClient,
    write: &ProgressWrite,
) -> Result<(), SourceError> {
    match &write.target {
        WriteTarget::Komga { config, position } => {
            let komga = Komga::new(http.clone(), config)?;
            match position {
                KomgaPosition::Page(PagePosition { page, completed }) => {
                    komga
                        .set_read_progress(&write.remote_id, *page, *completed)
                        .await?;
                }
                KomgaPosition::Locator { locator, modified } => {
                    komga
                        .set_progression(&write.remote_id, locator, modified)
                        .await?;
                }
            }
        }
        WriteTarget::Kavita { config, position } => {
            let kavita = Kavita::new(http.clone(), config)?;
            let chapter = chapter_id(&write.remote_id)?;
            let info = kavita.chapter_info(chapter).await?;
            // Kavita counts the page it is on from zero, and takes the page
            // count to mean read, as its own reader sends on the last page.
            let page_num = if position.completed {
                info.pages
            } else {
                position
                    .page
                    .saturating_sub(1)
                    .min(info.pages.saturating_sub(1))
            };
            kavita.set_progress(chapter, &info, page_num).await?;
        }
    }
    Ok(())
}

/// Queues a saved position for the source when the publication comes from
/// a Komga or Kavita library that writes this user's progress back.
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
    let target = match serde_json::from_str(&remote.source_config) {
        Ok(SourceConfig::Komga(config)) if config.progress_user_id == Some(user_id) => {
            let Some(position) = komga_position(&remote, locator, modified) else {
                return;
            };
            WriteTarget::Komga { config, position }
        }
        Ok(SourceConfig::Kavita(config)) if config.progress_user_id == Some(user_id) => {
            let Some(position) = page_position(&remote, locator) else {
                return;
            };
            // Kavita keeps a book's place as a point in its own layout, which
            // a locator can't name; only finishing one is written.
            if remote.format == "epub" && !position.completed {
                debug!(publication_id, "a place in an EPUB isn't written to Kavita");
                return;
            }
            WriteTarget::Kavita { config, position }
        }
        _ => return,
    };
    let write = ProgressWrite {
        library_id: remote.library_id,
        remote_id: remote.remote_id,
        target,
    };
    if state.progress_writes.try_send(write).is_err() {
        warn!("too many progress writes waiting for sources; dropping one");
    }
}

/// Komga keeps EPUB positions as Readium locators and other books as a page.
fn komga_position(
    remote: &RemoteContent,
    locator: &Value,
    modified: Option<String>,
) -> Option<KomgaPosition> {
    if remote.format == "epub" {
        return Some(KomgaPosition::Locator {
            locator: locator.clone(),
            modified: modified?,
        });
    }
    page_position(remote, locator).map(KomgaPosition::Page)
}

/// The page a locator names, counted from one, or the page that far along
/// the book when the locator only says how far.
fn page_position(remote: &RemoteContent, locator: &Value) -> Option<PagePosition> {
    let locations = locator.get("locations");
    let total = locations
        .and_then(|l| l.get("totalProgression"))
        .and_then(Value::as_f64);
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
    Some(PagePosition { page, completed })
}
