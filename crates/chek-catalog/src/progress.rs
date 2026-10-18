//! Reading progress: where each user is in each publication.

use rusqlite::{OptionalExtension, params};

use crate::{Catalog, Result};

/// A user's saved position in one publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    /// A Readium Locator as JSON. `None` when nothing has been saved.
    pub locator: Option<String>,
    /// How many times this position was saved; 0 when nothing has been saved.
    pub revision: i64,
    /// The device name of the session that saved it.
    pub device: Option<String>,
    pub updated_at: Option<String>,
}

impl Progress {
    const NONE: Progress = Progress {
        locator: None,
        revision: 0,
        device: None,
        updated_at: None,
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgressSave {
    Saved(Progress),
    /// Another save landed since the revision the caller expected; this is
    /// the position it saved.
    Conflict(Progress),
}

impl Catalog {
    pub fn progress(&self, user_id: i64, publication_id: i64) -> Result<Progress> {
        progress(&self.conn(), user_id, publication_id)
    }

    /// Saves a position if the stored revision is still `expected_revision`,
    /// so a device that missed another device's save can't overwrite it.
    pub fn save_progress(
        &self,
        user_id: i64,
        publication_id: i64,
        expected_revision: i64,
        locator: &str,
        device: Option<&str>,
    ) -> Result<ProgressSave> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let current = progress(&tx, user_id, publication_id)?;
        if current.revision != expected_revision {
            return Ok(ProgressSave::Conflict(current));
        }
        tx.execute(
            "INSERT INTO progress (user_id, publication_id, locator, revision, device, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
             ON CONFLICT (user_id, publication_id) DO UPDATE SET
                 locator = excluded.locator,
                 revision = excluded.revision,
                 device = excluded.device,
                 updated_at = excluded.updated_at",
            params![
                user_id,
                publication_id,
                locator,
                expected_revision + 1,
                device
            ],
        )?;
        let saved = progress(&tx, user_id, publication_id)?;
        tx.commit()?;
        Ok(ProgressSave::Saved(saved))
    }
}

fn progress(conn: &rusqlite::Connection, user_id: i64, publication_id: i64) -> Result<Progress> {
    let found = conn
        .query_row(
            "SELECT locator, revision, device, updated_at FROM progress
             WHERE user_id = ?1 AND publication_id = ?2",
            params![user_id, publication_id],
            |row| {
                Ok(Progress {
                    locator: row.get(0)?,
                    revision: row.get(1)?,
                    device: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            },
        )
        .optional()?;
    Ok(found.unwrap_or(Progress::NONE))
}
