-- The open record (valid_to IS NULL) per pin key IS the current pin — every
-- reader of these tables assumes at most one exists. Until now nothing
-- enforced it: the writers check for an open row and insert when they find
-- none, so two concurrent first-time pins for one key could both find nothing
-- and both insert. SQLite serialises writes and never reproduced it; Postgres
-- under READ COMMITTED does, and the duplicate never heals — both rows are
-- refreshed and closed together forever after. The invariant now lives in the
-- schema instead of in the writers' timing.
--
-- The indexes below already existed on exactly these columns, filtered to open
-- rows; only UNIQUE is added, so this costs no new structure and no extra
-- write amplification.

-- Defensive collapse: CREATE UNIQUE INDEX fails outright if duplicates are
-- already present. 2.2 is unreleased, so no deployment can have them — this is
-- insurance, not repair. The newest row per key stays open; the others are
-- closed at their own last_required_at, which is when they stopped being used.
UPDATE trunk_dependencies
SET valid_to = last_required_at
WHERE valid_to IS NULL
  AND id NOT IN (
    SELECT MAX(id) FROM trunk_dependencies
    WHERE valid_to IS NULL
    GROUP BY client_id, service_id, api_type, normalized_path, method
  );

UPDATE branch_dependencies
SET valid_to = last_required_at
WHERE valid_to IS NULL
  AND id NOT IN (
    SELECT MAX(id) FROM branch_dependencies
    WHERE valid_to IS NULL
    GROUP BY branch_id, client_id, service_id, api_type, normalized_path, method
  );

UPDATE branch_member_versions
SET valid_to = valid_from
WHERE valid_to IS NULL
  AND id NOT IN (
    SELECT MAX(id) FROM branch_member_versions
    WHERE valid_to IS NULL
    GROUP BY branch_id, service_id, api_type
  );

DROP INDEX idx_trunk_dependencies_open;
CREATE UNIQUE INDEX idx_trunk_dependencies_open
    ON trunk_dependencies(client_id, service_id, api_type, normalized_path, method)
    WHERE valid_to IS NULL;

DROP INDEX idx_branch_dependencies_open;
CREATE UNIQUE INDEX idx_branch_dependencies_open
    ON branch_dependencies(branch_id, client_id, service_id, api_type, normalized_path, method)
    WHERE valid_to IS NULL;

DROP INDEX idx_branch_member_versions_open;
CREATE UNIQUE INDEX idx_branch_member_versions_open
    ON branch_member_versions(branch_id, service_id, api_type)
    WHERE valid_to IS NULL;
