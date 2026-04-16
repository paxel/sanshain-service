-- Endpoint version history for tracking changes on protected branches
CREATE TABLE IF NOT EXISTS endpoint_versions (
    id BIGSERIAL PRIMARY KEY,
    endpoint_id BIGINT NOT NULL,
    version INTEGER NOT NULL,
    yaml_content TEXT NOT NULL,
    diff_from_previous TEXT,
    created_at TEXT NOT NULL DEFAULT (NOW()::TEXT),
    UNIQUE(endpoint_id, version),
    FOREIGN KEY(endpoint_id) REFERENCES endpoints(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_endpoint_versions_endpoint ON endpoint_versions(endpoint_id);
