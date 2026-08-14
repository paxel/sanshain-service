-- ADR-0005: sanshain-branches — named graphs ("Release Maribou") created by a
-- releaser as a copy of a source graph (trunk, or another branch) at a chosen
-- instant, then updated by tag-flagged hotfix builds. Branch identity is the
-- id; the unique-among-live name is a label (rename-safe). Deleting a branch
-- removes its rows (the deliberate, audited EOL act).
CREATE TABLE sanshain_branches (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    created_by TEXT NOT NULL,
    -- What the branch was created from: 'trunk' or the source branch's
    -- then-current name. Informational provenance, not a live reference.
    source TEXT NOT NULL,
    as_of TEXT NOT NULL
);

-- Branch graph rows: the same append-only shape as trunk_dependencies, keyed
-- by branch. The open record per pin key is the branch's current pin; closed
-- records are the branch's own timeline. Versions by value (ADR-0005) — a
-- deleted spec version leaves a visibly dangling reference that heals on
-- re-provide.
CREATE TABLE branch_dependencies (
    id BIGSERIAL PRIMARY KEY,
    branch_id BIGINT NOT NULL,
    client_id BIGINT NOT NULL,
    service_id BIGINT NOT NULL,
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
    FOREIGN KEY (branch_id) REFERENCES sanshain_branches(id) ON DELETE CASCADE,
    FOREIGN KEY (client_id) REFERENCES clients(id) ON DELETE CASCADE,
    FOREIGN KEY (service_id) REFERENCES services(id) ON DELETE CASCADE
);
CREATE INDEX idx_branch_dependencies_open
    ON branch_dependencies(branch_id, client_id, service_id, api_type, normalized_path, method)
    WHERE valid_to IS NULL;
CREATE INDEX idx_branch_dependencies_branch ON branch_dependencies(branch_id);
