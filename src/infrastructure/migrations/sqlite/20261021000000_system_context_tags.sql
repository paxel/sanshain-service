-- ADR-0010: the Component's `metadata.tags`, kept with its context as a JSON
-- array of lower-cased strings. Existing rows have none.
ALTER TABLE system_context ADD COLUMN tags_json TEXT NOT NULL DEFAULT '[]';
