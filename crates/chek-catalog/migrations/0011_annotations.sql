-- Bookmarks and highlights each user keeps in each publication. `locator` is
-- a Readium Locator as JSON; a highlight's selected text is in its
-- `text.highlight`.
CREATE TABLE annotation (
    id             INTEGER PRIMARY KEY,
    user_id        INTEGER NOT NULL REFERENCES user (id) ON DELETE CASCADE,
    publication_id INTEGER NOT NULL REFERENCES publication (id) ON DELETE CASCADE,
    kind           TEXT NOT NULL CHECK (kind IN ('bookmark', 'highlight')),
    locator        TEXT NOT NULL,
    color          TEXT,
    note           TEXT,
    device         TEXT,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);

CREATE INDEX annotation_user_publication ON annotation (user_id, publication_id);
