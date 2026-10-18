-- Metadata an admin changed by hand. Each scan of the file writes what the
-- file says and then these on top, so edits survive rescans. A `_set` flag
-- marks a field as edited, since an edit may clear a field to null.
CREATE TABLE publication_edit (
    publication_id    INTEGER PRIMARY KEY REFERENCES publication (id) ON DELETE CASCADE,
    title             TEXT,
    sort_title        TEXT,
    number_set        INTEGER NOT NULL DEFAULT 0,
    number_label      TEXT,
    sort_number       REAL,
    summary_set       INTEGER NOT NULL DEFAULT 0,
    summary           TEXT,
    publisher_set     INTEGER NOT NULL DEFAULT 0,
    publisher         TEXT,
    language_set      INTEGER NOT NULL DEFAULT 0,
    language          TEXT,
    published_set     INTEGER NOT NULL DEFAULT 0,
    published         TEXT,
    direction_set     INTEGER NOT NULL DEFAULT 0,
    reading_direction TEXT CHECK (reading_direction IN ('ltr', 'rtl')),
    contributors_set  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE publication_edit_contributor (
    publication_id INTEGER NOT NULL REFERENCES publication_edit (publication_id) ON DELETE CASCADE,
    position       INTEGER NOT NULL,
    name           TEXT NOT NULL,
    role           TEXT NOT NULL,
    PRIMARY KEY (publication_id, position)
) WITHOUT ROWID;
