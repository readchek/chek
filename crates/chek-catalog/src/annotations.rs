//! Bookmarks and highlights: places a user marked in a publication.

use rusqlite::{OptionalExtension, params};

use crate::{Catalog, Result};

/// What an annotation marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotationKind {
    /// A position.
    Bookmark,
    /// A passage of text, with the text in the locator.
    Highlight,
}

impl AnnotationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AnnotationKind::Bookmark => "bookmark",
            AnnotationKind::Highlight => "highlight",
        }
    }

    fn parse(s: &str) -> Self {
        if s == "highlight" {
            AnnotationKind::Highlight
        } else {
            AnnotationKind::Bookmark
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    pub id: i64,
    pub publication_id: i64,
    pub kind: AnnotationKind,
    /// A Readium Locator as JSON.
    pub locator: String,
    pub color: Option<String>,
    pub note: Option<String>,
    /// The device name of the session that last saved it.
    pub device: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A new annotation's fields.
#[derive(Debug, Clone)]
pub struct NewAnnotation {
    pub kind: AnnotationKind,
    pub locator: String,
    pub color: Option<String>,
    pub note: Option<String>,
}

/// Changes to an annotation. `None` leaves a field as it is; `Some(None)`
/// clears it.
#[derive(Debug, Clone, Default)]
pub struct AnnotationChanges {
    pub color: Option<Option<String>>,
    pub note: Option<Option<String>>,
}

const SELECT: &str =
    "SELECT id, publication_id, kind, locator, color, note, device, created_at, updated_at
     FROM annotation";

impl Catalog {
    /// A user's annotations in a publication, in reading order. Annotations
    /// without a total progression come last, in the order they were made.
    pub fn annotations(&self, user_id: i64, publication_id: i64) -> Result<Vec<Annotation>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "{SELECT}
             WHERE user_id = ?1 AND publication_id = ?2
             ORDER BY json_extract(locator, '$.locations.totalProgression') IS NULL,
                      json_extract(locator, '$.locations.totalProgression'), id"
        ))?;
        let rows = stmt.query_map(params![user_id, publication_id], from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// How many annotations a user has in a publication.
    pub fn annotation_count(&self, user_id: i64, publication_id: i64) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT count(*) FROM annotation WHERE user_id = ?1 AND publication_id = ?2",
            params![user_id, publication_id],
            |row| row.get(0),
        )?)
    }

    /// A user's annotation. `None` when it doesn't exist or belongs to
    /// someone else.
    pub fn annotation(&self, user_id: i64, id: i64) -> Result<Option<Annotation>> {
        Ok(self
            .conn()
            .query_row(
                &format!("{SELECT} WHERE user_id = ?1 AND id = ?2"),
                params![user_id, id],
                from_row,
            )
            .optional()?)
    }

    pub fn add_annotation(
        &self,
        user_id: i64,
        publication_id: i64,
        new: &NewAnnotation,
        device: Option<&str>,
    ) -> Result<Annotation> {
        let conn = self.conn();
        let id: i64 = conn.query_row(
            "INSERT INTO annotation (user_id, publication_id, kind, locator, color, note, device, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7,
                     strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
             RETURNING id",
            params![
                user_id,
                publication_id,
                new.kind.as_str(),
                new.locator,
                new.color,
                new.note,
                device
            ],
            |row| row.get(0),
        )?;
        Ok(conn.query_row(&format!("{SELECT} WHERE id = ?1"), [id], from_row)?)
    }

    /// Applies `changes` to a user's annotation. `None` when it doesn't
    /// exist or belongs to someone else.
    pub fn update_annotation(
        &self,
        user_id: i64,
        id: i64,
        changes: &AnnotationChanges,
        device: Option<&str>,
    ) -> Result<Option<Annotation>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let updated = tx.execute(
            "UPDATE annotation SET
                 color = CASE WHEN ?3 THEN ?4 ELSE color END,
                 note = CASE WHEN ?5 THEN ?6 ELSE note END,
                 device = ?7,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE user_id = ?1 AND id = ?2",
            params![
                user_id,
                id,
                changes.color.is_some(),
                changes.color.clone().flatten(),
                changes.note.is_some(),
                changes.note.clone().flatten(),
                device
            ],
        )?;
        if updated == 0 {
            return Ok(None);
        }
        let annotation = tx.query_row(&format!("{SELECT} WHERE id = ?1"), [id], from_row)?;
        tx.commit()?;
        Ok(Some(annotation))
    }

    /// Deletes a user's annotation. `false` when it doesn't exist or belongs
    /// to someone else.
    pub fn delete_annotation(&self, user_id: i64, id: i64) -> Result<bool> {
        Ok(self.conn().execute(
            "DELETE FROM annotation WHERE user_id = ?1 AND id = ?2",
            params![user_id, id],
        )? > 0)
    }
}

fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Annotation> {
    Ok(Annotation {
        id: row.get(0)?,
        publication_id: row.get(1)?,
        kind: AnnotationKind::parse(&row.get::<_, String>(2)?),
        locator: row.get(3)?,
        color: row.get(4)?,
        note: row.get(5)?,
        device: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}
