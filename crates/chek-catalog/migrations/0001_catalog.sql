-- Libraries hold series, series hold publications, and each publication is
-- backed by one content row: a local file now, a remote source item later.
-- Users and progress arrive with the sign-in slice.

CREATE TABLE library (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    kind        TEXT NOT NULL CHECK (kind IN ('folder', 'source')),
    root_path   TEXT,
    source_config TEXT,
    scan_status TEXT NOT NULL DEFAULT 'idle' CHECK (scan_status IN ('idle', 'scanning', 'failed')),
    scan_error  TEXT,
    scanned_at  TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    CHECK ((kind = 'folder') = (root_path IS NOT NULL))
);

CREATE TABLE series (
    id                INTEGER PRIMARY KEY,
    library_id        INTEGER NOT NULL REFERENCES library (id) ON DELETE CASCADE,
    title             TEXT NOT NULL,
    sort_title        TEXT NOT NULL,
    reading_direction TEXT CHECK (reading_direction IN ('ltr', 'rtl', 'ttb')),
    -- What the scan groups by: a folder's relative path, or a file's for
    -- files at the library root. Never the title, so same-named series stay apart.
    path_key          TEXT,
    remote_id         TEXT,
    UNIQUE (library_id, path_key),
    UNIQUE (library_id, remote_id)
);

CREATE TABLE publication (
    id           INTEGER PRIMARY KEY,
    series_id    INTEGER NOT NULL REFERENCES series (id) ON DELETE CASCADE,
    title        TEXT NOT NULL,
    number_label TEXT,
    sort_number  REAL,
    format       TEXT NOT NULL,
    page_count   INTEGER,
    available    INTEGER NOT NULL DEFAULT 1 CHECK (available IN (0, 1))
);

CREATE INDEX publication_series ON publication (series_id);

CREATE TABLE content (
    id             INTEGER PRIMARY KEY,
    publication_id INTEGER NOT NULL UNIQUE REFERENCES publication (id) ON DELETE CASCADE,
    library_id     INTEGER NOT NULL REFERENCES library (id) ON DELETE CASCADE,
    kind           TEXT NOT NULL CHECK (kind IN ('file', 'remote')),
    relative_path  TEXT,
    size           INTEGER,
    hash           TEXT,
    modified_at    INTEGER,
    remote_id      TEXT,
    remote_version TEXT,
    UNIQUE (library_id, relative_path),
    CHECK ((kind = 'file') = (relative_path IS NOT NULL))
);

CREATE TABLE page (
    content_id      INTEGER NOT NULL REFERENCES content (id) ON DELETE CASCADE,
    page_index      INTEGER NOT NULL,
    entry_name      TEXT,
    remote_key      TEXT,
    data_offset     INTEGER,
    compressed_size INTEGER,
    size            INTEGER,
    compression     TEXT,
    width           INTEGER,
    height          INTEGER,
    PRIMARY KEY (content_id, page_index)
) WITHOUT ROWID;
