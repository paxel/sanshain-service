-- Add api_type to endpoints and dependencies, and update unique constraints.
PRAGMA foreign_keys=OFF;
PRAGMA defer_foreign_keys=ON;

-- 1. Endpoints table update
CREATE TABLE endpoints_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    branch_id INTEGER NOT NULL,
    api_type TEXT NOT NULL DEFAULT 'openapi',
    path TEXT NOT NULL,
    normalized_path TEXT NOT NULL,
    method TEXT NOT NULL,
    yaml_content TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    UNIQUE(branch_id, api_type, path, method),
    FOREIGN KEY(branch_id) REFERENCES branches(id)
);

-- Copy data from old endpoints table
INSERT INTO endpoints_new (id, branch_id, path, normalized_path, method, yaml_content, deleted)
SELECT id, branch_id, path, normalized_path, method, yaml_content, deleted FROM endpoints;

-- 2. Copy dependencies to a temporary table while the old schema still exists.
CREATE TEMPORARY TABLE dependencies_old AS
SELECT id, client_id, endpoint_id, requested_service_id, requested_branch_name, requested_path, requested_normalized_path, requested_method, last_seen_at FROM dependencies;

-- Drop dependencies before endpoints because SQLite keeps foreign key checks
-- enabled inside transactional migrations even after PRAGMA foreign_keys=OFF;
-- defer_foreign_keys postpones validation until both rebuilt tables are back.
DROP TABLE dependencies;
DROP TABLE endpoints;
ALTER TABLE endpoints_new RENAME TO endpoints;
CREATE INDEX IF NOT EXISTS idx_endpoints_normalized_path ON endpoints(normalized_path);

-- 3. Dependencies table update
CREATE TABLE dependencies_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_id INTEGER NOT NULL,
    endpoint_id INTEGER,
    api_type TEXT NOT NULL DEFAULT 'openapi',
    requested_service_id INTEGER NOT NULL,
    requested_branch_name TEXT NOT NULL,
    requested_path TEXT NOT NULL,
    requested_normalized_path TEXT NOT NULL,
    requested_method TEXT NOT NULL,
    last_seen_at TEXT NOT NULL DEFAULT '1970-01-01T00:00:00',
    UNIQUE(client_id, endpoint_id, requested_service_id, requested_branch_name, api_type, requested_path, requested_method),
    FOREIGN KEY(client_id) REFERENCES clients(id),
    FOREIGN KEY(endpoint_id) REFERENCES endpoints(id),
    FOREIGN KEY(requested_service_id) REFERENCES services(id)
);

-- Copy data from old dependencies table
INSERT INTO dependencies_new (id, client_id, endpoint_id, requested_service_id, requested_branch_name, requested_path, requested_normalized_path, requested_method, last_seen_at)
SELECT id, client_id, endpoint_id, requested_service_id, requested_branch_name, requested_path, requested_normalized_path, requested_method, last_seen_at FROM dependencies_old;
DROP TABLE dependencies_old;

ALTER TABLE dependencies_new RENAME TO dependencies;
CREATE INDEX IF NOT EXISTS idx_dependencies_requested_normalized_path ON dependencies(requested_normalized_path);

PRAGMA foreign_keys=ON;
