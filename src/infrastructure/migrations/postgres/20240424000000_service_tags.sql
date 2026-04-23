-- Service tags for visual categorization in graph and reports
CREATE TABLE IF NOT EXISTS service_tags (
    id BIGSERIAL PRIMARY KEY,
    service_id BIGINT NOT NULL REFERENCES services(id) ON DELETE CASCADE,
    tag TEXT NOT NULL,
    UNIQUE(service_id, tag)
);

CREATE INDEX IF NOT EXISTS idx_service_tags_service_id ON service_tags(service_id);
