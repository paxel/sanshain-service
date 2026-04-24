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
