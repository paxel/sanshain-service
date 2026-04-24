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
