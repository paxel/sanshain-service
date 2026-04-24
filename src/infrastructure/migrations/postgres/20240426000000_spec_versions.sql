-- Add spec_versions table for optimistic concurrency and caching
CREATE TABLE IF NOT EXISTS service_spec_versions (
    service_id INTEGER NOT NULL REFERENCES services(id),
    branch_id INTEGER NOT NULL REFERENCES branches(id),
    version INTEGER NOT NULL DEFAULT 1,
    content_hash TEXT NOT NULL,
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (service_id, branch_id)
);
