//! Publications from a source library, such as a Komga server. Each item is
//! keyed by the source's own id and served through the source, so Chek keeps
//! no file for it.

use std::collections::{HashMap, HashSet};

use rusqlite::{OptionalExtension, params};

use crate::{Catalog, PageRecord, PublicationDetails, Recorded, Result, record_details};

/// Everything a sync records about one item.
#[derive(Debug, Clone)]
pub struct RemoteItem {
    /// The item's id in its source.
    pub remote_id: String,
    /// Changes whenever the item or its metadata changes in the source, so
    /// unchanged items can be skipped.
    pub version: String,
    /// Names this version of the item; pages are cached against it and its
    /// cover is stored under it.
    pub hash: String,
    pub series_remote_id: String,
    pub series_title: String,
    pub series_sort_title: String,
    pub title: String,
    pub sort_title: String,
    pub format: &'static str,
    pub size: u64,
    pub pages: Vec<PageRecord>,
    pub cover: Option<String>,
    pub number_label: Option<String>,
    pub sort_number: Option<f64>,
    pub details: PublicationDetails,
}

/// A source item behind a publication, for serving it.
#[derive(Debug, Clone)]
pub struct RemoteContent {
    pub publication_id: i64,
    pub library_id: i64,
    pub remote_id: String,
    pub hash: String,
    pub format: String,
    pub title: String,
    pub available: bool,
    pub source_config: String,
}

impl Catalog {
    /// The recorded version of each item in a source library, by remote id.
    pub fn remote_versions(&self, library_id: i64) -> Result<HashMap<String, String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT remote_id, remote_version FROM content
             WHERE library_id = ?1 AND kind = 'remote' AND remote_version IS NOT NULL",
        )?;
        let rows = stmt.query_map([library_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Records a new or changed item. An existing item keeps its publication
    /// id, so progress survives changes in the source.
    pub fn record_remote(&self, library_id: i64, item: &RemoteItem) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let series_id: i64 = tx.query_row(
            "INSERT INTO series (library_id, title, sort_title, remote_id) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (library_id, remote_id) DO UPDATE SET title = excluded.title, sort_title = excluded.sort_title
             RETURNING id",
            params![library_id, item.series_title, item.series_sort_title, item.series_remote_id],
            |row| row.get(0),
        )?;
        let page_count = (!item.pages.is_empty()).then_some(item.pages.len() as i64);
        let existing: Option<(i64, i64)> = tx
            .query_row(
                "SELECT id, publication_id FROM content WHERE library_id = ?1 AND remote_id = ?2 AND kind = 'remote'",
                params![library_id, item.remote_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let (content_id, publication_id) = match existing {
            Some((content_id, publication_id)) => {
                tx.execute(
                    "UPDATE publication SET series_id = ?2, title = ?3, format = ?4, page_count = ?5, available = 1, cover = ?6
                     WHERE id = ?1",
                    params![publication_id, series_id, item.title, item.format, page_count, item.cover],
                )?;
                tx.execute(
                    "UPDATE content SET size = ?2, hash = ?3, remote_version = ?4 WHERE id = ?1",
                    params![content_id, item.size as i64, item.hash, item.version],
                )?;
                tx.execute("DELETE FROM page WHERE content_id = ?1", [content_id])?;
                (content_id, publication_id)
            }
            None => {
                tx.execute(
                    "INSERT INTO publication (series_id, title, format, page_count, cover) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![series_id, item.title, item.format, page_count, item.cover],
                )?;
                let publication_id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO content (publication_id, library_id, kind, size, hash, remote_id, remote_version)
                     VALUES (?1, ?2, 'remote', ?3, ?4, ?5, ?6)",
                    params![
                        publication_id,
                        library_id,
                        item.size as i64,
                        item.hash,
                        item.remote_id,
                        item.version
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
                sort_title: &item.sort_title,
                number_label: item.number_label.as_deref(),
                sort_number: item.sort_number,
                details: &item.details,
                pages: &item.pages,
            },
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Marks each item in a source library available if the sync saw it and
    /// unavailable otherwise, and removes series left empty. Like missing
    /// files, items gone from the source are kept so progress survives.
    pub fn set_remote_availability(&self, library_id: i64, seen: &HashSet<String>) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        {
            let mut select = tx.prepare(
                "SELECT c.remote_id, c.publication_id, p.available
                 FROM content c JOIN publication p ON p.id = c.publication_id
                 WHERE c.library_id = ?1 AND c.kind = 'remote'",
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
                let (remote_id, publication_id, available) = row?;
                let present = seen.contains(&remote_id);
                if present != available {
                    update.execute(params![publication_id, present])?;
                }
            }
        }
        tx.execute(
            "DELETE FROM series WHERE library_id = ?1
             AND NOT EXISTS (SELECT 1 FROM publication p WHERE p.series_id = series.id)",
            [library_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The source item behind a publication. `None` when the publication
    /// doesn't exist or isn't from a source.
    pub fn publication_remote(&self, publication_id: i64) -> Result<Option<RemoteContent>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT p.id, l.id, c.remote_id, c.hash, p.format, p.title, p.available, l.source_config
                 FROM publication p
                 JOIN content c ON c.publication_id = p.id
                 JOIN library l ON l.id = c.library_id
                 WHERE p.id = ?1 AND c.kind = 'remote' AND l.source_config IS NOT NULL",
                [publication_id],
                |row| {
                    Ok(RemoteContent {
                        publication_id: row.get(0)?,
                        library_id: row.get(1)?,
                        remote_id: row.get(2)?,
                        hash: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                        format: row.get(4)?,
                        title: row.get(5)?,
                        available: row.get(6)?,
                        source_config: row.get(7)?,
                    })
                },
            )
            .optional()?)
    }

    /// A publication's cover name. `None` when the publication doesn't
    /// exist; `Some(None)` when it has no cover.
    pub fn publication_cover(&self, publication_id: i64) -> Result<Option<Option<String>>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT cover FROM publication WHERE id = ?1",
                [publication_id],
                |row| row.get(0),
            )
            .optional()?)
    }
}
