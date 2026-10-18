-- Extensions each user installed. The component is stored in the data folder
-- as extensions/<sha256>.wasm, shared by everyone on the same version.
-- `sites` and `languages` are JSON arrays copied from the manifest. Removing
-- the repository removes its installs, so dropping one from the allow list
-- stops its extensions running.
CREATE TABLE extension_install (
    user_id       INTEGER NOT NULL REFERENCES user (id) ON DELETE CASCADE,
    extension_id  TEXT NOT NULL,
    repository_id INTEGER NOT NULL REFERENCES extension_repository (id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    version       TEXT NOT NULL,
    sha256        TEXT NOT NULL,
    sites         TEXT NOT NULL,
    languages     TEXT NOT NULL,
    installed_at  TEXT NOT NULL,
    PRIMARY KEY (user_id, extension_id)
) WITHOUT ROWID;

CREATE INDEX extension_install_sha256 ON extension_install (sha256);
