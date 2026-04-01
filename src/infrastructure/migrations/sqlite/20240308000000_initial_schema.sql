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
