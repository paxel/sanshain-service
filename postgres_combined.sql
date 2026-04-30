-- Core tables
CREATE TABLE IF NOT EXISTS services (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);
CREATE TABLE IF NOT EXISTS branches (
    id BIGSERIAL PRIMARY KEY,
    service_id BIGINT NOT NULL,
    name TEXT NOT NULL,
    UNIQUE(service_id, name),
    FOREIGN KEY(service_id) REFERENCES services(id)
);
CREATE TABLE IF NOT EXISTS endpoints (
    id BIGSERIAL PRIMARY KEY,
    branch_id BIGINT NOT NULL,
    path TEXT NOT NULL,
    method TEXT NOT NULL,
    yaml_content TEXT NOT NULL,
    UNIQUE(branch_id, path, method),
    FOREIGN KEY(branch_id) REFERENCES branches(id)
);
CREATE TABLE IF NOT EXISTS clients (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);
CREATE TABLE IF NOT EXISTS dependencies (
    id BIGSERIAL PRIMARY KEY,
    client_id BIGINT NOT NULL,
    endpoint_id BIGINT,
    requested_service_id BIGINT NOT NULL,
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
    id BIGSERIAL PRIMARY KEY,
    pattern TEXT NOT NULL UNIQUE
);
INSERT INTO protected_branches (pattern) VALUES ('main') ON CONFLICT DO NOTHING;
INSERT INTO protected_branches (pattern) VALUES ('master') ON CONFLICT DO NOTHING;

-- Auth: users, sessions, settings
CREATE TABLE IF NOT EXISTS users (
    id BIGSERIAL PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    is_admin BOOLEAN NOT NULL DEFAULT FALSE,
    approved BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE IF NOT EXISTS sessions (
    token TEXT PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sessions_expires ON sessions(expires_at);
CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
INSERT INTO settings (key, value) VALUES ('dev_mode', 'false') ON CONFLICT DO NOTHING;
INSERT INTO settings (key, value) VALUES ('local_users_enabled', 'false') ON CONFLICT DO NOTHING;

-- API tokens for programmatic access
CREATE TABLE IF NOT EXISTS api_tokens (
    id TEXT PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
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
UPDATE branches SET updated_at = to_char(now() AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"');

-- Default branch max-age: 30 days
INSERT INTO settings (key, value) VALUES ('branch_max_age_days', '30') ON CONFLICT DO NOTHING;
-- Add soft-delete support for endpoints.
-- On protected branches, removed endpoints are marked deleted rather than hard-deleted,
-- so that re-introducing them later is detected as a contract violation.
ALTER TABLE endpoints ADD COLUMN deleted BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE dependencies ADD COLUMN last_seen_at TEXT NOT NULL DEFAULT '1970-01-01T00:00:00';
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
ALTER TABLE endpoints ADD COLUMN api_type TEXT NOT NULL DEFAULT 'openapi';
ALTER TABLE dependencies ADD COLUMN api_type TEXT NOT NULL DEFAULT 'openapi';

-- Drop the old unique constraints by their actual constrained columns instead of
-- relying on PostgreSQL's auto-generated name truncation. Production databases may
-- have either the default names from the initial schema or explicitly named variants.
DO $$
DECLARE
    constraint_to_drop TEXT;
BEGIN
    FOR constraint_to_drop IN
        SELECT con.conname
        FROM pg_constraint con
        JOIN pg_class rel ON rel.oid = con.conrelid
        JOIN pg_namespace nsp ON nsp.oid = rel.relnamespace
        WHERE nsp.nspname = current_schema()
          AND rel.relname = 'endpoints'
          AND con.contype = 'u'
          AND ARRAY(
              SELECT att.attname
              FROM unnest(con.conkey) WITH ORDINALITY AS cols(attnum, ord)
              JOIN pg_attribute att ON att.attrelid = con.conrelid AND att.attnum = cols.attnum
              ORDER BY cols.ord
          ) = ARRAY['branch_id', 'path', 'method']
    LOOP
        EXECUTE format('ALTER TABLE %I.%I DROP CONSTRAINT %I', current_schema(), 'endpoints', constraint_to_drop);
    END LOOP;
END $$;
ALTER TABLE endpoints ADD CONSTRAINT endpoints_branch_id_api_type_path_method_key UNIQUE (branch_id, api_type, path, method);

DO $$
DECLARE
    constraint_to_drop TEXT;
BEGIN
    FOR constraint_to_drop IN
        SELECT con.conname
        FROM pg_constraint con
        JOIN pg_class rel ON rel.oid = con.conrelid
        JOIN pg_namespace nsp ON nsp.oid = rel.relnamespace
        WHERE nsp.nspname = current_schema()
          AND rel.relname = 'dependencies'
          AND con.contype = 'u'
          AND ARRAY(
              SELECT att.attname
              FROM unnest(con.conkey) WITH ORDINALITY AS cols(attnum, ord)
              JOIN pg_attribute att ON att.attrelid = con.conrelid AND att.attnum = cols.attnum
              ORDER BY cols.ord
          ) = ARRAY['client_id', 'endpoint_id', 'requested_service_id', 'requested_branch_name', 'requested_path', 'requested_method']
    LOOP
        EXECUTE format('ALTER TABLE %I.%I DROP CONSTRAINT %I', current_schema(), 'dependencies', constraint_to_drop);
    END LOOP;
END $$;
ALTER TABLE dependencies ADD CONSTRAINT dependencies_client_id_endpoint_id_requested_service_id_branch_api_type_path_method_key UNIQUE (client_id, endpoint_id, requested_service_id, requested_branch_name, api_type, requested_path, requested_method);
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
CREATE UNIQUE INDEX IF NOT EXISTS idx_dependencies_null_endpoint
ON dependencies (client_id, requested_service_id, requested_branch_name, api_type, requested_path, requested_method)
WHERE endpoint_id IS NULL;
-- Service tags for visual categorization in graph and reports
CREATE TABLE IF NOT EXISTS service_tags (
    id BIGSERIAL PRIMARY KEY,
    service_id BIGINT NOT NULL REFERENCES services(id) ON DELETE CASCADE,
    tag TEXT NOT NULL,
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
    owner_service_id BIGINT REFERENCES services(id) ON DELETE SET NULL,
    PRIMARY KEY (branch_name, api_type, path, method)
);
-- Add spec_versions table for optimistic concurrency and caching
CREATE TABLE IF NOT EXISTS service_spec_versions (
    service_id INTEGER NOT NULL REFERENCES services(id),
    branch_id INTEGER NOT NULL REFERENCES branches(id),
    version INTEGER NOT NULL DEFAULT 1,
    content_hash TEXT NOT NULL,
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (service_id, branch_id)
);
