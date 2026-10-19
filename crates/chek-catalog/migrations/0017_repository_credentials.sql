-- How to sign in to a private extension repository, as the server's JSON.
-- Like a Komga library's settings, it never leaves the server.
ALTER TABLE extension_repository ADD COLUMN credentials TEXT;
