-- A publication's cover thumbnail, stored in the data folder under
-- covers/<cover>.jpg. NULL when the publication has no cover.
ALTER TABLE publication ADD COLUMN cover TEXT;

-- Forget file stamps so the next scan reads every file again and makes covers
-- for publications scanned before covers existed.
UPDATE content SET modified_at = NULL WHERE kind = 'file';
