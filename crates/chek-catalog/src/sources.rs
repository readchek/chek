//! Source libraries that run on an extension: series an admin followed, with
//! a publication per chapter backed by remote content. Unlike a Komga
//! library, which lists everything its server holds, only followed series
//! are synced, and a chapter's pages are fetched from the extension when
//! it's first read.

use rusqlite::{OptionalExtension, params};

use crate::{Catalog, ContributorRecord, PublicationDetails, Recorded, Result, record_details};

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

impl Catalog {
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
    /// Chapters keep their publication ids and edits across updates, so
    /// progress survives; chapters the source no longer lists are marked
    /// unavailable rather than deleted, and a chapter whose version changed
    /// loses its stored pages so they're fetched again.
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
        crate::edits::keep_series_edit(&tx, series_id)?;
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
            let (content_id, publication_id) = match existing {
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
                    (content_id, publication_id)
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
                    (tx.last_insert_rowid(), publication_id)
                }
            };
            record_details(
                &tx,
                publication_id,
                content_id,
                &Recorded {
                    sort_title: &chapter.sort_title,
                    number_label: chapter.number_label.as_deref(),
                    sort_number: chapter.sort_number,
                    details: &PublicationDetails {
                        summary: series.summary.clone(),
                        publisher: None,
                        language: None,
                        published: chapter.published.clone(),
                        // Publications allow only ltr and rtl; vertical
                        // strips stay on the series.
                        reading_direction: series
                            .reading_direction
                            .clone()
                            .filter(|d| matches!(d.as_str(), "ltr" | "rtl")),
                        contributors: series.contributors.clone(),
                    },
                    // Pages are fetched when the chapter is first read.
                    pages: &[],
                },
            )?;
        }
        tx.commit()?;
        Ok(series_id)
    }

    /// A chapter's stored pages in reading order, as the image requests the
    /// extension returned, in the server's JSON. Empty until it's read.
    pub fn remote_pages(&self, publication_id: i64) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT pg.remote_key FROM page pg JOIN content c ON c.id = pg.content_id
             WHERE c.publication_id = ?1 AND c.kind = 'remote' AND pg.remote_key IS NOT NULL
             ORDER BY pg.page_index",
        )?;
        let rows = stmt.query_map([publication_id], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Replaces a chapter's stored pages.
    pub fn set_remote_pages(&self, publication_id: i64, pages: &[String]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let content_id: Option<i64> = tx
            .query_row(
                "SELECT id FROM content WHERE publication_id = ?1 AND kind = 'remote'",
                [publication_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(content_id) = content_id else {
            return Ok(());
        };
        tx.execute("DELETE FROM page WHERE content_id = ?1", [content_id])?;
        for (index, key) in pages.iter().enumerate() {
            tx.execute(
                "INSERT INTO page (content_id, page_index, remote_key) VALUES (?1, ?2, ?3)",
                params![content_id, index as i64, key],
            )?;
        }
        tx.execute(
            "UPDATE publication SET page_count = ?2 WHERE id = ?1",
            params![publication_id, pages.len() as i64],
        )?;
        tx.commit()?;
        Ok(())
    }
}
