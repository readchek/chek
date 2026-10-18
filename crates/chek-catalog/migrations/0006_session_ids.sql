-- Give each session a plain id, so people can list and end their sessions
-- without the token hash ever leaving the server.

CREATE TABLE session_new (
    id           INTEGER PRIMARY KEY,
    token_hash   TEXT NOT NULL UNIQUE,
    user_id      INTEGER NOT NULL REFERENCES user (id) ON DELETE CASCADE,
    device       TEXT,
    created_at   INTEGER NOT NULL,
    last_used_at INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL
);

INSERT INTO session_new (token_hash, user_id, device, created_at, last_used_at, expires_at)
SELECT token_hash, user_id, device, created_at, last_used_at, expires_at FROM session;

DROP TABLE session;
ALTER TABLE session_new RENAME TO session;
CREATE INDEX session_user ON session (user_id);
