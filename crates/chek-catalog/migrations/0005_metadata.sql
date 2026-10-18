-- Metadata read from each file: the EPUB package document or ComicInfo.xml.
-- `title` and `number_label` already exist and now come from it when present.

ALTER TABLE publication ADD COLUMN summary TEXT;
ALTER TABLE publication ADD COLUMN publisher TEXT;
ALTER TABLE publication ADD COLUMN language TEXT;
ALTER TABLE publication ADD COLUMN published TEXT;
ALTER TABLE publication ADD COLUMN reading_direction TEXT
    CHECK (reading_direction IN ('ltr', 'rtl'));

-- People credited on a publication, in the order the file lists them.
CREATE TABLE contributor (
    publication_id INTEGER NOT NULL REFERENCES publication (id) ON DELETE CASCADE,
    position       INTEGER NOT NULL,
    name           TEXT NOT NULL,
    role           TEXT NOT NULL,
    PRIMARY KEY (publication_id, position)
) WITHOUT ROWID;

-- Forget file stamps so the next scan reads every file's metadata.
UPDATE content SET modified_at = NULL WHERE kind = 'file';
