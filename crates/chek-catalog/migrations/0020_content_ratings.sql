-- The content ratings each user chose to see when browsing sources, as a
-- comma-separated list such as `safe,suggestive`. A user without a row sees
-- the default, so it can change without touching every account.
CREATE TABLE content_rating_preference (
    user_id    INTEGER PRIMARY KEY REFERENCES user (id) ON DELETE CASCADE,
    ratings    TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
