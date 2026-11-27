-- san-1033: Product snapshots, one per umbrella chart version, refreshed by
-- Helm chart pushes. The members keep the chart version the umbrella asks for
-- as written; the scope names the graph the release is read from. Deleted
-- with the Product (explicitly: SQLite applies the cascade only with foreign
-- keys enabled).
CREATE TABLE product_snapshots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    product_id INTEGER NOT NULL REFERENCES products(id) ON DELETE CASCADE,
    chart_version TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    sales_name TEXT NULL,
    sales_name_override INTEGER NOT NULL DEFAULT 0,
    scope TEXT NOT NULL,
    scope_override INTEGER NOT NULL DEFAULT 0,
    pushed_at TEXT NOT NULL,
    pushed_by TEXT NULL,
    UNIQUE (product_id, chart_version)
);
CREATE TABLE product_snapshot_members (
    snapshot_id INTEGER NOT NULL REFERENCES product_snapshots(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    system_name TEXT NOT NULL,
    chart_version TEXT NULL,
    PRIMARY KEY (snapshot_id, system_name)
);
