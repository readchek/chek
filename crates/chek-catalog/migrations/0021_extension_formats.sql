-- The formats an installed extension's chapters come in, as a JSON array such
-- as `["epub"]`, copied from the component. Installs made before formats
-- existed read only images.
ALTER TABLE extension_install ADD COLUMN formats TEXT NOT NULL DEFAULT '["images"]';
