-- PDFs now have their title, authors and subject read. Forget file stamps
-- so the next scan reads them.
UPDATE content SET modified_at = NULL WHERE kind = 'file' AND relative_path LIKE '%.pdf';
