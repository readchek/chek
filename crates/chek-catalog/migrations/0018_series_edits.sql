-- A series title an admin set by hand. Scans derive a series' title from its
-- folder or the files' metadata and write it on every pass, so the edit is
-- kept apart and written over the scanned title afterwards. The scanned
-- title is kept too, so clearing the edit brings it back at once.
CREATE TABLE series_edit (
    series_id          INTEGER PRIMARY KEY REFERENCES series (id) ON DELETE CASCADE,
    title              TEXT NOT NULL,
    sort_title         TEXT NOT NULL,
    scanned_title      TEXT NOT NULL,
    scanned_sort_title TEXT NOT NULL
);
