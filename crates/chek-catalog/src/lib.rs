//! Chek's data model, SQLite migrations and queries.
//!
//! [`Catalog`] wraps one SQLite connection. Every method blocks, so callers on
//! an async runtime run them on a blocking thread.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, Transaction, params};

mod annotations;
mod edits;
mod progress;
mod users;

pub use annotations::{Annotation, AnnotationChanges, AnnotationKind, NewAnnotation};
pub use edits::{FileLocation, PublicationEdit};
pub use progress::{Progress, ProgressSave};
pub use users::{
    NewUser, Role, Session, SessionUser, User, UserChanges, UserUpdate, UserWithPassword,
};

/// Migrations in order. A database records how many it has applied in
/// `PRAGMA user_version`, so each one runs exactly once.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_catalog.sql"),
    include_str!("../migrations/0002_covers.sql"),
    include_str!("../migrations/0003_users.sql"),
    include_str!("../migrations/0004_progress.sql"),
    include_str!("../migrations/0005_metadata.sql"),
    include_str!("../migrations/0006_session_ids.sql"),
    include_str!("../migrations/0007_sort_titles.sql"),
    include_str!("../migrations/0008_metadata_series.sql"),
    include_str!("../migrations/0009_pdf_metadata.sql"),
    include_str!("../migrations/0010_search.sql"),
    include_str!("../migrations/0011_annotations.sql"),
    include_str!("../migrations/0012_publication_edits.sql"),
];

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("a user named {0:?} already exists")]
    NameTaken(String),
    #[error("database was created by a newer server (schema {found}, this server knows {known})")]
    NewerSchema { found: usize, known: usize },
}

pub type Result<T> = std::result::Result<T, CatalogError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryKind {
    Folder,
    Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanStatus {
    Idle,
    Scanning,
    Failed,
}

impl ScanStatus {
    fn as_str(self) -> &'static str {
        match self {
            ScanStatus::Idle => "idle",
            ScanStatus::Scanning => "scanning",
            ScanStatus::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Library {
    pub id: i64,
    pub name: String,
    pub kind: LibraryKind,
    pub root_path: Option<String>,
    pub scan_status: ScanStatus,
    pub scan_error: Option<String>,
    pub scanned_at: Option<String>,
    pub created_at: String,
}

/// A publication as listed in a library.
#[derive(Debug, Clone)]
pub struct PublicationSummary {
    pub id: i64,
    pub series_id: i64,
    pub series_title: String,
    pub title: String,
    pub number_label: Option<String>,
    pub format: String,
    pub page_count: Option<i64>,
    pub available: bool,
    pub has_cover: bool,
}

/// A series as listed in a library.
#[derive(Debug, Clone)]
pub struct SeriesSummary {
    pub id: i64,
    pub library_id: i64,
    pub title: String,
    pub publication_count: i64,
    /// The first publication in reading order that has a cover, to show as
    /// the series' cover.
    pub cover_publication_id: Option<i64>,
}

/// Where a publication's file is and what the last scan recorded about it.
#[derive(Debug, Clone)]
pub struct PublicationFile {
    pub publication_id: i64,
    pub library_id: i64,
    pub root_path: String,
    pub relative_path: String,
    pub format: String,
    /// `None` until the next scan reads the file again.
    pub stamp: Option<FileStamp>,
    pub hash: String,
    pub available: bool,
    /// The cover thumbnail's name in the covers folder.
    pub cover: Option<String>,
}

/// The size and modification time recorded for a file, which a scan compares
/// against the disk to skip files that haven't changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    pub size: u64,
    pub modified_at: i64,
}

/// Everything a scan records about one file.
#[derive(Debug, Clone)]
pub struct ScannedFile {
    pub relative_path: String,
    pub series_key: String,
    pub series_title: String,
    /// Sorts series in a library; see `chek_formats::natural_key`.
    pub series_sort_title: String,
    pub title: String,
    /// Sorts publications that share a number, or have none, in a series.
    pub sort_title: String,
    pub format: &'static str,
    pub stamp: FileStamp,
    pub hash: String,
    pub pages: Vec<PageRecord>,
    /// The cover thumbnail's name in the covers folder, if one was made.
    pub cover: Option<String>,
    /// The publication's number in its series, as the file writes it.
    pub number_label: Option<String>,
    /// The number for sorting, when the label starts with one.
    pub sort_number: Option<f64>,
    pub details: PublicationDetails,
}

/// Metadata shown when a publication is opened rather than listed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PublicationDetails {
    pub summary: Option<String>,
    pub publisher: Option<String>,
    pub language: Option<String>,
    /// `YYYY`, `YYYY-MM` or `YYYY-MM-DD`.
    pub published: Option<String>,
    /// `ltr` or `rtl`.
    pub reading_direction: Option<String>,
    pub contributors: Vec<ContributorRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContributorRecord {
    pub name: String,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRecord {
    pub entry_name: String,
    pub data_offset: u64,
    pub compressed_size: u64,
    pub size: u64,
    pub compression: &'static str,
}

pub struct Catalog {
    conn: Mutex<Connection>,
}

/// The order publications are listed in, over `publication p` joined with
/// `series s`. Unnumbered publications follow numbered ones; the trailing id
/// makes every key unique.
const ORDER_KEY: &str =
    "s.sort_title, s.id, p.sort_number IS NULL, coalesce(p.sort_number, 0), p.sort_title, p.id";

/// A series with its publication count and the first publication, in
/// reading order, that has a cover.
const SERIES_SELECT: &str = "SELECT s.id, s.library_id, s.title,
        (SELECT count(*) FROM publication p WHERE p.series_id = s.id),
        (SELECT p.id FROM publication p WHERE p.series_id = s.id AND p.cover IS NOT NULL
         ORDER BY p.sort_number IS NULL, coalesce(p.sort_number, 0), p.sort_title, p.id LIMIT 1)
     FROM series s";

impl Catalog {
    /// Opens or creates the database at `path` and applies pending migrations.
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "wal")?;
        conn.pragma_update(None, "synchronous", "normal")?;
        conn.pragma_update(None, "foreign_keys", "on")?;
        migrate(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        // A panic while holding the lock leaves SQLite itself consistent, since
        // every write runs in a transaction, so a poisoned lock is still usable.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn add_folder_library(&self, name: &str, root_path: &str) -> Result<Library> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO library (name, kind, root_path) VALUES (?1, 'folder', ?2)",
            params![name, root_path],
        )?;
        let id = conn.last_insert_rowid();
        Ok(library_by_id(&conn, id)?.expect("library was just inserted"))
    }

    pub fn libraries(&self) -> Result<Vec<Library>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("{LIBRARY_SELECT} ORDER BY id"))?;
        let rows = stmt.query_map([], library_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn library(&self, id: i64) -> Result<Option<Library>> {
        library_by_id(&self.conn(), id)
    }

    /// Marks a library as scanning. Returns false when a scan is already
    /// running, so two scans of one library never overlap.
    pub fn begin_scan(&self, library_id: i64) -> Result<bool> {
        let changed = self.conn().execute(
            "UPDATE library SET scan_status = 'scanning', scan_error = NULL
             WHERE id = ?1 AND scan_status != 'scanning'",
            [library_id],
        )?;
        Ok(changed == 1)
    }

    pub fn finish_scan(&self, library_id: i64, error: Option<&str>) -> Result<()> {
        let status = if error.is_some() {
            ScanStatus::Failed
        } else {
            ScanStatus::Idle
        };
        self.conn().execute(
            "UPDATE library SET scan_status = ?2, scan_error = ?3,
                 scanned_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            params![library_id, status.as_str(), error],
        )?;
        Ok(())
    }

    /// Libraries left in `scanning` by a server that stopped mid-scan.
    pub fn reset_interrupted_scans(&self) -> Result<usize> {
        Ok(self.conn().execute(
            "UPDATE library SET scan_status = 'failed', scan_error = 'The server stopped during the scan.'
             WHERE scan_status = 'scanning'",
            [],
        )?)
    }

    /// The publication for a file in a library, by its path relative to the
    /// library's folder.
    pub fn publication_at(&self, library_id: i64, relative_path: &str) -> Result<Option<i64>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT publication_id FROM content WHERE library_id = ?1 AND relative_path = ?2",
                params![library_id, relative_path],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn file_stamp(&self, library_id: i64, relative_path: &str) -> Result<Option<FileStamp>> {
        let conn = self.conn();
        let stamp = conn
            .query_row(
                "SELECT size, modified_at FROM content
                 WHERE library_id = ?1 AND relative_path = ?2",
                params![library_id, relative_path],
                |row| {
                    let size: Option<i64> = row.get(0)?;
                    let modified_at: Option<i64> = row.get(1)?;
                    Ok(size.zip(modified_at).map(|(size, modified_at)| FileStamp {
                        size: size as u64,
                        modified_at,
                    }))
                },
            )
            .optional()?;
        Ok(stamp.flatten())
    }

    /// Records a new or changed file. An existing file keeps its publication
    /// id, so progress survives edits to the file.
    pub fn record_file(&self, library_id: i64, file: &ScannedFile) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        record_file(&tx, library_id, file)?;
        tx.commit()?;
        Ok(())
    }

    /// Marks each file publication in the library available if its path was
    /// seen by the scan and unavailable otherwise, and removes series with no
    /// publications left. Missing files are kept, not deleted, so a
    /// disconnected drive doesn't erase anyone's progress.
    pub fn set_availability(&self, library_id: i64, seen: &HashSet<String>) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        {
            let mut select = tx.prepare(
                "SELECT c.relative_path, c.publication_id, p.available
                 FROM content c JOIN publication p ON p.id = c.publication_id
                 WHERE c.library_id = ?1 AND c.kind = 'file'",
            )?;
            let mut update = tx.prepare("UPDATE publication SET available = ?2 WHERE id = ?1")?;
            let rows = select.query_map([library_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            })?;
            for row in rows {
                let (path, publication_id, available) = row?;
                let present = seen.contains(&path);
                if present != available {
                    update.execute(params![publication_id, present])?;
                }
            }
        }
        // A series is left empty when its files' metadata moves them to
        // another one.
        tx.execute(
            "DELETE FROM series WHERE library_id = ?1
             AND NOT EXISTS (SELECT 1 FROM publication p WHERE p.series_id = series.id)",
            [library_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Lists a library's publications in reading order, starting after the
    /// publication id `after`. Series sort by title; within a series,
    /// numbered publications come first by number, then the rest by title.
    pub fn publications(
        &self,
        library_id: i64,
        after: Option<i64>,
        limit: u32,
    ) -> Result<Vec<PublicationSummary>> {
        self.list_publications("s.library_id", library_id, after, limit)
    }

    /// Lists a series' publications in reading order, starting after the
    /// publication id `after`.
    pub fn series_publications(
        &self,
        series_id: i64,
        after: Option<i64>,
        limit: u32,
    ) -> Result<Vec<PublicationSummary>> {
        self.list_publications("s.id", series_id, after, limit)
    }

    /// Publications whose `scope` column equals `scope_id`, in [`ORDER_KEY`]
    /// order.
    fn list_publications(
        &self,
        scope: &'static str,
        scope_id: i64,
        after: Option<i64>,
        limit: u32,
    ) -> Result<Vec<PublicationSummary>> {
        // The cursor is the last publication's id; comparing whole sort keys
        // against that row's keeps pages stable and complete as long as the
        // row exists, and publications are never deleted, only marked
        // unavailable.
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT p.id, s.id, s.title, p.title, p.number_label, p.format, p.page_count, p.available, p.cover IS NOT NULL
             FROM publication p JOIN series s ON s.id = p.series_id
             WHERE {scope} = ?1
               AND (?2 IS NULL OR ({ORDER_KEY}) > (
                   SELECT {ORDER_KEY} FROM publication p JOIN series s ON s.id = p.series_id WHERE p.id = ?2))
             ORDER BY {ORDER_KEY}
             LIMIT ?3"
        ))?;
        let rows = stmt.query_map(params![scope_id, after, limit], summary_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Lists a library's series by title, starting after the series id
    /// `after`.
    pub fn series_list(
        &self,
        library_id: i64,
        after: Option<i64>,
        limit: u32,
    ) -> Result<Vec<SeriesSummary>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "{SERIES_SELECT}
             WHERE s.library_id = ?1
               AND (?2 IS NULL OR (s.sort_title, s.id) > (SELECT sort_title, id FROM series WHERE id = ?2))
             ORDER BY s.sort_title, s.id
             LIMIT ?3"
        ))?;
        let rows = stmt.query_map(params![library_id, after, limit], series_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Publications matching every word of `query` in their title, series,
    /// contributors or publisher, best match first. Each word also matches
    /// as the start of a longer word, so results appear while typing. Only
    /// libraries `user` can read are searched, and only `library_id` when
    /// given. Pages are by position, since relevance has no stable key.
    pub fn search(
        &self,
        user: &User,
        library_id: Option<i64>,
        query: &str,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<PublicationSummary>> {
        let Some(expression) = match_expression(query) else {
            return Ok(Vec::new());
        };
        let conn = self.conn();
        // bm25 weights favour a title match over a series, contributor or
        // publisher match. Equal matches, such as every issue of a series,
        // stay in reading order.
        let mut stmt = conn.prepare(&format!(
            "SELECT p.id, s.id, s.title, p.title, p.number_label, p.format, p.page_count, p.available, p.cover IS NOT NULL
             FROM publication_search f
             JOIN publication p ON p.id = f.rowid
             JOIN series s ON s.id = p.series_id
             WHERE publication_search MATCH ?1
               AND (?2 OR s.library_id IN (SELECT library_id FROM library_grant WHERE user_id = ?3))
               AND (?4 IS NULL OR s.library_id = ?4)
             ORDER BY bm25(publication_search, 10.0, 5.0, 2.0, 1.0), {ORDER_KEY}
             LIMIT ?5 OFFSET ?6"
        ))?;
        let rows = stmt.query_map(
            params![
                expression,
                user.role == Role::Admin,
                user.id,
                library_id,
                limit,
                offset
            ],
            summary_from_row,
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Available publications in libraries `user` can read, newest first,
    /// starting after the publication id `after`. Ids grow as scans find
    /// files, so the newest id is the most recently added; a file changed in
    /// place keeps its id and its place.
    pub fn recently_added(
        &self,
        user: &User,
        library_id: Option<i64>,
        after: Option<i64>,
        limit: u32,
    ) -> Result<Vec<PublicationSummary>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT p.id, s.id, s.title, p.title, p.number_label, p.format, p.page_count, p.available, p.cover IS NOT NULL
             FROM publication p JOIN series s ON s.id = p.series_id
             WHERE p.available
               AND (?1 OR s.library_id IN (SELECT library_id FROM library_grant WHERE user_id = ?2))
               AND (?3 IS NULL OR s.library_id = ?3)
               AND (?4 IS NULL OR p.id < ?4)
             ORDER BY p.id DESC
             LIMIT ?5",
        )?;
        let rows = stmt.query_map(
            params![user.role == Role::Admin, user.id, library_id, after, limit],
            summary_from_row,
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn series(&self, id: i64) -> Result<Option<SeriesSummary>> {
        Ok(self
            .conn()
            .query_row(
                &format!("{SERIES_SELECT} WHERE s.id = ?1"),
                [id],
                series_from_row,
            )
            .optional()?)
    }

    pub fn publication(&self, id: i64) -> Result<Option<PublicationSummary>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT p.id, s.id, s.title, p.title, p.number_label, p.format, p.page_count, p.available, p.cover IS NOT NULL
                 FROM publication p JOIN series s ON s.id = p.series_id
                 WHERE p.id = ?1",
                [id],
                summary_from_row,
            )
            .optional()?)
    }

    /// A publication's metadata. `None` when the publication doesn't exist.
    pub fn publication_details(&self, id: i64) -> Result<Option<PublicationDetails>> {
        let conn = self.conn();
        let details = conn
            .query_row(
                "SELECT summary, publisher, language, published, reading_direction
                 FROM publication WHERE id = ?1",
                [id],
                |row| {
                    Ok(PublicationDetails {
                        summary: row.get(0)?,
                        publisher: row.get(1)?,
                        language: row.get(2)?,
                        published: row.get(3)?,
                        reading_direction: row.get(4)?,
                        contributors: Vec::new(),
                    })
                },
            )
            .optional()?;
        let Some(mut details) = details else {
            return Ok(None);
        };
        let mut stmt = conn.prepare(
            "SELECT name, role FROM contributor WHERE publication_id = ?1 ORDER BY position",
        )?;
        details.contributors = stmt
            .query_map([id], |row| {
                Ok(ContributorRecord {
                    name: row.get(0)?,
                    role: row.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Some(details))
    }

    /// The file behind a publication. `None` when the publication doesn't
    /// exist or isn't backed by a local file.
    pub fn publication_file(&self, publication_id: i64) -> Result<Option<PublicationFile>> {
        let file = self
            .conn()
            .query_row(
                "SELECT p.id, l.id, l.root_path, c.relative_path, p.format, c.size, c.modified_at, c.hash, p.available, p.cover
                 FROM publication p
                 JOIN content c ON c.publication_id = p.id
                 JOIN library l ON l.id = c.library_id
                 WHERE p.id = ?1 AND c.kind = 'file' AND l.root_path IS NOT NULL",
                [publication_id],
                |row| {
                    Ok(PublicationFile {
                        publication_id: row.get(0)?,
                        library_id: row.get(1)?,
                        root_path: row.get(2)?,
                        relative_path: row.get(3)?,
                        format: row.get(4)?,
                        stamp: row
                            .get::<_, Option<i64>>(5)?
                            .zip(row.get::<_, Option<i64>>(6)?)
                            .map(|(size, modified_at)| FileStamp {
                                size: size as u64,
                                modified_at,
                            }),
                        hash: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
                        available: row.get(8)?,
                        cover: row.get(9)?,
                    })
                },
            )
            .optional()?;
        Ok(file)
    }

    /// The library a publication belongs to.
    pub fn publication_library(&self, publication_id: i64) -> Result<Option<i64>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT s.library_id FROM publication p JOIN series s ON s.id = p.series_id WHERE p.id = ?1",
                [publication_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// A publication's page index in reading order.
    pub fn pages(&self, publication_id: i64) -> Result<Vec<PageRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT pg.entry_name, pg.data_offset, pg.compressed_size, pg.size, pg.compression
             FROM page pg JOIN content c ON c.id = pg.content_id
             WHERE c.publication_id = ?1 AND pg.entry_name IS NOT NULL
             ORDER BY pg.page_index",
        )?;
        let rows = stmt.query_map([publication_id], page_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// One page of a publication, by its zero-based index.
    pub fn page(&self, publication_id: i64, index: u32) -> Result<Option<PageRecord>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT pg.entry_name, pg.data_offset, pg.compressed_size, pg.size, pg.compression
                 FROM page pg JOIN content c ON c.id = pg.content_id
                 WHERE c.publication_id = ?1 AND pg.page_index = ?2 AND pg.entry_name IS NOT NULL",
                params![publication_id, index],
                page_from_row,
            )
            .optional()?)
    }
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let applied: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let applied = usize::try_from(applied).unwrap_or(usize::MAX);
    if applied > MIGRATIONS.len() {
        return Err(CatalogError::NewerSchema {
            found: applied,
            known: MIGRATIONS.len(),
        });
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(applied) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

const LIBRARY_SELECT: &str = "SELECT id, name, kind, root_path, scan_status, scan_error, scanned_at, created_at FROM library";

fn library_by_id(conn: &Connection, id: i64) -> Result<Option<Library>> {
    Ok(conn
        .query_row(
            &format!("{LIBRARY_SELECT} WHERE id = ?1"),
            [id],
            library_from_row,
        )
        .optional()?)
}

fn library_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Library> {
    let kind = match row.get_ref(2)?.as_str()? {
        "source" => LibraryKind::Source,
        _ => LibraryKind::Folder,
    };
    let scan_status = match row.get_ref(4)?.as_str()? {
        "scanning" => ScanStatus::Scanning,
        "failed" => ScanStatus::Failed,
        _ => ScanStatus::Idle,
    };
    Ok(Library {
        id: row.get(0)?,
        name: row.get(1)?,
        kind,
        root_path: row.get(3)?,
        scan_status,
        scan_error: row.get(5)?,
        scanned_at: row.get(6)?,
        created_at: row.get(7)?,
    })
}

/// Turns what a person typed into an FTS5 query: each word quoted, so
/// characters FTS5 treats as syntax are searched as text, and marked as a
/// prefix. `None` when nothing searchable is left.
fn match_expression(query: &str) -> Option<String> {
    let terms: Vec<String> = query
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .map(|word| format!("\"{}\"*", word.replace('"', "\"\"")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

fn series_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SeriesSummary> {
    Ok(SeriesSummary {
        id: row.get(0)?,
        library_id: row.get(1)?,
        title: row.get(2)?,
        publication_count: row.get(3)?,
        cover_publication_id: row.get(4)?,
    })
}

fn summary_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PublicationSummary> {
    Ok(PublicationSummary {
        id: row.get(0)?,
        series_id: row.get(1)?,
        series_title: row.get(2)?,
        title: row.get(3)?,
        number_label: row.get(4)?,
        format: row.get(5)?,
        page_count: row.get(6)?,
        available: row.get(7)?,
        has_cover: row.get(8)?,
    })
}

fn page_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PageRecord> {
    let compression = match row.get_ref(4)?.as_str_or_null()? {
        Some("deflated") => "deflated",
        _ => "stored",
    };
    Ok(PageRecord {
        entry_name: row.get(0)?,
        data_offset: row.get::<_, i64>(1)? as u64,
        compressed_size: row.get::<_, i64>(2)? as u64,
        size: row.get::<_, i64>(3)? as u64,
        compression,
    })
}

/// Rewrites a publication's search index row from what's stored now.
fn index_for_search(conn: &Connection, publication_id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM publication_search WHERE rowid = ?1",
        [publication_id],
    )?;
    conn.execute(
        "INSERT INTO publication_search (rowid, title, series, contributors, publisher)
         SELECT p.id, p.title, s.title,
                (SELECT group_concat(c.name, ' ') FROM contributor c WHERE c.publication_id = p.id),
                p.publisher
         FROM publication p JOIN series s ON s.id = p.series_id
         WHERE p.id = ?1",
        [publication_id],
    )?;
    Ok(())
}

fn record_file(tx: &Transaction<'_>, library_id: i64, file: &ScannedFile) -> Result<()> {
    let series_id: i64 = tx.query_row(
        "INSERT INTO series (library_id, title, sort_title, path_key) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (library_id, path_key) DO UPDATE SET title = excluded.title, sort_title = excluded.sort_title
         RETURNING id",
        params![library_id, file.series_title, file.series_sort_title, file.series_key],
        |row| row.get(0),
    )?;

    let page_count = (!file.pages.is_empty()).then_some(file.pages.len() as i64);
    let existing: Option<(i64, i64)> = tx
        .query_row(
            "SELECT id, publication_id FROM content WHERE library_id = ?1 AND relative_path = ?2",
            params![library_id, file.relative_path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;

    let (content_id, publication_id) = match existing {
        Some((content_id, publication_id)) => {
            tx.execute(
                "UPDATE publication SET series_id = ?2, title = ?3, format = ?4, page_count = ?5, available = 1, cover = ?6
                 WHERE id = ?1",
                params![publication_id, series_id, file.title, file.format, page_count, file.cover],
            )?;
            tx.execute(
                "UPDATE content SET size = ?2, hash = ?3, modified_at = ?4 WHERE id = ?1",
                params![
                    content_id,
                    file.stamp.size as i64,
                    file.hash,
                    file.stamp.modified_at
                ],
            )?;
            tx.execute("DELETE FROM page WHERE content_id = ?1", [content_id])?;
            (content_id, publication_id)
        }
        None => {
            tx.execute(
                "INSERT INTO publication (series_id, title, format, page_count, cover) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![series_id, file.title, file.format, page_count, file.cover],
            )?;
            let publication_id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO content (publication_id, library_id, kind, relative_path, size, hash, modified_at)
                 VALUES (?1, ?2, 'file', ?3, ?4, ?5, ?6)",
                params![
                    publication_id,
                    library_id,
                    file.relative_path,
                    file.stamp.size as i64,
                    file.hash,
                    file.stamp.modified_at
                ],
            )?;
            (tx.last_insert_rowid(), publication_id)
        }
    };

    let details = &file.details;
    tx.execute(
        "UPDATE publication SET number_label = ?2, sort_number = ?3, summary = ?4, publisher = ?5,
             language = ?6, published = ?7, reading_direction = ?8, sort_title = ?9
         WHERE id = ?1",
        params![
            publication_id,
            file.number_label,
            file.sort_number,
            details.summary,
            details.publisher,
            details.language,
            details.published,
            details.reading_direction,
            file.sort_title
        ],
    )?;
    tx.execute(
        "DELETE FROM contributor WHERE publication_id = ?1",
        [publication_id],
    )?;
    let mut insert_contributor = tx.prepare(
        "INSERT INTO contributor (publication_id, position, name, role) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (position, c) in details.contributors.iter().enumerate() {
        insert_contributor.execute(params![publication_id, position as i64, c.name, c.role])?;
    }

    edits::apply(tx, publication_id)?;
    index_for_search(tx, publication_id)?;

    let mut insert = tx.prepare(
        "INSERT INTO page (content_id, page_index, entry_name, data_offset, compressed_size, size, compression)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for (index, page) in file.pages.iter().enumerate() {
        insert.execute(params![
            content_id,
            index as i64,
            page.entry_name,
            page.data_offset as i64,
            page.compressed_size as i64,
            page.size as i64,
            page.compression
        ])?;
    }
    Ok(())
}
