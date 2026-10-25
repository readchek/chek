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
    /// How to sign in to a private repository, as the server's JSON.
    pub credentials: Option<String>,
}

/// An extension a user installed.
#[derive(Debug, Clone)]
pub struct ExtensionInstall {
    pub user_id: i64,
    pub extension_id: String,
    pub repository_id: i64,
    pub name: String,
    pub version: String,
    /// Names the component file in the data folder.
    pub sha256: String,
    /// JSON array of the hosts the manifest declares.
    pub sites: String,
    /// JSON array of BCP 47 tags.
    pub languages: String,
    /// JSON array of the formats its chapters come in, such as `images`.
    pub formats: String,
    pub installed_at: String,
}

const SELECT_INSTALL: &str = "SELECT user_id, extension_id, repository_id, name, version, sha256,
            sites, languages, installed_at, formats
     FROM extension_install";

const SELECT: &str = r#"SELECT id, url, owner_id, name, "index", fetched_at, fetch_error, created_at,
            credentials
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

    /// Every repository, everyone's, for refreshing them all.
    pub fn all_extension_repositories(&self) -> Result<Vec<ExtensionRepository>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("{SELECT} ORDER BY id"))?;
        let rows = stmt.query_map([], from_row)?;
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
        credentials: Option<&str>,
    ) -> Result<ExtensionRepository> {
        let conn = self.conn();
        let inserted = conn.query_row(
            r#"INSERT INTO extension_repository (url, owner_id, name, "index", fetched_at, credentials)
             VALUES (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), ?5)
             RETURNING id"#,
            params![url, owner_id, name, index, credentials],
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

    /// Replaces a repository's credentials along with the index fetched
    /// with them.
    pub fn set_repository_credentials(
        &self,
        id: i64,
        credentials: Option<&str>,
        name: Option<&str>,
        index: &str,
    ) -> Result<Option<ExtensionRepository>> {
        let conn = self.conn();
        conn.execute(
            r#"UPDATE extension_repository
             SET credentials = ?2, name = ?3, "index" = ?4, fetch_error = NULL,
                 fetched_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1"#,
            params![id, credentials, name, index],
        )?;
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

impl Catalog {
    /// A user's installed extensions, by name.
    pub fn extension_installs(&self, user_id: i64) -> Result<Vec<ExtensionInstall>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "{SELECT_INSTALL} WHERE user_id = ?1 ORDER BY name COLLATE NOCASE, extension_id"
        ))?;
        let rows = stmt.query_map([user_id], install_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn extension_install(
        &self,
        user_id: i64,
        extension_id: &str,
    ) -> Result<Option<ExtensionInstall>> {
        Ok(self
            .conn()
            .query_row(
                &format!("{SELECT_INSTALL} WHERE user_id = ?1 AND extension_id = ?2"),
                params![user_id, extension_id],
                install_from_row,
            )
            .optional()?)
    }

    /// Installs an extension, or replaces the user's install of it. Returns
    /// the replaced install's digest when no install uses it any more, so
    /// its file can be deleted.
    pub fn save_extension_install(&self, install: &ExtensionInstall) -> Result<Option<String>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT sha256 FROM extension_install WHERE user_id = ?1 AND extension_id = ?2",
                params![install.user_id, install.extension_id],
                |row| row.get(0),
            )
            .optional()?;
        tx.execute(
            "INSERT INTO extension_install
                 (user_id, extension_id, repository_id, name, version, sha256, sites, languages,
                  formats, installed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
             ON CONFLICT (user_id, extension_id) DO UPDATE SET
                 repository_id = excluded.repository_id, name = excluded.name,
                 version = excluded.version, sha256 = excluded.sha256, sites = excluded.sites,
                 languages = excluded.languages, formats = excluded.formats,
                 installed_at = excluded.installed_at",
            params![
                install.user_id,
                install.extension_id,
                install.repository_id,
                install.name,
                install.version,
                install.sha256,
                install.sites,
                install.languages,
                install.formats
            ],
        )?;
        let unused = match previous {
            Some(sha256) if sha256 != install.sha256 && !digest_in_use(&tx, &sha256)? => {
                Some(sha256)
            }
            _ => None,
        };
        tx.commit()?;
        Ok(unused)
    }

    /// Uninstalls an extension. `None` when the user hadn't installed it;
    /// otherwise whether its file is now unused.
    pub fn delete_extension_install(
        &self,
        user_id: i64,
        extension_id: &str,
    ) -> Result<Option<(String, bool)>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let removed: Option<String> = tx
            .query_row(
                "DELETE FROM extension_install WHERE user_id = ?1 AND extension_id = ?2
                 RETURNING sha256",
                params![user_id, extension_id],
                |row| row.get(0),
            )
            .optional()?;
        let result = match removed {
            Some(sha256) => {
                let unused = !digest_in_use(&tx, &sha256)?;
                Some((sha256, unused))
            }
            None => None,
        };
        tx.commit()?;
        Ok(result)
    }

    /// Component digests no install uses, among those given.
    pub fn unused_digests(&self, digests: &[String]) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut unused = Vec::new();
        for digest in digests {
            if !digest_in_use(&conn, digest)? {
                unused.push(digest.clone());
            }
        }
        Ok(unused)
    }

    /// Digests of the components installed from a repository, so their
    /// files can be cleaned up after the repository is removed.
    pub fn repository_digests(&self, repository_id: i64) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare("SELECT DISTINCT sha256 FROM extension_install WHERE repository_id = ?1")?;
        let rows = stmt.query_map([repository_id], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

impl Catalog {
    /// The release group `user_id` prefers for a series of an extension.
    pub fn release_group_preference(
        &self,
        user_id: i64,
        extension_id: &str,
        series_id: &str,
    ) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT release_group FROM release_group_preference
                 WHERE user_id = ?1 AND extension_id = ?2 AND series_id = ?3",
                params![user_id, extension_id, series_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Sets the release group `user_id` prefers for a series, or clears it.
    pub fn set_release_group_preference(
        &self,
        user_id: i64,
        extension_id: &str,
        series_id: &str,
        release_group: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn();
        match release_group {
            Some(group) => conn.execute(
                "INSERT INTO release_group_preference
                     (user_id, extension_id, series_id, release_group, updated_at)
                 VALUES (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
                 ON CONFLICT (user_id, extension_id, series_id) DO UPDATE SET
                     release_group = excluded.release_group, updated_at = excluded.updated_at",
                params![user_id, extension_id, series_id, group],
            )?,
            None => conn.execute(
                "DELETE FROM release_group_preference
                 WHERE user_id = ?1 AND extension_id = ?2 AND series_id = ?3",
                params![user_id, extension_id, series_id],
            )?,
        };
        Ok(())
    }

    /// The content ratings `user_id` chose to see, comma-separated, or
    /// `None` if they never chose.
    pub fn content_rating_preference(&self, user_id: i64) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT ratings FROM content_rating_preference WHERE user_id = ?1",
                [user_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Sets the content ratings `user_id` sees, comma-separated.
    pub fn set_content_rating_preference(&self, user_id: i64, ratings: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO content_rating_preference (user_id, ratings, updated_at)
             VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
             ON CONFLICT (user_id) DO UPDATE SET
                 ratings = excluded.ratings, updated_at = excluded.updated_at",
            params![user_id, ratings],
        )?;
        Ok(())
    }
}

fn digest_in_use(conn: &rusqlite::Connection, sha256: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM extension_install WHERE sha256 = ?1)",
        [sha256],
        |row| row.get(0),
    )
}

fn install_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ExtensionInstall> {
    Ok(ExtensionInstall {
        user_id: row.get(0)?,
        extension_id: row.get(1)?,
        repository_id: row.get(2)?,
        name: row.get(3)?,
        version: row.get(4)?,
        sha256: row.get(5)?,
        sites: row.get(6)?,
        languages: row.get(7)?,
        installed_at: row.get(8)?,
        formats: row.get(9)?,
    })
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
        credentials: row.get(8)?,
    })
}
