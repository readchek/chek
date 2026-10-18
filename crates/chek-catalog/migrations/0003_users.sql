-- Accounts, the libraries each reader may open, and sign-in sessions.
-- Admins see every library, so they have no grant rows.

CREATE TABLE user (
    id            INTEGER PRIMARY KEY,
    name          TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash TEXT NOT NULL,
    role          TEXT NOT NULL CHECK (role IN ('admin', 'reader')),
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE TABLE library_grant (
    user_id    INTEGER NOT NULL REFERENCES user (id) ON DELETE CASCADE,
    library_id INTEGER NOT NULL REFERENCES library (id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, library_id)
) WITHOUT ROWID;

-- Only a hash of each token is stored, so a copy of the database can't be
-- used to sign in.
CREATE TABLE session (
    token_hash   TEXT PRIMARY KEY,
    user_id      INTEGER NOT NULL REFERENCES user (id) ON DELETE CASCADE,
    device       TEXT,
    created_at   INTEGER NOT NULL,
    last_used_at INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL
) WITHOUT ROWID;

CREATE INDEX session_user ON session (user_id);
