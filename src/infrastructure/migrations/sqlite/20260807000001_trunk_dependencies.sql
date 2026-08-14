-- ADR-0004/0005: the trunk pin set, append-only. The open record
-- (valid_to IS NULL) per (client, service, api_type, normalized_path, method)
-- is the current trunk pin; a re-pin to a different version closes it and
-- inserts a new one. Rows are never overwritten or deleted — closed records
-- are the timeline ADR-0005 reconstructs. The pinned version is stored BY
-- VALUE (never a spec_versions id): a deleted version leaves a visibly
-- dangling reference that heals when the number is re-provided.
CREATE TABLE trunk_dependencies (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_id INTEGER NOT NULL,
    service_id INTEGER NOT NULL,
    api_type TEXT NOT NULL,
    path TEXT NOT NULL,
    normalized_path TEXT NOT NULL,
    method TEXT NOT NULL,
    major INTEGER NOT NULL,
    minor INTEGER NOT NULL,
    patch INTEGER NOT NULL,
    valid_from TEXT NOT NULL,
    last_required_at TEXT NOT NULL,
    valid_to TEXT NULL,
    FOREIGN KEY (client_id) REFERENCES clients(id) ON DELETE CASCADE,
    FOREIGN KEY (service_id) REFERENCES services(id) ON DELETE CASCADE
);
CREATE INDEX idx_trunk_dependencies_open
    ON trunk_dependencies(client_id, service_id, api_type, normalized_path, method)
    WHERE valid_to IS NULL;
CREATE INDEX idx_trunk_dependencies_service ON trunk_dependencies(service_id);
