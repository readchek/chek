//! Accounts at sign-in providers linked to Chek accounts.

use rusqlite::{OptionalExtension, params};

use crate::{Catalog, CatalogError, Result, Role, User};

/// A provider account linked to a user.
#[derive(Debug, Clone)]
pub struct Identity {
    pub provider: String,
    pub created_at: String,
}

pub enum LinkOutcome {
    Linked,
    /// The provider account is already linked to another user.
    TakenByOther,
    /// The user already has a different account at this provider.
    ProviderLinked,
}

pub enum UnlinkOutcome {
    Unlinked,
    NotLinked,
    /// The user has no password and no other provider, so they couldn't
    /// sign in again.
    LastSignIn,
}

/// Tried after a name that's taken: "Name 2", "Name 3" and so on.
const MAX_NAME_TRIES: u32 = 100;

impl Catalog {
    /// The user a provider account is linked to.
    pub fn identity_user(&self, provider: &str, subject: &str) -> Result<Option<User>> {
        let user_id: Option<i64> = self
            .conn()
            .query_row(
                "SELECT user_id FROM user_identity WHERE provider = ?1 AND subject = ?2",
                params![provider, subject],
                |row| row.get(0),
            )
            .optional()?;
        match user_id {
            Some(id) => self.user(id),
            None => Ok(None),
        }
    }

    /// Creates a user with no password, linked to a provider account. The
    /// name is the first of `name`, "name 2", "name 3" and so on that's free;
    /// `max_chars` bounds the whole name.
    pub fn create_identity_user(
        &self,
        name: &str,
        max_chars: usize,
        role: Role,
        provider: &str,
        subject: &str,
    ) -> Result<User> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut user_id = None;
        for n in 1..=MAX_NAME_TRIES {
            let candidate = if n == 1 {
                name.chars().take(max_chars).collect::<String>()
            } else {
                let suffix = format!(" {n}");
                let keep = max_chars.saturating_sub(suffix.chars().count());
                let base: String = name.chars().take(keep).collect();
                format!("{}{suffix}", base.trim_end())
            };
            let inserted = tx.execute(
                "INSERT INTO user (name, password_hash, role) VALUES (?1, '', ?2) ON CONFLICT (name) DO NOTHING",
                params![candidate, role.as_str()],
            )?;
            if inserted == 1 {
                user_id = Some(tx.last_insert_rowid());
                break;
            }
        }
        let user_id = user_id.ok_or_else(|| CatalogError::NameTaken(name.to_owned()))?;
        tx.execute(
            "INSERT INTO user_identity (provider, subject, user_id) VALUES (?1, ?2, ?3)",
            params![provider, subject, user_id],
        )?;
        tx.commit()?;
        drop(conn);
        self.user(user_id)?
            .ok_or_else(|| CatalogError::Sqlite(rusqlite::Error::QueryReturnedNoRows))
    }

    /// Links a provider account to a user. Linking the same account again
    /// succeeds.
    pub fn link_identity(
        &self,
        user_id: i64,
        provider: &str,
        subject: &str,
    ) -> Result<LinkOutcome> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let owner: Option<i64> = tx
            .query_row(
                "SELECT user_id FROM user_identity WHERE provider = ?1 AND subject = ?2",
                params![provider, subject],
                |row| row.get(0),
            )
            .optional()?;
        match owner {
            Some(owner) if owner == user_id => return Ok(LinkOutcome::Linked),
            Some(_) => return Ok(LinkOutcome::TakenByOther),
            None => {}
        }
        let has_provider = tx
            .prepare("SELECT 1 FROM user_identity WHERE user_id = ?1 AND provider = ?2")?
            .exists(params![user_id, provider])?;
        if has_provider {
            return Ok(LinkOutcome::ProviderLinked);
        }
        tx.execute(
            "INSERT INTO user_identity (provider, subject, user_id) VALUES (?1, ?2, ?3)",
            params![provider, subject, user_id],
        )?;
        tx.commit()?;
        Ok(LinkOutcome::Linked)
    }

    /// A user's linked providers, oldest first.
    pub fn identities(&self, user_id: i64) -> Result<Vec<Identity>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT provider, created_at FROM user_identity WHERE user_id = ?1 ORDER BY created_at, provider",
        )?;
        let rows = stmt.query_map([user_id], |row| {
            Ok(Identity {
                provider: row.get(0)?,
                created_at: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Unlinks a provider from a user, unless it's the only way they can
    /// sign in.
    pub fn unlink_identity(&self, user_id: i64, provider: &str) -> Result<UnlinkOutcome> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let linked = tx
            .prepare("SELECT 1 FROM user_identity WHERE user_id = ?1 AND provider = ?2")?
            .exists(params![user_id, provider])?;
        if !linked {
            return Ok(UnlinkOutcome::NotLinked);
        }
        let has_password: bool = tx.query_row(
            "SELECT password_hash != '' FROM user WHERE id = ?1",
            [user_id],
            |row| row.get(0),
        )?;
        let others: i64 = tx.query_row(
            "SELECT count(*) FROM user_identity WHERE user_id = ?1 AND provider != ?2",
            params![user_id, provider],
            |row| row.get(0),
        )?;
        if !has_password && others == 0 {
            return Ok(UnlinkOutcome::LastSignIn);
        }
        tx.execute(
            "DELETE FROM user_identity WHERE user_id = ?1 AND provider = ?2",
            params![user_id, provider],
        )?;
        tx.commit()?;
        Ok(UnlinkOutcome::Unlinked)
    }
}
