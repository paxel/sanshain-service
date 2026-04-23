-- Service tags for visual categorization in graph and reports
CREATE TABLE IF NOT EXISTS service_tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    service_id INTEGER NOT NULL,
    tag TEXT NOT NULL,
    FOREIGN KEY (service_id) REFERENCES services(id) ON DELETE CASCADE,
    UNIQUE(service_id, tag)
);

CREATE INDEX IF NOT EXISTS idx_service_tags_service_id ON service_tags(service_id);
