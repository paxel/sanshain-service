ALTER TABLE users ADD COLUMN approved BOOLEAN NOT NULL DEFAULT TRUE;

-- Default: local_users disabled
INSERT OR IGNORE INTO settings (key, value) VALUES ('local_users_enabled', 'false');
