-- ai/improvements.md #26 Problem B: the append-only pin stores claim "closed
-- records are the timeline", but client_id/service_id carried ON DELETE
-- CASCADE, so deleting a Producer or Consumer physically erased its rows —
-- including the closed history the timeline reconstructs from. The pin already
-- references the pinned *version* by value (a deleted version dangles, it does
-- not vanish); this makes it reference the *participant* by value too, so a
-- deleted participant's history survives just the same.
--
-- Each table is rebuilt: the participant name columns are added and backfilled
-- from the current join, and the client/service foreign keys lose their
-- cascade (branch_id keeps it — deleting a branch SHOULD remove its rows).
-- SQLite cannot ALTER a constraint, hence the create/copy/drop/rename. These
-- are leaf tables (nothing references them), so the rebuild needs no
-- foreign_keys toggle — which would be a no-op inside the migration
-- transaction anyway.

-- ── trunk_dependencies ───────────────────────────────────────────────
CREATE TABLE trunk_dependencies_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_id INTEGER NOT NULL,
    client_name TEXT NOT NULL,
    service_id INTEGER NOT NULL,
    service_name TEXT NOT NULL,
    api_type TEXT NOT NULL,
    path TEXT NOT NULL,
    normalized_path TEXT NOT NULL,
    method TEXT NOT NULL,
    major INTEGER NOT NULL,
    minor INTEGER NOT NULL,
    patch INTEGER NOT NULL,
    valid_from TEXT NOT NULL,
    last_required_at TEXT NOT NULL,
    valid_to TEXT NULL
);
INSERT INTO trunk_dependencies_new
    (id, client_id, client_name, service_id, service_name, api_type, path,
     normalized_path, method, major, minor, patch, valid_from, last_required_at, valid_to)
SELECT t.id, t.client_id, COALESCE(c.name, '(deleted)'), t.service_id,
       COALESCE(s.name, '(deleted)'), t.api_type, t.path, t.normalized_path,
       t.method, t.major, t.minor, t.patch, t.valid_from, t.last_required_at, t.valid_to
FROM trunk_dependencies t
LEFT JOIN clients c ON c.id = t.client_id
LEFT JOIN services s ON s.id = t.service_id;
DROP TABLE trunk_dependencies;
ALTER TABLE trunk_dependencies_new RENAME TO trunk_dependencies;
CREATE UNIQUE INDEX idx_trunk_dependencies_open
    ON trunk_dependencies(client_id, service_id, api_type, normalized_path, method)
    WHERE valid_to IS NULL;
CREATE INDEX idx_trunk_dependencies_service ON trunk_dependencies(service_id);

-- ── branch_dependencies ──────────────────────────────────────────────
CREATE TABLE branch_dependencies_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    branch_id INTEGER NOT NULL,
    client_id INTEGER NOT NULL,
    client_name TEXT NOT NULL,
    service_id INTEGER NOT NULL,
    service_name TEXT NOT NULL,
    api_type TEXT NOT NULL,
    path TEXT NOT NULL,
    normalized_path TEXT NOT NULL,
    method TEXT NOT NULL,
    major INTEGER NOT NULL,
    minor INTEGER NOT NULL,
    patch INTEGER NOT NULL,
    valid_from TEXT NOT NULL,
    last_required_at TEXT NOT NULL,
    valid_to TEXT NULL,
    FOREIGN KEY (branch_id) REFERENCES sanshain_branches(id) ON DELETE CASCADE
);
INSERT INTO branch_dependencies_new
    (id, branch_id, client_id, client_name, service_id, service_name, api_type, path,
     normalized_path, method, major, minor, patch, valid_from, last_required_at, valid_to)
SELECT b.id, b.branch_id, b.client_id, COALESCE(c.name, '(deleted)'), b.service_id,
       COALESCE(s.name, '(deleted)'), b.api_type, b.path, b.normalized_path,
       b.method, b.major, b.minor, b.patch, b.valid_from, b.last_required_at, b.valid_to
FROM branch_dependencies b
LEFT JOIN clients c ON c.id = b.client_id
LEFT JOIN services s ON s.id = b.service_id;
DROP TABLE branch_dependencies;
ALTER TABLE branch_dependencies_new RENAME TO branch_dependencies;
CREATE UNIQUE INDEX idx_branch_dependencies_open
    ON branch_dependencies(branch_id, client_id, service_id, api_type, normalized_path, method)
    WHERE valid_to IS NULL;
CREATE INDEX idx_branch_dependencies_branch ON branch_dependencies(branch_id);

-- ── branch_member_versions ───────────────────────────────────────────
CREATE TABLE branch_member_versions_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    branch_id INTEGER NOT NULL,
    service_id INTEGER NOT NULL,
    service_name TEXT NOT NULL,
    api_type TEXT NOT NULL,
    major INTEGER NOT NULL,
    minor INTEGER NOT NULL,
    patch INTEGER NOT NULL,
    valid_from TEXT NOT NULL,
    valid_to TEXT NULL,
    FOREIGN KEY (branch_id) REFERENCES sanshain_branches(id) ON DELETE CASCADE
);
INSERT INTO branch_member_versions_new
    (id, branch_id, service_id, service_name, api_type, major, minor, patch, valid_from, valid_to)
SELECT m.id, m.branch_id, m.service_id, COALESCE(s.name, '(deleted)'),
       m.api_type, m.major, m.minor, m.patch, m.valid_from, m.valid_to
FROM branch_member_versions m
LEFT JOIN services s ON s.id = m.service_id;
DROP TABLE branch_member_versions;
ALTER TABLE branch_member_versions_new RENAME TO branch_member_versions;
CREATE UNIQUE INDEX idx_branch_member_versions_open
    ON branch_member_versions(branch_id, service_id, api_type)
    WHERE valid_to IS NULL;

