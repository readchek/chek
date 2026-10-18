//! Extension repositories and the settings that govern them.

use rusqlite::{OptionalExtension, params};

use crate::{Catalog, CatalogError, Result};

/// A repository extensions are installed from.
#[derive(Debug, Clone)]
pub struct ExtensionRepository {
    pub id: i64,
    pub url: String,
    /// `None` for a repository on the admin's allow list, open to everyone.
    pub owner_id: Option<i64>,
    pub name: Option<String>,
    /// The last index.json fetched, kept when a later fetch fails.
    pub index: Option<String>,
    pub fetched_at: Option<String>,
    /// Why the last fetch failed, cleared by the next one that works.
    pub fetch_error: Option<String>,
    pub created_at: String,
}

const SELECT: &str = r#"SELECT id, url, owner_id, name, "index", fetched_at, fetch_error, created_at
     FROM extension_repository"#;

const ALLOW_ANY_REPOSITORY: &str = "allow_any_repository";

impl Catalog {
    /// Repositories on the allow list, then the ones `user_id` added, each
    /// in the order they were added.
    pub fn extension_repositories(&self, user_id: i64) -> Result<Vec<ExtensionRepository>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "{SELECT} WHERE owner_id IS NULL OR owner_id = ?1
             ORDER BY owner_id IS NOT NULL, id"
        ))?;
        let rows = stmt.query_map([user_id], from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn extension_repository(&self, id: i64) -> Result<Option<ExtensionRepository>> {
        Ok(self
            .conn()
            .query_row(&format!("{SELECT} WHERE id = ?1"), [id], from_row)
            .optional()?)
    }

    /// Adds a repository with the index just fetched from it. Fails with
    /// [`CatalogError::RepositoryExists`] when the allow list, or the
    /// owner's own list, already has the URL.
    pub fn add_extension_repository(
        &self,
        url: &str,
        owner_id: Option<i64>,
        name: Option<&str>,
        index: &str,
    ) -> Result<ExtensionRepository> {
        let conn = self.conn();
        let inserted = conn.query_row(
            r#"INSERT INTO extension_repository (url, owner_id, name, "index", fetched_at)
             VALUES (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
             RETURNING id"#,
            params![url, owner_id, name, index],
            |row| row.get::<_, i64>(0),
        );
        let id = match inserted {
            Ok(id) => id,
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                return Err(CatalogError::RepositoryExists(url.to_owned()));
            }
            Err(e) => return Err(e.into()),
        };
        Ok(conn.query_row(&format!("{SELECT} WHERE id = ?1"), [id], from_row)?)
    }

    /// Records a fetch of a repository's index: the new index and name when
    /// it worked, or why it failed, keeping the last good index.
    pub fn record_repository_fetch(
        &self,
        id: i64,
        fetched: std::result::Result<(Option<&str>, &str), &str>,
    ) -> Result<Option<ExtensionRepository>> {
        let conn = self.conn();
        match fetched {
            Ok((name, index)) => conn.execute(
                r#"UPDATE extension_repository
                 SET name = ?2, "index" = ?3, fetch_error = NULL,
                     fetched_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
                 WHERE id = ?1"#,
                params![id, name, index],
            )?,
            Err(error) => conn.execute(
                "UPDATE extension_repository SET fetch_error = ?2 WHERE id = ?1",
                params![id, error],
            )?,
        };
        Ok(conn
            .query_row(&format!("{SELECT} WHERE id = ?1"), [id], from_row)
            .optional()?)
    }

    pub fn delete_extension_repository(&self, id: i64) -> Result<bool> {
        Ok(self
            .conn()
            .execute("DELETE FROM extension_repository WHERE id = ?1", [id])?
            > 0)
    }

    /// Whether users may add repositories that aren't on the allow list.
    pub fn allow_any_repository(&self) -> Result<bool> {
        let value: Option<String> = self
            .conn()
            .query_row(
                "SELECT value FROM server_setting WHERE key = ?1",
                [ALLOW_ANY_REPOSITORY],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.as_deref() == Some("true"))
    }

    pub fn set_allow_any_repository(&self, allow: bool) -> Result<()> {
        self.conn().execute(
            "INSERT INTO server_setting (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![ALLOW_ANY_REPOSITORY, if allow { "true" } else { "false" }],
        )?;
        Ok(())
    }
}

fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ExtensionRepository> {
    Ok(ExtensionRepository {
        id: row.get(0)?,
        url: row.get(1)?,
        owner_id: row.get(2)?,
        name: row.get(3)?,
        index: row.get(4)?,
        fetched_at: row.get(5)?,
        fetch_error: row.get(6)?,
        created_at: row.get(7)?,
    })
}
