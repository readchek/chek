//! Source libraries: series followed through an extension, with a
//! publication per chapter backed by remote content.

use rusqlite::{OptionalExtension, params};

use crate::{Catalog, ContributorRecord, Library, Result, index_for_search, library_by_id};

/// A followed series as the source describes it.
#[derive(Debug, Clone)]
pub struct RemoteSeries {
    pub remote_id: String,
    pub title: String,
    /// Sorts series in a library; see `chek_formats::natural_key`.
    pub sort_title: String,
    /// `ltr`, `rtl` or `ttb`.
    pub reading_direction: Option<String>,
    pub summary: Option<String>,
    pub contributors: Vec<ContributorRecord>,
    /// The cover thumbnail's name in the covers folder, shared by every
    /// chapter.
    pub cover: Option<String>,
}

/// One chapter of a followed series.
#[derive(Debug, Clone)]
pub struct RemoteChapter {
    pub remote_id: String,
    pub title: String,
    pub sort_title: String,
    pub number_label: Option<String>,
    pub sort_number: Option<f64>,
    /// Changes when the chapter's pages change.
    pub version: Option<String>,
    pub published: Option<String>,
}

/// The remote content behind a publication.
#[derive(Debug, Clone)]
pub struct RemoteContent {
    pub content_id: i64,
    pub library_id: i64,
    pub remote_id: String,
    pub available: bool,
    /// Whether the stored page list belongs to the current version.
    pub has_pages: bool,
}

impl Catalog {
    /// Adds a library whose series come from an extension. `config` is the
    /// server's JSON naming the extension and whose install runs it.
    pub fn add_source_library(&self, name: &str, config: &str) -> Result<Library> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO library (name, kind, source_config) VALUES (?1, 'source', ?2)",
            params![name, config],
        )?;
        let id = conn.last_insert_rowid();
        Ok(library_by_id(&conn, id)?.expect("library was just inserted"))
    }

    /// A source library's config; `None` for folder libraries.
    pub fn library_source_config(&self, library_id: i64) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT source_config FROM library WHERE id = ?1",
                [library_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    /// The source's ids of the series a library follows.
    pub fn remote_series_ids(&self, library_id: i64) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT remote_id FROM series WHERE library_id = ?1 AND remote_id IS NOT NULL ORDER BY id",
        )?;
        let rows = stmt.query_map([library_id], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Records a followed series and its chapters, returning the series id.
    /// Chapters keep their publication ids across updates, so progress
    /// survives; chapters the source no longer lists are marked unavailable
    /// rather than deleted, and a chapter whose version changed loses its
    /// stored pages so they're fetched again.
    pub fn record_remote_series(
        &self,
        library_id: i64,
        series: &RemoteSeries,
        chapters: &[RemoteChapter],
    ) -> Result<i64> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let series_id: i64 = tx.query_row(
            "INSERT INTO series (library_id, title, sort_title, reading_direction, remote_id)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (library_id, remote_id) DO UPDATE SET title = excluded.title,
                 sort_title = excluded.sort_title, reading_direction = excluded.reading_direction
             RETURNING id",
            params![
                library_id,
                series.title,
                series.sort_title,
                series.reading_direction,
                series.remote_id
            ],
            |row| row.get(0),
        )?;
        // Publications allow only ltr and rtl; vertical strips stay on the
        // series.
        let direction = series
            .reading_direction
            .as_deref()
            .filter(|d| matches!(*d, "ltr" | "rtl"));
        tx.execute(
            "UPDATE publication SET available = 0 WHERE series_id = ?1",
            [series_id],
        )?;
        for chapter in chapters {
            let existing: Option<(i64, i64, Option<String>)> = tx
                .query_row(
                    "SELECT id, publication_id, remote_version FROM content
                     WHERE library_id = ?1 AND kind = 'remote' AND remote_id = ?2",
                    params![library_id, chapter.remote_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            let publication_id = match existing {
                Some((content_id, publication_id, version)) => {
                    if version != chapter.version {
                        tx.execute("DELETE FROM page WHERE content_id = ?1", [content_id])?;
                        tx.execute(
                            "UPDATE content SET remote_version = ?2 WHERE id = ?1",
                            params![content_id, chapter.version],
                        )?;
                        tx.execute(
                            "UPDATE publication SET page_count = NULL WHERE id = ?1",
                            [publication_id],
                        )?;
                    }
                    tx.execute(
                        "UPDATE publication SET series_id = ?2, title = ?3, available = 1, cover = ?4
                         WHERE id = ?1",
                        params![publication_id, series_id, chapter.title, series.cover],
                    )?;
                    publication_id
                }
                None => {
                    tx.execute(
                        "INSERT INTO publication (series_id, title, format, cover) VALUES (?1, ?2, 'remote', ?3)",
                        params![series_id, chapter.title, series.cover],
                    )?;
                    let publication_id = tx.last_insert_rowid();
                    tx.execute(
                        "INSERT INTO content (publication_id, library_id, kind, remote_id, remote_version)
                         VALUES (?1, ?2, 'remote', ?3, ?4)",
                        params![
                            publication_id,
                            library_id,
                            chapter.remote_id,
                            chapter.version
                        ],
                    )?;
                    publication_id
                }
            };
            tx.execute(
                "UPDATE publication SET number_label = ?2, sort_number = ?3, sort_title = ?4,
                     summary = ?5, published = ?6, reading_direction = ?7
                 WHERE id = ?1",
                params![
                    publication_id,
                    chapter.number_label,
                    chapter.sort_number,
                    chapter.sort_title,
                    series.summary,
                    chapter.published,
                    direction
                ],
            )?;
            tx.execute(
                "DELETE FROM contributor WHERE publication_id = ?1",
                [publication_id],
            )?;
            for (position, c) in series.contributors.iter().enumerate() {
                tx.execute(
                    "INSERT INTO contributor (publication_id, position, name, role) VALUES (?1, ?2, ?3, ?4)",
                    params![publication_id, position as i64, c.name, c.role],
                )?;
            }
            index_for_search(&tx, publication_id)?;
        }
        tx.commit()?;
        Ok(series_id)
    }

    /// The remote content behind a publication; `None` for file
    /// publications and missing ones.
    pub fn remote_content(&self, publication_id: i64) -> Result<Option<RemoteContent>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT c.id, c.library_id, c.remote_id, p.available,
                        EXISTS (SELECT 1 FROM page pg WHERE pg.content_id = c.id)
                 FROM content c JOIN publication p ON p.id = c.publication_id
                 WHERE c.publication_id = ?1 AND c.kind = 'remote'",
                [publication_id],
                |row| {
                    Ok(RemoteContent {
                        content_id: row.get(0)?,
                        library_id: row.get(1)?,
                        remote_id: row.get(2)?,
                        available: row.get(3)?,
                        has_pages: row.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    /// A publication's cover thumbnail name, from a file or a source.
    pub fn publication_cover(&self, publication_id: i64) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT cover FROM publication WHERE id = ?1",
                [publication_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    /// A remote publication's pages in reading order, as the image requests
    /// the extension returned, in the server's JSON.
    pub fn remote_pages(&self, publication_id: i64) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT pg.remote_key FROM page pg JOIN content c ON c.id = pg.content_id
             WHERE c.publication_id = ?1 AND pg.remote_key IS NOT NULL
             ORDER BY pg.page_index",
        )?;
        let rows = stmt.query_map([publication_id], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Replaces a remote publication's pages.
    pub fn set_remote_pages(&self, content_id: i64, pages: &[String]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM page WHERE content_id = ?1", [content_id])?;
        for (index, key) in pages.iter().enumerate() {
            tx.execute(
                "INSERT INTO page (content_id, page_index, remote_key) VALUES (?1, ?2, ?3)",
                params![content_id, index as i64, key],
            )?;
        }
        tx.execute(
            "UPDATE publication SET page_count = ?2
             WHERE id = (SELECT publication_id FROM content WHERE id = ?1)",
            params![content_id, pages.len() as i64],
        )?;
        tx.commit()?;
        Ok(())
    }
}
