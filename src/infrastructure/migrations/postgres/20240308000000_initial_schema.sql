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
