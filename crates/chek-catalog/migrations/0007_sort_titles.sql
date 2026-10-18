-- Publications sort within their series by number, then by title compared
-- the way people number things, so "Episode 2" comes before "Episode 10".
-- The key is made by the scan; SQLite can't compare that way itself.
ALTER TABLE publication ADD COLUMN sort_title TEXT NOT NULL DEFAULT '';

-- Forget file stamps so the next scan writes every file's sort keys, and
-- the series' too.
UPDATE content SET modified_at = NULL WHERE kind = 'file';
