-- #46: a system's periphery, read from the `dependsOn` of its Backstage
-- catalog-info.yaml — the Peripheral Resources outside Sanshain it depends on.
--
-- Keyed by value, with no foreign key: a pure Consumer may push its catalog
-- before it ever requires anything, and a branch's copy must survive a later
-- participant delete (ADR-0005 #26B convention).
--
-- `track` is 'dev', 'trunk' or 'branch' (then `branch_id` names the
-- sanshain-branch). A trunk push writes both 'trunk' and 'dev'; a branch gets
-- its rows only as a copy when it is cut.
--
-- Append-only (ADR-0005): the open row (valid_to IS NULL) per
-- (track, branch, system, resource) is current; closed rows are the timeline.
CREATE TABLE peripheral_dependencies (
    id BIGSERIAL PRIMARY KEY,
    track TEXT NOT NULL,
    branch_id BIGINT NULL REFERENCES sanshain_branches(id) ON DELETE CASCADE,
    system_name TEXT NOT NULL,
    resource_ref TEXT NOT NULL,
    valid_from TEXT NOT NULL,
    valid_to TEXT NULL
);
CREATE UNIQUE INDEX idx_peripheral_dependencies_open
    ON peripheral_dependencies(track, COALESCE(branch_id, 0), system_name, resource_ref)
    WHERE valid_to IS NULL;
CREATE INDEX idx_peripheral_dependencies_system
    ON peripheral_dependencies(system_name);
