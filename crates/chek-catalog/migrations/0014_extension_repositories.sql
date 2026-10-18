-- Where extensions come from. A repository without an owner is on the
-- admin's allow list and open to everyone; one with an owner was added by
-- that user while any repository was allowed, and only they see it.
-- `index` is the repository's last fetched index.json, kept when a later
-- fetch fails.
CREATE TABLE extension_repository (
    id          INTEGER PRIMARY KEY,
    url         TEXT NOT NULL,
    owner_id    INTEGER REFERENCES user (id) ON DELETE CASCADE,
    name        TEXT,
    "index"     TEXT,
    fetched_at  TEXT,
    fetch_error TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE UNIQUE INDEX extension_repository_url
    ON extension_repository (url, coalesce(owner_id, 0));

-- Server-wide settings an admin changes through the API.
CREATE TABLE server_setting (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;

-- The allow list starts with the official repository.
INSERT INTO extension_repository (url, name)
VALUES ('https://chek.page/extensions/index.json', 'Chek');
