CREATE TABLE IF NOT EXISTS endpoint_version_metadata (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    endpoint_version_id INTEGER NOT NULL UNIQUE,
    username TEXT,
    source_branch TEXT,
    FOREIGN KEY(endpoint_version_id) REFERENCES endpoint_versions(id) ON DELETE CASCADE
);
