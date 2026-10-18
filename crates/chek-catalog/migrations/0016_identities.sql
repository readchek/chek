-- Accounts at sign-in providers (OpenID Connect) linked to Chek accounts.
-- `subject` is the provider's stable id for the person, never their email or
-- name, which can change or be reused. A user has at most one account per
-- provider. Accounts made by a provider have an empty password hash, which
-- no password matches.

CREATE TABLE user_identity (
    provider   TEXT NOT NULL,
    subject    TEXT NOT NULL,
    user_id    INTEGER NOT NULL REFERENCES user (id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (provider, subject)
) WITHOUT ROWID;

CREATE UNIQUE INDEX user_identity_user ON user_identity (user_id, provider);
