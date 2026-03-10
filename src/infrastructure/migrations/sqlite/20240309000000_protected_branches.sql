CREATE TABLE IF NOT EXISTS protected_branches (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    pattern TEXT NOT NULL UNIQUE
);

-- By default, 'main' and 'master' are protected
INSERT OR IGNORE INTO protected_branches (pattern) VALUES ('main');
INSERT OR IGNORE INTO protected_branches (pattern) VALUES ('master');
