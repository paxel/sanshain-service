-- PostgreSQL Initial Schema v0.14.0

-- Core tables
CREATE TABLE IF NOT EXISTS services (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    fallback_branch TEXT
);

CREATE TABLE IF NOT EXISTS branches (
    id BIGSERIAL PRIMARY KEY,
    service_id BIGINT NOT NULL,
    name TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z',
    UNIQUE(service_id, name),
    FOREIGN KEY(service_id) REFERENCES services(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_branches_service_id ON branches(service_id);

CREATE TABLE IF NOT EXISTS endpoints (
    id BIGSERIAL PRIMARY KEY,
    branch_id BIGINT NOT NULL,
    api_type TEXT NOT NULL DEFAULT 'openapi',
    path TEXT NOT NULL,
    normalized_path TEXT NOT NULL DEFAULT '',
    method TEXT NOT NULL,
    yaml_content TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    UNIQUE(branch_id, api_type, path, method),
    FOREIGN KEY(branch_id) REFERENCES branches(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_endpoints_branch_deleted ON endpoints(branch_id, deleted);
CREATE INDEX IF NOT EXISTS idx_endpoints_normalized_path ON endpoints(normalized_path);

CREATE TABLE IF NOT EXISTS endpoint_versions (
    id BIGSERIAL PRIMARY KEY,
    endpoint_id BIGINT NOT NULL,
    version INTEGER NOT NULL,
    yaml_content TEXT NOT NULL,
    diff_from_previous TEXT,
    created_at TEXT NOT NULL,
    FOREIGN KEY(endpoint_id) REFERENCES endpoints(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS clients (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS dependencies (
    id BIGSERIAL PRIMARY KEY,
    client_id BIGINT NOT NULL,
    endpoint_id BIGINT,
    api_type TEXT NOT NULL DEFAULT 'openapi',
    requested_service_id BIGINT NOT NULL,
    requested_branch_name TEXT NOT NULL,
    requested_path TEXT NOT NULL,
    requested_normalized_path TEXT NOT NULL DEFAULT '',
    requested_method TEXT NOT NULL,
    last_seen_at TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z',
    UNIQUE(client_id, endpoint_id, requested_service_id, requested_branch_name, api_type, requested_path, requested_method),
    FOREIGN KEY(client_id) REFERENCES clients(id) ON DELETE CASCADE,
    FOREIGN KEY(endpoint_id) REFERENCES endpoints(id) ON DELETE SET NULL,
    FOREIGN KEY(requested_service_id) REFERENCES services(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_dependencies_requested_service_branch ON dependencies(requested_service_id, requested_branch_name);
CREATE INDEX IF NOT EXISTS idx_dependencies_client_id ON dependencies(client_id);
CREATE INDEX IF NOT EXISTS idx_dependencies_requested_normalized_path ON dependencies(requested_normalized_path);
CREATE UNIQUE INDEX idx_dependencies_null_endpoint
ON dependencies (client_id, requested_service_id, requested_branch_name, api_type, requested_path, requested_method)
WHERE endpoint_id IS NULL;

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
INSERT INTO settings (key, value) VALUES ('branch_max_age_days', '30') ON CONFLICT DO NOTHING;

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

-- Service Tags
CREATE TABLE IF NOT EXISTS service_tags (
    service_id BIGINT NOT NULL,
    tag TEXT NOT NULL,
    PRIMARY KEY (service_id, tag),
    FOREIGN KEY (service_id) REFERENCES services(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_service_tags_tag ON service_tags(tag);

-- Shared Contracts
CREATE TABLE IF NOT EXISTS shared_contracts (
    id BIGSERIAL PRIMARY KEY,
    branch_name TEXT NOT NULL,
    api_type TEXT NOT NULL,
    path TEXT NOT NULL,
    method TEXT NOT NULL,
    source_yaml TEXT NOT NULL,
    current_yaml TEXT NOT NULL,
    owner_service_id BIGINT,
    UNIQUE(branch_name, api_type, path, method),
    FOREIGN KEY(owner_service_id) REFERENCES services(id) ON DELETE SET NULL
);

-- Spec Versions
CREATE TABLE IF NOT EXISTS service_spec_versions (
    service_id BIGINT NOT NULL,
    branch_id BIGINT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    content_hash TEXT NOT NULL,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (service_id, branch_id),
    FOREIGN KEY (service_id) REFERENCES services(id) ON DELETE CASCADE,
    FOREIGN KEY (branch_id) REFERENCES branches(id) ON DELETE CASCADE
);
