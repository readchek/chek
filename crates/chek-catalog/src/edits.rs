//! Metadata an admin changed by hand. Edits are kept apart from what the
//! file says and written over it after every scan of the file, so they
//! survive rescans and clearing them brings the file's own metadata back.

use rusqlite::{Connection, OptionalExtension, params};

use crate::{Catalog, ContributorRecord, Result, index_for_search};

/// Changes to a publication's metadata. `None` leaves a field as it is;
/// `Some(None)` clears it.
#[derive(Debug, Clone, Default)]
pub struct PublicationEdit {
    /// The title and its sort key. A title can't be cleared.
    pub title: Option<(String, String)>,
    /// The number label and the number it sorts by.
    pub number: Option<Option<(String, Option<f64>)>>,
    pub summary: Option<Option<String>>,
    pub publisher: Option<Option<String>>,
    pub language: Option<Option<String>>,
    pub published: Option<Option<String>>,
    /// `ltr` or `rtl`.
    pub reading_direction: Option<Option<String>>,
    pub contributors: Option<Vec<ContributorRecord>>,
}

/// Where a publication's file is, so it can be read again.
#[derive(Debug, Clone)]
pub struct FileLocation {
    pub library_id: i64,
    pub relative_path: String,
}

impl Catalog {
    /// Records `edit` on top of earlier edits and applies them. `false` when
    /// the publication doesn't exist.
    pub fn edit_publication(&self, publication_id: i64, edit: &PublicationEdit) -> Result<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let exists = tx
            .query_row(
                "SELECT 1 FROM publication WHERE id = ?1",
                [publication_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            return Ok(false);
        }
        tx.execute(
            "INSERT OR IGNORE INTO publication_edit (publication_id) VALUES (?1)",
            [publication_id],
        )?;
        if let Some((title, sort_title)) = &edit.title {
            tx.execute(
                "UPDATE publication_edit SET title = ?2, sort_title = ?3 WHERE publication_id = ?1",
                params![publication_id, title, sort_title],
            )?;
        }
        if let Some(number) = &edit.number {
            let (label, sort_number) = match number {
                Some((label, sort_number)) => (Some(label), *sort_number),
                None => (None, None),
            };
            tx.execute(
                "UPDATE publication_edit SET number_set = 1, number_label = ?2, sort_number = ?3
                 WHERE publication_id = ?1",
                params![publication_id, label, sort_number],
            )?;
        }
        for (column, value) in [
            ("summary", &edit.summary),
            ("publisher", &edit.publisher),
            ("language", &edit.language),
            ("published", &edit.published),
        ] {
            if let Some(value) = value {
                tx.execute(
                    &format!(
                        "UPDATE publication_edit SET {column}_set = 1, {column} = ?2
                         WHERE publication_id = ?1"
                    ),
                    params![publication_id, value],
                )?;
            }
        }
        if let Some(direction) = &edit.reading_direction {
            tx.execute(
                "UPDATE publication_edit SET direction_set = 1, reading_direction = ?2
                 WHERE publication_id = ?1",
                params![publication_id, direction],
            )?;
        }
        if let Some(contributors) = &edit.contributors {
            tx.execute(
                "UPDATE publication_edit SET contributors_set = 1 WHERE publication_id = ?1",
                [publication_id],
            )?;
            tx.execute(
                "DELETE FROM publication_edit_contributor WHERE publication_id = ?1",
                [publication_id],
            )?;
            let mut insert = tx.prepare(
                "INSERT INTO publication_edit_contributor (publication_id, position, name, role)
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (position, c) in contributors.iter().enumerate() {
                insert.execute(params![publication_id, position as i64, c.name, c.role])?;
            }
        }
        apply(&tx, publication_id)?;
        index_for_search(&tx, publication_id)?;
        tx.commit()?;
        Ok(true)
    }

    /// The fields an admin edited, by their API names.
    pub fn edited_fields(&self, publication_id: i64) -> Result<Vec<&'static str>> {
        let row = self
            .conn()
            .query_row(
                "SELECT title IS NOT NULL, number_set, summary_set, publisher_set, language_set,
                        published_set, direction_set, contributors_set
                 FROM publication_edit WHERE publication_id = ?1",
                [publication_id],
                |row| {
                    let mut flags = [false; 8];
                    for (i, flag) in flags.iter_mut().enumerate() {
                        *flag = row.get(i)?;
                    }
                    Ok(flags)
                },
            )
            .optional()?;
        const NAMES: [&str; 8] = [
            "title",
            "number",
            "description",
            "publisher",
            "language",
            "published",
            "readingDirection",
            "contributors",
        ];
        Ok(row.map_or_else(Vec::new, |flags| {
            NAMES
                .iter()
                .zip(flags)
                .filter_map(|(name, set)| set.then_some(*name))
                .collect()
        }))
    }

    /// Forgets a publication's edits and its file's stamp, so the next read
    /// of the file brings its own metadata back. Returns where the file is,
    /// or `None` when the publication has no file, such as one from a source,
    /// which its library's next sync reads again.
    pub fn clear_edits(&self, publication_id: i64) -> Result<Option<FileLocation>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM publication_edit WHERE publication_id = ?1",
            [publication_id],
        )?;
        tx.execute(
            "UPDATE content SET modified_at = NULL WHERE publication_id = ?1 AND kind = 'file'",
            [publication_id],
        )?;
        // A source's item is read again by the library's next sync.
        tx.execute(
            "UPDATE content SET remote_version = NULL WHERE publication_id = ?1 AND kind = 'remote'",
            [publication_id],
        )?;
        let location = tx
            .query_row(
                "SELECT library_id, relative_path FROM content
                 WHERE publication_id = ?1 AND kind = 'file'",
                [publication_id],
                |row| {
                    Ok(FileLocation {
                        library_id: row.get(0)?,
                        relative_path: row.get(1)?,
                    })
                },
            )
            .optional()?;
        tx.commit()?;
        Ok(location)
    }
}

/// Writes a publication's edits over its stored metadata.
pub(crate) fn apply(conn: &Connection, publication_id: i64) -> Result<()> {
    let edited = conn.execute(
        "UPDATE publication SET
             title = coalesce(e.title, publication.title),
             sort_title = coalesce(e.sort_title, publication.sort_title),
             number_label = CASE WHEN e.number_set THEN e.number_label ELSE publication.number_label END,
             sort_number = CASE WHEN e.number_set THEN e.sort_number ELSE publication.sort_number END,
             summary = CASE WHEN e.summary_set THEN e.summary ELSE publication.summary END,
             publisher = CASE WHEN e.publisher_set THEN e.publisher ELSE publication.publisher END,
             language = CASE WHEN e.language_set THEN e.language ELSE publication.language END,
             published = CASE WHEN e.published_set THEN e.published ELSE publication.published END,
             reading_direction = CASE WHEN e.direction_set THEN e.reading_direction
                                      ELSE publication.reading_direction END
         FROM publication_edit e
         WHERE e.publication_id = publication.id AND publication.id = ?1",
        [publication_id],
    )?;
    if edited == 0 {
        return Ok(());
    }
    let contributors_set: bool = conn.query_row(
        "SELECT contributors_set FROM publication_edit WHERE publication_id = ?1",
        [publication_id],
        |row| row.get(0),
    )?;
    if contributors_set {
        conn.execute(
            "DELETE FROM contributor WHERE publication_id = ?1",
            [publication_id],
        )?;
        conn.execute(
            "INSERT INTO contributor (publication_id, position, name, role)
             SELECT publication_id, position, name, role FROM publication_edit_contributor
             WHERE publication_id = ?1",
            [publication_id],
        )?;
    }
    Ok(())
}
