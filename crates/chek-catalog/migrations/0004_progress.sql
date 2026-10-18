-- Where each user is in each publication. `locator` is a Readium Locator as
-- JSON. `revision` counts saves, so a device can save only on top of the
-- position it last saw.

CREATE TABLE progress (
    user_id        INTEGER NOT NULL REFERENCES user (id) ON DELETE CASCADE,
    publication_id INTEGER NOT NULL REFERENCES publication (id) ON DELETE CASCADE,
    locator        TEXT NOT NULL,
    revision       INTEGER NOT NULL,
    device         TEXT,
    updated_at     TEXT NOT NULL,
    PRIMARY KEY (user_id, publication_id)
) WITHOUT ROWID;
