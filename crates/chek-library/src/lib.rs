//! Folder scanning for Chek libraries.
//!
//! A scan walks a library's folder, skips files whose size and modification
//! time haven't changed, and records the rest in the catalog. Scans run on a
//! few blocking threads, always fewer than the machine's cores, so serving
//! pages stays fast while a scan runs.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use chek_catalog::{Catalog, CatalogError, FileStamp, PageRecord, ScannedFile};
use chek_formats::Format;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{debug, info, warn};

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("library {0} does not exist")]
    UnknownLibrary(i64),
    #[error("library {0} is not a folder library")]
    NotAFolder(i64),
    #[error("library {0} is already being scanned")]
    AlreadyScanning(i64),
    #[error("cannot read the library folder {path}: {source}")]
    Root { path: PathBuf, source: io::Error },
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error("scan task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
}

/// What a finished scan did.
#[derive(Debug, Default, Clone, Copy)]
pub struct ScanReport {
    pub files_seen: usize,
    pub files_recorded: usize,
    pub files_unchanged: usize,
    pub files_failed: usize,
}

/// How many files a scan reads at once: one fewer than the cores, at least one.
pub fn default_scan_workers() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get().saturating_sub(1).max(1))
}

/// Scans a folder library and records what it finds. Files that fail to read
/// are logged and skipped; the scan only fails if the folder itself can't be
/// read or the catalog can't be written.
pub async fn scan(
    catalog: Arc<Catalog>,
    library_id: i64,
    workers: usize,
) -> Result<ScanReport, ScanError> {
    let library = {
        let catalog = catalog.clone();
        tokio::task::spawn_blocking(move || catalog.library(library_id)).await??
    };
    let library = library.ok_or(ScanError::UnknownLibrary(library_id))?;
    let root = PathBuf::from(library.root_path.ok_or(ScanError::NotAFolder(library_id))?);

    let began = {
        let catalog = catalog.clone();
        tokio::task::spawn_blocking(move || catalog.begin_scan(library_id)).await??
    };
    if !began {
        return Err(ScanError::AlreadyScanning(library_id));
    }

    let result = run(catalog.clone(), library_id, root, workers.max(1)).await;
    let error = result.as_ref().err().map(ToString::to_string);
    let finished = {
        let catalog = catalog.clone();
        tokio::task::spawn_blocking(move || catalog.finish_scan(library_id, error.as_deref()))
            .await?
    };
    let report = result?;
    finished?;
    info!(library_id, ?report, "scan finished");
    Ok(report)
}

async fn run(
    catalog: Arc<Catalog>,
    library_id: i64,
    root: PathBuf,
    workers: usize,
) -> Result<ScanReport, ScanError> {
    let files = {
        let root = root.clone();
        tokio::task::spawn_blocking(move || list_candidates(&root)).await??
    };

    let mut report = ScanReport {
        files_seen: files.len(),
        ..ScanReport::default()
    };
    let mut seen = HashSet::with_capacity(files.len());
    let permits = Arc::new(Semaphore::new(workers));
    let mut tasks = JoinSet::new();

    for (path, relative_path, format) in files {
        seen.insert(relative_path.clone());
        let permit = permits
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore is never closed");
        let catalog = catalog.clone();
        let root = root.clone();
        tasks.spawn_blocking(move || {
            let _permit = permit;
            let outcome = scan_file(&catalog, library_id, &root, &path, relative_path, format);
            (path, outcome)
        });
        // Drain finished tasks as we go so results don't pile up.
        while let Some(done) = tasks.try_join_next() {
            tally(&mut report, done?)?;
        }
    }
    while let Some(done) = tasks.join_next().await {
        tally(&mut report, done?)?;
    }

    tokio::task::spawn_blocking(move || catalog.set_availability(library_id, &seen)).await??;
    Ok(report)
}

enum FileOutcome {
    Recorded,
    Unchanged,
}

#[derive(Debug, thiserror::Error)]
enum FileError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Format(#[from] chek_formats::FormatError),
    #[error(transparent)]
    Catalog(#[from] CatalogError),
}

fn tally(
    report: &mut ScanReport,
    (path, outcome): (PathBuf, Result<FileOutcome, FileError>),
) -> Result<(), ScanError> {
    match outcome {
        Ok(FileOutcome::Recorded) => report.files_recorded += 1,
        Ok(FileOutcome::Unchanged) => report.files_unchanged += 1,
        // A catalog failure would fail every later file too, so stop the scan.
        Err(FileError::Catalog(e)) => return Err(e.into()),
        Err(e) => {
            warn!(path = %path.display(), error = %e, "skipping file");
            report.files_failed += 1;
        }
    }
    Ok(())
}

/// Walks the folder and returns each supported file with its path relative to
/// the root. Hidden files and folders are skipped, and symlinks are not
/// followed so a library can't reach outside its folder.
fn list_candidates(root: &Path) -> Result<Vec<(PathBuf, String, Format)>, ScanError> {
    fs::read_dir(root).map_err(|source| ScanError::Root {
        path: root.to_owned(),
        source,
    })?;

    let mut files = Vec::new();
    let walker = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'));
    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                warn!(error = %e, "skipping unreadable path");
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.into_path();
        let Some(relative_path) = relative(root, &path) else {
            warn!(path = %path.display(), "skipping path that isn't valid UTF-8");
            continue;
        };
        match chek_formats::detect(&path) {
            Ok(Some(format)) => files.push((path, relative_path, format)),
            Ok(None) => {}
            Err(e) => warn!(path = %path.display(), error = %e, "skipping file"),
        }
    }
    Ok(files)
}

fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let parts: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
    Some(parts?.join("/"))
}

fn scan_file(
    catalog: &Catalog,
    library_id: i64,
    root: &Path,
    path: &Path,
    relative_path: String,
    format: Format,
) -> Result<FileOutcome, FileError> {
    let metadata = fs::metadata(path)?;
    let stamp = FileStamp {
        size: metadata.len(),
        modified_at: metadata
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64),
    };
    if catalog.file_stamp(library_id, &relative_path)? == Some(stamp) {
        return Ok(FileOutcome::Unchanged);
    }

    debug!(path = %relative_path, "reading file");
    let inspection = chek_formats::inspect(path, format)?;
    let hash = hash_file(path)?;
    let (series_key, series_title) = series_for(root, path, &relative_path);
    let title = path.file_stem().map_or_else(
        || relative_path.clone(),
        |s| s.to_string_lossy().into_owned(),
    );

    catalog.record_file(
        library_id,
        &ScannedFile {
            relative_path,
            series_key,
            series_title,
            title,
            format: format.as_str(),
            stamp,
            hash,
            pages: inspection
                .pages
                .into_iter()
                .map(|p| PageRecord {
                    entry_name: p.name,
                    data_offset: p.data_offset,
                    compressed_size: p.compressed_size,
                    size: p.size,
                    compression: p.compression.as_str(),
                })
                .collect(),
        },
    )?;
    Ok(FileOutcome::Recorded)
}

/// A file's series is the folder it sits in. A file at the library root is a
/// series of its own, keyed by its path, since it has no folder to group by.
fn series_for(root: &Path, path: &Path, relative_path: &str) -> (String, String) {
    match relative_path.rsplit_once('/') {
        Some((folder, _)) => {
            let title = path
                .parent()
                .filter(|p| *p != root)
                .and_then(|p| p.file_name())
                .map_or_else(|| folder.to_owned(), |n| n.to_string_lossy().into_owned());
            (folder.to_owned(), title)
        }
        None => {
            let title = path.file_stem().map_or_else(
                || relative_path.to_owned(),
                |s| s.to_string_lossy().into_owned(),
            );
            (relative_path.to_owned(), title)
        }
    }
}

fn hash_file(path: &Path) -> io::Result<String> {
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(File::open(path)?)?;
    Ok(hasher.finalize().to_hex().to_string())
}
