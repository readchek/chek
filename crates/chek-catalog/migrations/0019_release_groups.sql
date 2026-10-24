-- The release group each user prefers for a series they browse through an
-- extension. Sites list a chapter once per group that released it, and the
-- server shows one chapter with this group's release picked when it has
-- one. Kept apart from installs so reinstalling an extension keeps it.
CREATE TABLE release_group_preference (
    user_id       INTEGER NOT NULL REFERENCES user (id) ON DELETE CASCADE,
    extension_id  TEXT NOT NULL,
    series_id     TEXT NOT NULL,
    release_group TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    PRIMARY KEY (user_id, extension_id, series_id)
) WITHOUT ROWID;
