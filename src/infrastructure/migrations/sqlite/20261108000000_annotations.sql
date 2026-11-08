-- Annotations (#91): project-declared entries from sanshain.yaml whose values
-- the build fills in. One row per name.
--
-- Version scope: stored with the version-line entry, deleted with it (no
-- foreign key, like spec_version_origin). A real Provide replaces the names it
-- sends; an identical re-provide only fills names not set yet.
CREATE TABLE spec_version_annotation (
    spec_version_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (spec_version_id, name)
);

-- System scope: once per system by name, the latest call naming the system
-- wins per name. Removed with the system's catalog context.
CREATE TABLE system_annotation (
    system_name TEXT NOT NULL,
    name TEXT NOT NULL,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (system_name, name)
);
