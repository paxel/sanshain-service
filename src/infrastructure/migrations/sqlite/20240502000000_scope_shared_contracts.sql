-- Scope shared contracts by (branch_name, service_id) instead of branch_name alone
DROP TABLE IF EXISTS shared_contracts;

CREATE TABLE shared_contracts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    branch_name TEXT NOT NULL,
    service_id INTEGER NOT NULL,
    api_type TEXT NOT NULL,
    path TEXT NOT NULL,
    method TEXT NOT NULL,
    source_yaml TEXT NOT NULL,
    current_yaml TEXT NOT NULL,
    owner_service_id INTEGER,
    UNIQUE(branch_name, service_id, api_type, path, method),
    FOREIGN KEY(service_id) REFERENCES services(id) ON DELETE CASCADE,
    FOREIGN KEY(owner_service_id) REFERENCES services(id) ON DELETE SET NULL
);
