//! Accounts, library grants and sessions.

use std::collections::HashSet;

use rusqlite::{OptionalExtension, params};

use crate::{Catalog, CatalogError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Admin,
    Reader,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Reader => "reader",
        }
    }

    fn parse(s: &str) -> Self {
        if s == "admin" {
            Role::Admin
        } else {
            Role::Reader
        }
    }
}

#[derive(Debug, Clone)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub role: Role,
    /// Libraries a reader was granted. Admins see every library, so theirs is empty.
    pub library_ids: Vec<i64>,
    pub created_at: String,
}

impl User {
    pub fn can_read(&self, library_id: i64) -> bool {
        self.role == Role::Admin || self.library_ids.contains(&library_id)
    }
}

#[derive(Debug, Clone)]
pub struct UserWithPassword {
    pub user: User,
    pub password_hash: String,
}

pub struct NewUser<'a> {
    pub name: &'a str,
    pub password_hash: &'a str,
    pub role: Role,
    pub library_ids: &'a [i64],
}

/// Fields to change on a user; `None` leaves a field as it is.
#[derive(Default)]
pub struct UserChanges<'a> {
    pub password_hash: Option<&'a str>,
    pub role: Option<Role>,
    pub library_ids: Option<&'a [i64]>,
}

impl Catalog {
    pub fn user_count(&self) -> Result<i64> {
        Ok(self
            .conn()
            .query_row("SELECT count(*) FROM user", [], |row| row.get(0))?)
    }

    /// Creates a user. With `only_if_none`, it fails with `Ok(None)` when any
    /// user already exists, so first-admin setup can't race a second request.
    pub fn create_user(&self, new: &NewUser<'_>, only_if_none: bool) -> Result<Option<User>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if only_if_none
            && tx.query_row("SELECT count(*) FROM user", [], |row| row.get::<_, i64>(0))? > 0
        {
            return Ok(None);
        }
        let inserted = tx.execute(
            "INSERT INTO user (name, password_hash, role) VALUES (?1, ?2, ?3) ON CONFLICT (name) DO NOTHING",
            params![new.name, new.password_hash, new.role.as_str()],
        )?;
        if inserted == 0 {
            return Err(CatalogError::NameTaken(new.name.to_owned()));
        }
        let id = tx.last_insert_rowid();
        set_grants(&tx, id, new.role, new.library_ids)?;
        tx.commit()?;
        drop(conn);
        self.user(id)
    }

    pub fn user(&self, id: i64) -> Result<Option<User>> {
        let conn = self.conn();
        let user = conn
            .query_row(
                "SELECT id, name, role, created_at FROM user WHERE id = ?1",
                [id],
                |row| {
                    Ok(User {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        role: Role::parse(row.get_ref(2)?.as_str()?),
                        library_ids: Vec::new(),
                        created_at: row.get(3)?,
                    })
                },
            )
            .optional()?;
        let Some(mut user) = user else {
            return Ok(None);
        };
        user.library_ids = grants(&conn, id)?;
        Ok(Some(user))
    }

    pub fn user_by_name(&self, name: &str) -> Result<Option<UserWithPassword>> {
        let found: Option<(i64, String)> = self
            .conn()
            .query_row(
                "SELECT id, password_hash FROM user WHERE name = ?1",
                [name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((id, password_hash)) = found else {
            return Ok(None);
        };
        Ok(self.user(id)?.map(|user| UserWithPassword {
            user,
            password_hash,
        }))
    }

    pub fn password_hash(&self, user_id: i64) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT password_hash FROM user WHERE id = ?1",
                [user_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn users(&self) -> Result<Vec<User>> {
        let ids: Vec<i64> = {
            let conn = self.conn();
            let mut stmt = conn.prepare("SELECT id FROM user ORDER BY id")?;
            stmt.query_map([], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        let mut users = Vec::with_capacity(ids.len());
        for id in ids {
            users.extend(self.user(id)?);
        }
        Ok(users)
    }

    /// Applies changes to a user. Returns `Ok(None)` when the user doesn't
    /// exist, and refuses to demote the last admin.
    pub fn update_user(&self, id: i64, changes: &UserChanges<'_>) -> Result<Option<UserUpdate>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let current: Option<String> = tx
            .query_row("SELECT role FROM user WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?;
        let Some(current) = current else {
            return Ok(None);
        };
        let role = changes.role.unwrap_or(Role::parse(&current));
        if Role::parse(&current) == Role::Admin && role != Role::Admin && admin_count(&tx)? <= 1 {
            return Ok(Some(UserUpdate::LastAdmin));
        }
        tx.execute(
            "UPDATE user SET role = ?2 WHERE id = ?1",
            params![id, role.as_str()],
        )?;
        if let Some(hash) = changes.password_hash {
            tx.execute(
                "UPDATE user SET password_hash = ?2 WHERE id = ?1",
                params![id, hash],
            )?;
        }
        if let Some(library_ids) = changes.library_ids {
            set_grants(&tx, id, role, library_ids)?;
        } else if role == Role::Admin {
            set_grants(&tx, id, role, &[])?;
        }
        tx.commit()?;
        drop(conn);
        Ok(self.user(id)?.map(UserUpdate::Updated))
    }

    /// Deletes a user and their sessions. Refuses to delete the last admin.
    pub fn delete_user(&self, id: i64) -> Result<Option<UserUpdate>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let role: Option<String> = tx
            .query_row("SELECT role FROM user WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?;
        let Some(role) = role else {
            return Ok(None);
        };
        if Role::parse(&role) == Role::Admin && admin_count(&tx)? <= 1 {
            return Ok(Some(UserUpdate::LastAdmin));
        }
        tx.execute("DELETE FROM user WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(Some(UserUpdate::Deleted))
    }

    pub fn create_session(
        &self,
        token_hash: &str,
        user_id: i64,
        device: Option<&str>,
        now: i64,
        expires_at: i64,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO session (token_hash, user_id, device, created_at, last_used_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
            params![token_hash, user_id, device, now, expires_at],
        )?;
        Ok(())
    }

    /// The user a session token belongs to, if the session hasn't expired.
    /// A session used after `refresh_after` seconds is extended to `expires_at`,
    /// so people who keep reading stay signed in.
    pub fn session_user(
        &self,
        token_hash: &str,
        now: i64,
        refresh_after: i64,
        expires_at: i64,
    ) -> Result<Option<User>> {
        let session: Option<(i64, i64)> = {
            let conn = self.conn();
            let session = conn
                .query_row(
                    "SELECT user_id, last_used_at FROM session WHERE token_hash = ?1 AND expires_at > ?2",
                    params![token_hash, now],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if let Some((_, last_used_at)) = session
                && now - last_used_at >= refresh_after
            {
                conn.execute(
                    "UPDATE session SET last_used_at = ?2, expires_at = ?3 WHERE token_hash = ?1",
                    params![token_hash, now, expires_at],
                )?;
            }
            session
        };
        match session {
            Some((user_id, _)) => self.user(user_id),
            None => Ok(None),
        }
    }

    pub fn delete_session(&self, token_hash: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM session WHERE token_hash = ?1", [token_hash])?;
        Ok(())
    }

    /// Signs a user out everywhere except, optionally, one session.
    pub fn delete_user_sessions(&self, user_id: i64, keep_token_hash: Option<&str>) -> Result<()> {
        self.conn().execute(
            "DELETE FROM session WHERE user_id = ?1 AND token_hash IS NOT ?2",
            params![user_id, keep_token_hash],
        )?;
        Ok(())
    }

    pub fn delete_expired_sessions(&self, now: i64) -> Result<usize> {
        Ok(self
            .conn()
            .execute("DELETE FROM session WHERE expires_at <= ?1", [now])?)
    }

    /// Library ids that exist, out of `ids`.
    pub fn existing_libraries(&self, ids: &[i64]) -> Result<HashSet<i64>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT 1 FROM library WHERE id = ?1")?;
        let mut found = HashSet::new();
        for &id in ids {
            if stmt.exists([id])? {
                found.insert(id);
            }
        }
        Ok(found)
    }
}

pub enum UserUpdate {
    Updated(User),
    Deleted,
    /// The change would leave the server without an admin.
    LastAdmin,
}

fn admin_count(conn: &rusqlite::Connection) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT count(*) FROM user WHERE role = 'admin'",
        [],
        |row| row.get(0),
    )?)
}

fn grants(conn: &rusqlite::Connection, user_id: i64) -> Result<Vec<i64>> {
    let mut stmt = conn
        .prepare("SELECT library_id FROM library_grant WHERE user_id = ?1 ORDER BY library_id")?;
    let ids = stmt.query_map([user_id], |row| row.get(0))?;
    Ok(ids.collect::<rusqlite::Result<_>>()?)
}

fn set_grants(
    conn: &rusqlite::Connection,
    user_id: i64,
    role: Role,
    library_ids: &[i64],
) -> Result<()> {
    conn.execute("DELETE FROM library_grant WHERE user_id = ?1", [user_id])?;
    if role == Role::Reader {
        let mut insert = conn
            .prepare("INSERT OR IGNORE INTO library_grant (user_id, library_id) VALUES (?1, ?2)")?;
        for id in library_ids {
            insert.execute(params![user_id, id])?;
        }
    }
    Ok(())
}
