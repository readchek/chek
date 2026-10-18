//! Syncing a Komga source library: Komga's series and books are recorded in
//! the catalog, and pages and files stay on Komga, served through it.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use chek_catalog::{Catalog, ContributorRecord, PageRecord, PublicationDetails, RemoteItem};
use chek_formats::{MAX_FIELD_CHARS, MAX_SUMMARY_CHARS, Role};
use chek_komga::{Book, Komga, KomgaError, Series};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{debug, warn};

use crate::{ScanError, ScanOptions, ScanReport, save_cover};

/// The most contributors kept per book, as scans keep from a file.
const MAX_CONTRIBUTORS: usize = 50;

pub(crate) async fn sync(
    catalog: Arc<Catalog>,
    library_id: i64,
    komga: Komga,
    options: ScanOptions,
) -> Result<ScanReport, ScanError> {
    let series: HashMap<String, Series> = komga
        .series()
        .await?
        .into_iter()
        .map(|s| (s.id.clone(), s))
        .collect();
    let books = komga.books().await?;
    let recorded = {
        let catalog = catalog.clone();
        tokio::task::spawn_blocking(move || catalog.remote_versions(library_id)).await??
    };

    let mut report = ScanReport::default();
    let mut seen = HashSet::with_capacity(books.len());
    let permits = Arc::new(Semaphore::new(options.workers.max(1)));
    let covers_dir = Arc::new(options.covers_dir);
    let mut tasks = JoinSet::new();
    for book in books {
        let Some(format) = format_of(&book.media.media_type) else {
            debug!(book = %book.id, media_type = %book.media.media_type, "skipping Komga book in a format Chek doesn't serve");
            continue;
        };
        report.files_seen += 1;
        seen.insert(book.id.clone());
        let series = series.get(&book.series_id).cloned();
        let version = version_of(&book, series.as_ref());
        if recorded.get(&book.id) == Some(&version) {
            report.files_unchanged += 1;
            continue;
        }
        let permit = permits
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore is never closed");
        let catalog = catalog.clone();
        let komga = komga.clone();
        let covers_dir = covers_dir.clone();
        tasks.spawn(async move {
            let _permit = permit;
            let id = book.id.clone();
            let outcome = record_book(
                &catalog,
                library_id,
                &komga,
                &covers_dir,
                book,
                series,
                format,
                version,
            )
            .await;
            (id, outcome)
        });
        while let Some(done) = tasks.try_join_next() {
            tally(&mut report, done?)?;
        }
    }
    while let Some(done) = tasks.join_next().await {
        tally(&mut report, done?)?;
    }

    tokio::task::spawn_blocking(move || catalog.set_remote_availability(library_id, &seen))
        .await??;
    Ok(report)
}

#[derive(Debug, thiserror::Error)]
enum BookError {
    #[error(transparent)]
    Komga(#[from] KomgaError),
    #[error(transparent)]
    Catalog(#[from] chek_catalog::CatalogError),
    #[error("task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
}

fn tally(
    report: &mut ScanReport,
    (id, outcome): (String, Result<(), BookError>),
) -> Result<(), ScanError> {
    match outcome {
        Ok(()) => report.files_recorded += 1,
        // A catalog failure would fail every later book too.
        Err(BookError::Catalog(e)) => return Err(e.into()),
        Err(BookError::Task(e)) => return Err(e.into()),
        Err(e) => {
            warn!(book = %id, error = %e, "skipping Komga book");
            report.files_failed += 1;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn record_book(
    catalog: &Arc<Catalog>,
    library_id: i64,
    komga: &Komga,
    covers_dir: &Path,
    book: Book,
    series: Option<Series>,
    format: &'static str,
    version: String,
) -> Result<(), BookError> {
    // Only comics are read page by page; books and PDFs are served whole.
    let pages = if matches!(format, "cbz" | "cbr" | "cb7" | "cbt") {
        komga
            .pages(&book.id)
            .await?
            .into_iter()
            .map(|page| PageRecord {
                entry_name: page.file_name,
                data_offset: 0,
                compressed_size: 0,
                size: page.size_bytes.unwrap_or(0),
                compression: "remote",
            })
            .collect()
    } else {
        Vec::new()
    };

    let hash = blake3::hash(format!("komga\n{library_id}\n{}\n{version}", book.id).as_bytes())
        .to_hex()[..32]
        .to_owned();
    let cover = match komga.thumbnail(&book.id).await {
        Ok(image) => {
            let covers_dir = covers_dir.to_owned();
            let name = hash.clone();
            tokio::task::spawn_blocking(move || match chek_formats::cover_from_image(&image) {
                Ok(jpeg) => save_cover(&covers_dir, &name, &jpeg),
                Err(e) => {
                    warn!(error = %e, "could not make cover from Komga's thumbnail");
                    None
                }
            })
            .await?
        }
        Err(e) => {
            warn!(book = %book.id, error = %e, "could not fetch Komga's thumbnail");
            None
        }
    };

    let item = item_for(book, series, format, version, hash, pages, cover);
    let catalog = catalog.clone();
    tokio::task::spawn_blocking(move || catalog.record_remote(library_id, &item)).await??;
    Ok(())
}

fn item_for(
    book: Book,
    series: Option<Series>,
    format: &'static str,
    version: String,
    hash: String,
    pages: Vec<PageRecord>,
    cover: Option<String>,
) -> RemoteItem {
    let metadata = book.metadata;
    let series_title = series
        .as_ref()
        .and_then(|s| text(&s.metadata.title, MAX_FIELD_CHARS))
        .unwrap_or_else(|| book.series_title.clone());
    let series_sort = series
        .as_ref()
        .and_then(|s| text(&s.metadata.title_sort, MAX_FIELD_CHARS))
        .unwrap_or_else(|| series_title.clone());
    let title = text(&metadata.title, MAX_FIELD_CHARS).unwrap_or_else(|| book.name.clone());
    let number_label = text(&metadata.number, MAX_FIELD_CHARS);
    let sort_number = metadata.number_sort.filter(|n| n.is_finite()).or_else(|| {
        number_label
            .as_deref()
            .and_then(chek_formats::leading_number)
    });
    let series_metadata = series.map(|s| s.metadata);
    let details = PublicationDetails {
        summary: text(&metadata.summary, MAX_SUMMARY_CHARS),
        publisher: series_metadata
            .as_ref()
            .and_then(|m| text(&m.publisher, MAX_FIELD_CHARS)),
        language: series_metadata
            .as_ref()
            .and_then(|m| text(&m.language, MAX_FIELD_CHARS)),
        published: metadata.release_date.filter(|d| is_date(d)),
        reading_direction: series_metadata.and_then(|m| match m.reading_direction.as_str() {
            "LEFT_TO_RIGHT" => Some("ltr".to_owned()),
            "RIGHT_TO_LEFT" => Some("rtl".to_owned()),
            _ => None,
        }),
        contributors: metadata
            .authors
            .iter()
            .filter_map(|a| {
                Some(ContributorRecord {
                    name: text(&a.name, MAX_FIELD_CHARS)?,
                    role: role_of(&a.role).as_str().to_owned(),
                })
            })
            .take(MAX_CONTRIBUTORS)
            .collect(),
    };
    RemoteItem {
        remote_id: book.id,
        version,
        hash,
        series_remote_id: book.series_id,
        series_sort_title: chek_formats::natural_key(&series_sort),
        series_title,
        sort_title: chek_formats::natural_key(&title),
        title,
        format,
        size: book.size_bytes,
        pages,
        cover,
        number_label,
        sort_number,
        details,
    }
}

/// Changes when the book, its file or its series' metadata changes in
/// Komga, which is when Chek reads the book again.
fn version_of(book: &Book, series: Option<&Series>) -> String {
    format!(
        "{}|{}|{}",
        book.last_modified,
        book.size_bytes,
        series.map_or("", |s| s.last_modified.as_str())
    )
}

/// The Chek format for a media type Komga detected.
fn format_of(media_type: &str) -> Option<&'static str> {
    let base = media_type.split(';').next().unwrap_or("").trim();
    Some(match base {
        "application/zip" | "application/vnd.comicbook+zip" => "cbz",
        "application/x-rar-compressed"
        | "application/vnd.rar"
        | "application/vnd.comicbook-rar" => "cbr",
        "application/x-7z-compressed" => "cb7",
        "application/x-tar" => "cbt",
        "application/epub+zip" => "epub",
        "application/pdf" => "pdf",
        _ => return None,
    })
}

/// Komga's author roles, which follow ComicInfo's credits.
fn role_of(role: &str) -> Role {
    match role.to_ascii_lowercase().as_str() {
        "writer" | "author" => Role::Author,
        "penciller" | "penciler" => Role::Penciler,
        "inker" => Role::Inker,
        "colorist" => Role::Colorist,
        "letterer" => Role::Letterer,
        "cover" => Role::CoverArtist,
        "editor" => Role::Editor,
        "translator" => Role::Translator,
        "illustrator" => Role::Illustrator,
        _ => Role::Contributor,
    }
}

/// Trims a field and cuts it to `max` characters; blank is `None`.
fn text(value: &str, max: usize) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.chars().take(max).collect())
}

fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
}
