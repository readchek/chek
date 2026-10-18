//! Folder scanning for Chek libraries.
//!
//! A scan walks a library's folder, skips files whose size and modification
//! time haven't changed, and records the rest in the catalog. Scans run on a
//! few blocking threads, always fewer than the machine's cores, so serving
//! pages stays fast while a scan runs.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::UNIX_EPOCH;

use chek_catalog::{
    Catalog, CatalogError, ContributorRecord, FileStamp, PageRecord, PublicationDetails,
    ScannedFile,
};
use chek_formats::Format;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{debug, info, warn};

mod komga;

pub use chek_komga::{Client as HttpClient, KomgaAuth, KomgaConfig, KomgaError, http_client};

/// How a source library reaches its source, stored with the library as JSON.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SourceConfig {
    Komga(KomgaConfig),
    Extension(ExtensionSource),
}

/// A library of series followed through an installed extension. The server
/// runs extensions, so it syncs these libraries itself rather than through
/// [`scan`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionSource {
    pub extension_id: String,
    /// The admin whose install the library runs on.
    pub user_id: i64,
}

impl SourceConfig {
    /// Checks that the source answers with these settings. Extensions are
    /// checked by the server, against the install.
    pub async fn check(&self, http: &HttpClient) -> Result<(), KomgaError> {
        match self {
            SourceConfig::Komga(config) => {
                chek_komga::Komga::new(http.clone(), config)?.check().await
            }
            SourceConfig::Extension(_) => Ok(()),
        }
    }
}

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
    #[error("cannot create the covers folder {path}: {source}")]
    CoversDir { path: PathBuf, source: io::Error },
    #[error("library {0}'s source settings can't be read")]
    SourceConfig(i64),
    #[error("library {0} runs on an extension, which the server syncs")]
    ExtensionSource(i64),
    #[error("{0}")]
    Source(#[from] KomgaError),
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

#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// How many files to read at once.
    pub workers: usize,
    /// Where cover thumbnails are written, named by the file's content hash.
    pub covers_dir: PathBuf,
    /// Reaches source libraries' servers.
    pub http: HttpClient,
}

/// The size and modification time of a file on disk, comparable with what a
/// scan recorded.
pub fn file_stamp(path: &Path) -> io::Result<FileStamp> {
    let metadata = fs::metadata(path)?;
    Ok(FileStamp {
        size: metadata.len(),
        modified_at: metadata
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64),
    })
}

/// Scans a folder library, or syncs a source library, and records what it
/// finds. Files or items that fail to read are logged and skipped; the scan
/// only fails if the folder or source itself can't be read or the catalog
/// can't be written.
pub async fn scan(
    catalog: Arc<Catalog>,
    library_id: i64,
    options: ScanOptions,
) -> Result<ScanReport, ScanError> {
    let library = {
        let catalog = catalog.clone();
        tokio::task::spawn_blocking(move || catalog.library(library_id)).await??
    };
    let library = library.ok_or(ScanError::UnknownLibrary(library_id))?;
    let target = match (library.root_path, library.source_config) {
        (Some(root), _) => Target::Folder(PathBuf::from(root)),
        (None, Some(config)) => {
            match serde_json::from_str(&config).map_err(|_| ScanError::SourceConfig(library_id))? {
                SourceConfig::Komga(config) => {
                    Target::Komga(chek_komga::Komga::new(options.http.clone(), &config)?)
                }
                SourceConfig::Extension(_) => return Err(ScanError::ExtensionSource(library_id)),
            }
        }
        (None, None) => return Err(ScanError::NotAFolder(library_id)),
    };

    let began = {
        let catalog = catalog.clone();
        tokio::task::spawn_blocking(move || catalog.begin_scan(library_id)).await??
    };
    if !began {
        return Err(ScanError::AlreadyScanning(library_id));
    }

    let guard = library_lock(library_id).lock_owned().await;
    let result = match target {
        Target::Folder(root) => run(catalog.clone(), library_id, root, options).await,
        Target::Komga(komga) => sync_source(catalog.clone(), library_id, komga, options).await,
    };
    drop(guard);
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

enum Target {
    Folder(PathBuf),
    Komga(chek_komga::Komga),
}

async fn sync_source(
    catalog: Arc<Catalog>,
    library_id: i64,
    komga: chek_komga::Komga,
    options: ScanOptions,
) -> Result<ScanReport, ScanError> {
    {
        let covers_dir = options.covers_dir.clone();
        tokio::task::spawn_blocking(move || fs::create_dir_all(covers_dir))
            .await?
            .map_err(|source| ScanError::CoversDir {
                path: options.covers_dir.clone(),
                source,
            })?;
    }
    komga::sync(catalog, library_id, komga, options).await
}

/// Serializes scans and added files per library. A scan marks every file it
/// didn't list at its start as unavailable, so a file recorded while it runs
/// would be hidden until the next scan.
fn library_lock(library_id: i64) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: LazyLock<Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>> =
        LazyLock::new(Default::default);
    LOCKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(library_id)
        .or_default()
        .clone()
}

#[derive(Debug, thiserror::Error)]
pub enum AddFileError {
    #[error("library {0} does not exist")]
    UnknownLibrary(i64),
    #[error("library {0} is not a folder library")]
    NotAFolder(i64),
    #[error("{0} is not an EPUB, PDF, CBZ, CBR, CB7 or CBT file")]
    Unsupported(String),
    #[error("cannot create the covers folder {path}: {source}")]
    CoversDir { path: PathBuf, source: io::Error },
    #[error("cannot read {path}: {source}")]
    File { path: String, source: FileError },
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error("task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
}

/// Records one file already in a folder library, such as an upload, and
/// returns its publication id. Waits for a running scan of the library to
/// finish first.
pub async fn add_file(
    catalog: Arc<Catalog>,
    library_id: i64,
    relative_path: String,
    options: ScanOptions,
) -> Result<i64, AddFileError> {
    let _guard = library_lock(library_id).lock_owned().await;
    tokio::task::spawn_blocking(move || {
        let library = catalog
            .library(library_id)?
            .ok_or(AddFileError::UnknownLibrary(library_id))?;
        let root = PathBuf::from(
            library
                .root_path
                .ok_or(AddFileError::NotAFolder(library_id))?,
        );
        let path = root.join(&relative_path);
        let format = chek_formats::detect(&path)
            .map_err(|e| AddFileError::File {
                path: relative_path.clone(),
                source: e.into(),
            })?
            .ok_or_else(|| AddFileError::Unsupported(relative_path.clone()))?;
        fs::create_dir_all(&options.covers_dir).map_err(|source| AddFileError::CoversDir {
            path: options.covers_dir.clone(),
            source,
        })?;
        match scan_file(
            &catalog,
            library_id,
            &root,
            &options.covers_dir,
            &path,
            relative_path.clone(),
            format,
        ) {
            Ok(_) => {}
            Err(FileError::Catalog(e)) => return Err(e.into()),
            Err(source) => {
                return Err(AddFileError::File {
                    path: relative_path,
                    source,
                });
            }
        }
        catalog
            .publication_at(library_id, &relative_path)?
            .ok_or_else(|| AddFileError::Unsupported(relative_path))
    })
    .await?
}

async fn run(
    catalog: Arc<Catalog>,
    library_id: i64,
    root: PathBuf,
    options: ScanOptions,
) -> Result<ScanReport, ScanError> {
    {
        let covers_dir = options.covers_dir.clone();
        tokio::task::spawn_blocking(move || fs::create_dir_all(covers_dir))
            .await?
            .map_err(|source| ScanError::CoversDir {
                path: options.covers_dir.clone(),
                source,
            })?;
    }
    let files = {
        let root = root.clone();
        tokio::task::spawn_blocking(move || list_candidates(&root)).await??
    };

    let mut report = ScanReport {
        files_seen: files.len(),
        ..ScanReport::default()
    };
    let mut seen = HashSet::with_capacity(files.len());
    let permits = Arc::new(Semaphore::new(options.workers.max(1)));
    let covers_dir = Arc::new(options.covers_dir);
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
        let covers_dir = covers_dir.clone();
        tasks.spawn_blocking(move || {
            let _permit = permit;
            let outcome = scan_file(
                &catalog,
                library_id,
                &root,
                &covers_dir,
                &path,
                relative_path,
                format,
            );
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
pub enum FileError {
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
    covers_dir: &Path,
    path: &Path,
    relative_path: String,
    format: Format,
) -> Result<FileOutcome, FileError> {
    let stamp = file_stamp(path)?;
    if catalog.file_stamp(library_id, &relative_path)? == Some(stamp) {
        return Ok(FileOutcome::Unchanged);
    }

    debug!(path = %relative_path, "reading file");
    let inspection = chek_formats::inspect(path, format)?;
    let hash = hash_file(path)?;
    let cover = make_cover(covers_dir, path, format, &inspection.pages, &hash);
    let metadata = inspection.metadata;
    let (series_key, series_title) = match &metadata.series {
        Some(name) => metadata_series(name),
        None => series_for(root, path, &relative_path),
    };
    let title = metadata.title.unwrap_or_else(|| {
        path.file_stem().map_or_else(
            || relative_path.clone(),
            |s| s.to_string_lossy().into_owned(),
        )
    });
    let sort_number = metadata
        .number
        .as_deref()
        .and_then(chek_formats::leading_number);

    catalog.record_file(
        library_id,
        &ScannedFile {
            relative_path,
            series_key,
            series_sort_title: chek_formats::natural_key(&series_title),
            series_title,
            sort_title: chek_formats::natural_key(&title),
            title,
            format: format.as_str(),
            stamp,
            hash,
            cover,
            number_label: metadata.number,
            sort_number,
            details: PublicationDetails {
                summary: metadata.summary,
                publisher: metadata.publisher,
                language: metadata.language,
                published: metadata.published,
                reading_direction: metadata.reading_direction.map(|d| d.as_str().to_owned()),
                contributors: metadata
                    .contributors
                    .into_iter()
                    .map(|c| ContributorRecord {
                        name: c.name,
                        role: c.role.as_str().to_owned(),
                    })
                    .collect(),
            },
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

/// A series named in a file's metadata groups every file with that name in
/// the library, wherever it sits. Names match ignoring case, and the key
/// starts with `/`, which no folder's relative path does.
fn metadata_series(name: &str) -> (String, String) {
    (format!("/series/{}", name.to_lowercase()), name.to_owned())
}

/// Without a series in its metadata, a file's series is the folder it sits in. A file at the library root is a
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

/// Makes the cover thumbnail unless one already exists for this content, and
/// returns its name. A cover that fails is logged and left out; it never fails
/// the file.
fn make_cover(
    covers_dir: &Path,
    path: &Path,
    format: Format,
    pages: &[chek_formats::PageEntry],
    hash: &str,
) -> Option<String> {
    let target = covers_dir.join(format!("{hash}.jpg"));
    if target.exists() {
        return Some(hash.to_owned());
    }
    match chek_formats::cover_thumbnail(path, format, pages) {
        Ok(Some(jpeg)) => save_cover(covers_dir, hash, &jpeg),
        Ok(None) => None,
        Err(e) => {
            warn!(path = %path.display(), error = %e, "could not make cover");
            None
        }
    }
}

/// Writes a cover thumbnail as `name` and returns the name, or `None` when it
/// can't be saved. Writes then renames, so a reader never sees half a cover.
fn save_cover(covers_dir: &Path, name: &str, jpeg: &[u8]) -> Option<String> {
    let partial = covers_dir.join(format!("{name}.jpg.partial"));
    let target = covers_dir.join(format!("{name}.jpg"));
    match fs::write(&partial, jpeg).and_then(|()| fs::rename(&partial, &target)) {
        Ok(()) => Some(name.to_owned()),
        Err(e) => {
            warn!(cover = %target.display(), error = %e, "could not save cover");
            None
        }
    }
}

fn hash_file(path: &Path) -> io::Result<String> {
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(File::open(path)?)?;
    Ok(hasher.finalize().to_hex().to_string())
}
