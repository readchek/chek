-- Files whose metadata names a series are now grouped under it rather than
-- under their folder. Forget file stamps so the next scan regroups them.
UPDATE content SET modified_at = NULL WHERE kind = 'file';
