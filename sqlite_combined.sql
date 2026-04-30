-- Core tables
CREATE TABLE IF NOT EXISTS services (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE
);
CREATE TABLE IF NOT EXISTS branches (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    service_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    UNIQUE(service_id, name),
    FOREIGN KEY(service_id) REFERENCES services(id)
);
CREATE TABLE IF NOT EXISTS endpoints (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    branch_id INTEGER NOT NULL,
    path TEXT NOT NULL,
    method TEXT NOT NULL,
    yaml_content TEXT NOT NULL,
    UNIQUE(branch_id, path, method),
    FOREIGN KEY(branch_id) REFERENCES branches(id)
);
CREATE TABLE IF NOT EXISTS clients (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE
);
CREATE TABLE IF NOT EXISTS dependencies (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_id INTEGER NOT NULL,
    endpoint_id INTEGER,
    requested_service_id INTEGER NOT NULL,
    requested_branch_name TEXT NOT NULL,
    requested_path TEXT NOT NULL,
    requested_method TEXT NOT NULL,
    UNIQUE(client_id, endpoint_id, requested_service_id, requested_branch_name, requested_path, requested_method),
    FOREIGN KEY(client_id) REFERENCES clients(id),
    FOREIGN KEY(endpoint_id) REFERENCES endpoints(id),
    FOREIGN KEY(requested_service_id) REFERENCES services(id)
);

-- Protected branches
CREATE TABLE IF NOT EXISTS protected_branches (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    pattern TEXT NOT NULL UNIQUE
);
INSERT OR IGNORE INTO protected_branches (pattern) VALUES ('main');
INSERT OR IGNORE INTO protected_branches (pattern) VALUES ('master');

-- Auth: users, sessions, settings
CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    is_admin BOOLEAN NOT NULL DEFAULT FALSE,
    approved BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE IF NOT EXISTS sessions (
    token TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sessions_expires ON sessions(expires_at);
CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
INSERT OR IGNORE INTO settings (key, value) VALUES ('dev_mode', 'false');
INSERT OR IGNORE INTO settings (key, value) VALUES ('local_users_enabled', 'false');

-- API tokens for programmatic access
CREATE TABLE IF NOT EXISTS api_tokens (
    id TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    last_used_at TEXT,
    UNIQUE(user_id, name)
);
CREATE INDEX IF NOT EXISTS idx_api_tokens_user ON api_tokens(user_id);
CREATE INDEX IF NOT EXISTS idx_api_tokens_hash ON api_tokens(token_hash);
-- Add updated_at column to branches for max-age auto-cleanup
ALTER TABLE branches ADD COLUMN updated_at TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z';

-- Set existing branches to current time so they don't get immediately cleaned up
UPDATE branches SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now');

-- Default branch max-age: 30 days (in seconds)
INSERT OR IGNORE INTO settings (key, value) VALUES ('branch_max_age_days', '30');
-- Add soft-delete support for endpoints.
-- On protected branches, removed endpoints are marked deleted rather than hard-deleted,
-- so that re-introducing them later is detected as a contract violation.
ALTER TABLE endpoints ADD COLUMN deleted BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE dependencies ADD COLUMN last_seen_at TEXT NOT NULL DEFAULT '1970-01-01T00:00:00';
-- Endpoint version history for tracking changes on protected branches
CREATE TABLE IF NOT EXISTS endpoint_versions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    endpoint_id INTEGER NOT NULL,
    version INTEGER NOT NULL,
    yaml_content TEXT NOT NULL,
    diff_from_previous TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(endpoint_id, version),
    FOREIGN KEY(endpoint_id) REFERENCES endpoints(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_endpoint_versions_endpoint ON endpoint_versions(endpoint_id);
-- Add fallback_branch column to services
ALTER TABLE services ADD COLUMN fallback_branch TEXT;
-- Add normalized_path to endpoints table
ALTER TABLE endpoints ADD COLUMN normalized_path TEXT NOT NULL DEFAULT '';

-- Add index for fast lookup
CREATE INDEX IF NOT EXISTS idx_endpoints_normalized_path ON endpoints(normalized_path);

-- Add requested_normalized_path to dependencies table
ALTER TABLE dependencies ADD COLUMN requested_normalized_path TEXT NOT NULL DEFAULT '';

-- Add index for fast lookup
CREATE INDEX IF NOT EXISTS idx_dependencies_requested_normalized_path ON dependencies(requested_normalized_path);
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
-- Add indices for performance
CREATE INDEX IF NOT EXISTS idx_endpoints_branch_deleted ON endpoints(branch_id, deleted);
CREATE INDEX IF NOT EXISTS idx_branches_service_id ON branches(service_id);
CREATE INDEX IF NOT EXISTS idx_dependencies_requested_service_branch ON dependencies(requested_service_id, requested_branch_name);
CREATE INDEX IF NOT EXISTS idx_dependencies_client_id ON dependencies(client_id);
-- Deduplicate existing NULL-endpoint dependency rows, keeping the newest inserted row.
DELETE FROM dependencies
WHERE endpoint_id IS NULL
  AND id NOT IN (
    SELECT MAX(id)
    FROM dependencies
    WHERE endpoint_id IS NULL
    GROUP BY client_id, requested_service_id, requested_branch_name, api_type, requested_path, requested_method
  );

-- Add partial unique index for dependencies with NULL endpoint_id
-- SQLite treats NULLs as distinct for UNIQUE constraints, so the table-level
-- UNIQUE(client_id, endpoint_id, ...) never fires ON CONFLICT when endpoint_id IS NULL.
-- This partial index ensures at most one row per (client, service, branch, api_type, path, method)
-- when the endpoint has not been resolved yet.
CREATE UNIQUE INDEX IF NOT EXISTS idx_dependencies_null_endpoint
ON dependencies (client_id, requested_service_id, requested_branch_name, api_type, requested_path, requested_method)
WHERE endpoint_id IS NULL;
-- Service tags for visual categorization in graph and reports
CREATE TABLE IF NOT EXISTS service_tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    service_id INTEGER NOT NULL,
    tag TEXT NOT NULL,
    FOREIGN KEY (service_id) REFERENCES services(id) ON DELETE CASCADE,
    UNIQUE(service_id, tag)
);

CREATE INDEX IF NOT EXISTS idx_service_tags_service_id ON service_tags(service_id);
-- Track shared endpoint contracts on feature branches for multi-publisher conflict detection
CREATE TABLE IF NOT EXISTS shared_contracts (
    branch_name TEXT NOT NULL,
    api_type TEXT NOT NULL,
    path TEXT NOT NULL,
    method TEXT NOT NULL,
    source_yaml TEXT NOT NULL,
    current_yaml TEXT NOT NULL,
    owner_service_id INTEGER,
    PRIMARY KEY (branch_name, api_type, path, method),
    FOREIGN KEY (owner_service_id) REFERENCES services(id) ON DELETE SET NULL
);
-- Add spec_versions table for optimistic concurrency and caching
CREATE TABLE IF NOT EXISTS service_spec_versions (
    service_id INTEGER NOT NULL,
    branch_id INTEGER NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    content_hash TEXT NOT NULL,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (service_id, branch_id),
    FOREIGN KEY (service_id) REFERENCES services(id),
    FOREIGN KEY (branch_id) REFERENCES branches(id)
);
