//! What extensions keep between calls: a cache of values and responses, and
//! cookies. Each extension has its own, keyed by its id and sites, so
//! versions share them and users who install the same extension share them
//! too.
//!
//! Everything lives in one SQLite file next to the components. Entries
//! expire, and an extension's entries are capped in total size, so a busy
//! source can't fill the disk.

use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

/// The largest value one entry may hold.
pub(crate) const MAX_ENTRY: usize = 4 << 20;
/// The most an extension's entries may hold together. The oldest are
/// dropped to make room.
const MAX_PER_EXTENSION: i64 = 128 << 20;
/// How long an entry may live.
pub(crate) const MAX_TTL: u32 = 30 * 24 * 60 * 60;

/// Which kind of entry a key names, so an extension's own keys can't
/// collide with the responses the host caches for it.
#[derive(Clone, Copy)]
pub(crate) enum Namespace {
    /// Values the extension set through the `cache` interface.
    Value,
    /// Responses to requests the extension marked cacheable.
    Response,
    /// What the browser solver earned, such as its user agent.
    Browser,
}

impl Namespace {
    fn as_str(self) -> &'static str {
        match self {
            Namespace::Value => "value",
            Namespace::Response => "response",
            Namespace::Browser => "browser",
        }
    }
}

pub(crate) struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS entry (
                 extension_id TEXT NOT NULL,
                 namespace    TEXT NOT NULL,
                 key          TEXT NOT NULL,
                 meta         TEXT,
                 value        BLOB NOT NULL,
                 size         INTEGER NOT NULL,
                 stored_at    INTEGER NOT NULL,
                 expires_at   INTEGER NOT NULL,
                 PRIMARY KEY (extension_id, namespace, key)
             );
             CREATE INDEX IF NOT EXISTS entry_age ON entry (extension_id, stored_at);
             CREATE TABLE IF NOT EXISTS cookie_jar (
                 extension_id TEXT PRIMARY KEY,
                 jar          TEXT NOT NULL
             );",
        )
        .map_err(|e| e.to_string())?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        // A panic while holding the lock leaves SQLite consistent, so the
        // connection stays usable.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// An unexpired entry's metadata and value.
    pub(crate) fn get(
        &self,
        extension_id: &str,
        namespace: Namespace,
        key: &str,
    ) -> Option<(Option<String>, Vec<u8>)> {
        self.conn()
            .query_row(
                "SELECT meta, value FROM entry
                 WHERE extension_id = ?1 AND namespace = ?2 AND key = ?3 AND expires_at > ?4",
                params![extension_id, namespace.as_str(), key, now()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .ok()
            .flatten()
    }

    /// Stores an entry for `ttl` seconds, then drops expired entries and, if
    /// the extension is over its share, its oldest ones.
    pub(crate) fn set(
        &self,
        extension_id: &str,
        namespace: Namespace,
        key: &str,
        meta: Option<&str>,
        value: &[u8],
        ttl: u32,
    ) -> Result<(), String> {
        if value.len() > MAX_ENTRY {
            return Err(format!("values are limited to {MAX_ENTRY} bytes"));
        }
        let now = now();
        let size = (value.len() + key.len() + meta.map_or(0, str::len)) as i64;
        let conn = self.conn();
        conn.execute(
            "INSERT OR REPLACE INTO entry
                 (extension_id, namespace, key, meta, value, size, stored_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                extension_id,
                namespace.as_str(),
                key,
                meta,
                value,
                size,
                now,
                now + i64::from(ttl.min(MAX_TTL))
            ],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM entry WHERE extension_id = ?1 AND expires_at <= ?2",
            params![extension_id, now],
        )
        .map_err(|e| e.to_string())?;
        // Drops the oldest entries past the share, keeping the newest that fit.
        conn.execute(
            "DELETE FROM entry WHERE extension_id = ?1 AND rowid IN (
                 SELECT rowid FROM (
                     SELECT rowid, SUM(size) OVER (ORDER BY stored_at DESC, rowid DESC) AS total
                     FROM entry WHERE extension_id = ?1
                 ) WHERE total > ?2
             )",
            params![extension_id, MAX_PER_EXTENSION],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub(crate) fn delete(&self, extension_id: &str, namespace: Namespace, key: &str) {
        let _ = self.conn().execute(
            "DELETE FROM entry WHERE extension_id = ?1 AND namespace = ?2 AND key = ?3",
            params![extension_id, namespace.as_str(), key],
        );
    }

    /// The extension's saved cookie jar, as `cookie_store` JSON.
    pub(crate) fn cookie_jar(&self, extension_id: &str) -> Option<String> {
        self.conn()
            .query_row(
                "SELECT jar FROM cookie_jar WHERE extension_id = ?1",
                [extension_id],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    pub(crate) fn save_cookie_jar(&self, extension_id: &str, jar: &str) -> Result<(), String> {
        self.conn()
            .execute(
                "INSERT OR REPLACE INTO cookie_jar (extension_id, jar) VALUES (?1, ?2)",
                params![extension_id, jar],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
