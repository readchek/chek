-- Source items are looked up by their source's id on every sync.
CREATE UNIQUE INDEX content_remote ON content (library_id, remote_id) WHERE kind = 'remote';
