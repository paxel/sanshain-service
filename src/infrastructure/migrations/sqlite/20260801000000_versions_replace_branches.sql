-- Sanshain 2.0 (ADR-0003): producer-declared versions replace branches.
--
-- Clean slate for spec data: the branch-era tables are dropped, not migrated —
-- branch history has no honest mapping onto version lines, and the rollout
-- republishes every Producer under a real version anyway. Users, tokens,
-- groups, roles, maintainer scopes, settings and the audit log survive.
-- The operator's escape hatch is a pre-upgrade backup.

DROP TABLE IF EXISTS endpoint_version_metadata;
DROP TABLE IF EXISTS endpoint_versions;
DROP TABLE IF EXISTS dependencies;
DROP TABLE IF EXISTS endpoints;
DROP TABLE IF EXISTS service_spec_versions;
DROP TABLE IF EXISTS pending_specs;
DROP TABLE IF EXISTS channel_message_contracts;
DROP TABLE IF EXISTS branches;
DROP TABLE IF EXISTS protected_branches;

-- Branch-era service columns. Onboarding existed to soften breaking-change
-- gatekeeping, which pinned consumers make obsolete; the fallback branch has
-- nothing to fall back from.
ALTER TABLE services DROP COLUMN fallback_branch;
ALTER TABLE services DROP COLUMN onboarding;

-- A version line entry: one Producer's spec for one API type under one
-- producer-declared MAJOR.MINOR.PATCH. `stability` is a state, not part of the
-- version string: 'snapshot' rows are overwritable (last writer wins) and may
-- expire; 'ga' rows are immutable and permanently claim their number
-- (promotion flips the state in place). The full provided document is stored
-- alongside the split endpoints so diff views work on whole specs.
CREATE TABLE spec_versions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    service_id INTEGER NOT NULL,
    api_type TEXT NOT NULL,
    major INTEGER NOT NULL,
    minor INTEGER NOT NULL,
    patch INTEGER NOT NULL,
    stability TEXT NOT NULL CHECK (stability IN ('snapshot', 'ga')),
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    provided_by TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    last_required_at TEXT,
    UNIQUE (service_id, api_type, major, minor, patch),
    FOREIGN KEY (service_id) REFERENCES services(id) ON DELETE CASCADE
);
CREATE INDEX idx_spec_versions_line ON spec_versions(service_id, api_type);
CREATE INDEX idx_spec_versions_stability ON spec_versions(stability, updated_at);

-- Split endpoints of one spec version. Replaced wholesale on every provide;
-- no soft deletes — a version's endpoint set is complete by definition, so
-- absence from it *is* the deliberate answer (410 on require).
CREATE TABLE endpoints (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    spec_version_id INTEGER NOT NULL,
    api_type TEXT NOT NULL,
    path TEXT NOT NULL,
    normalized_path TEXT NOT NULL DEFAULT '',
    method TEXT NOT NULL,
    yaml_content TEXT NOT NULL,
    deprecated BOOLEAN NOT NULL DEFAULT FALSE,
    UNIQUE (spec_version_id, api_type, path, method),
    FOREIGN KEY (spec_version_id) REFERENCES spec_versions(id) ON DELETE CASCADE
);
CREATE INDEX idx_endpoints_spec_version ON endpoints(spec_version_id);
CREATE INDEX idx_endpoints_normalized_path ON endpoints(normalized_path);

-- A Consumer's recorded Pin on one endpoint of one spec version. Written only
-- by a successful require — resolution never creates graph entities — and
-- `last_seen_at` keeps both the dependency-age cleanup and the use-based
-- snapshot expiry honest.
CREATE TABLE dependencies (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client_id INTEGER NOT NULL,
    spec_version_id INTEGER NOT NULL,
    api_type TEXT NOT NULL,
    path TEXT NOT NULL,
    normalized_path TEXT NOT NULL DEFAULT '',
    method TEXT NOT NULL,
    last_seen_at TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z',
    UNIQUE (client_id, spec_version_id, api_type, path, method),
    FOREIGN KEY (client_id) REFERENCES clients(id) ON DELETE CASCADE,
    FOREIGN KEY (spec_version_id) REFERENCES spec_versions(id) ON DELETE CASCADE
);
CREATE INDEX idx_dependencies_client ON dependencies(client_id);
CREATE INDEX idx_dependencies_spec_version ON dependencies(spec_version_id);

-- Message-level AsyncAPI channel contracts, minimally adapted: Kafka topic
-- names are a global namespace, so with branches gone the key is simply
-- (channel, message_name). Registered and enforced on GA provides only —
-- snapshots are declared work-in-progress and are never compat-checked.
CREATE TABLE channel_message_contracts (
    channel TEXT NOT NULL,
    message_name TEXT NOT NULL,
    owner_service_id INTEGER NOT NULL,
    payload_yaml TEXT NOT NULL,
    PRIMARY KEY (channel, message_name),
    FOREIGN KEY (owner_service_id) REFERENCES services(id) ON DELETE CASCADE
);

-- Settings: branch cleanup gives way to use-based snapshot expiry.
DELETE FROM settings WHERE key = 'branch_max_age_days';
INSERT OR IGNORE INTO settings (key, value) VALUES ('snapshot_max_age_days', '30');

-- The audit log survives with its data; the column that recorded a branch now
-- records a version string on new entries. Old entries stay readable as text.
ALTER TABLE audit_logs RENAME COLUMN branch TO version;
DROP INDEX IF EXISTS idx_audit_logs_branch;
CREATE INDEX IF NOT EXISTS idx_audit_logs_version ON audit_logs(version);
