-- Source libraries hold series followed through an extension. Each chapter
-- is a publication backed by a remote content row, keyed by the source's
-- chapter id; its pages are stored as the image requests the extension
-- returned, in `page.remote_key`, and refetched when the version changes.
CREATE UNIQUE INDEX content_remote ON content (library_id, remote_id) WHERE kind = 'remote';
