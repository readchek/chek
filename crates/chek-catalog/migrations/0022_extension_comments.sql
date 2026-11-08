-- The orders an installed extension lists readers' comments in, as a JSON
-- array such as `["top","newest"]`, copied from the component. `[]` means it
-- gives only a count, and NULL that it gives no comments.
ALTER TABLE extension_install ADD COLUMN comments TEXT;
