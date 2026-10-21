-- The origin of a version: what the build that provided it said about why.
-- One row per version-line entry, replaced by the latest Provide that
-- carried an origin; the audit log keeps the earlier ones. Apart from the
-- entry's own columns so the version rows stay what they were.
--
-- `tickets_json` is a JSON array of ticket ids, at most 20.
CREATE TABLE spec_version_origin (
    spec_version_id INTEGER PRIMARY KEY,
    build_url TEXT NULL,
    change_url TEXT NULL,
    tickets_json TEXT NOT NULL,
    note TEXT NULL,
    updated_at TEXT NOT NULL
);
