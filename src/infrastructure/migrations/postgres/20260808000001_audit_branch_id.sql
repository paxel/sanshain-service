-- ADR-0005 promises that a sanshain-branch's identity is its id: "membership,
-- timeline and audit stamps survive" a rename. Membership and timeline did —
-- they key on branch_id. Audit stamps did not: `stream` holds the branch *name*
-- as it was at write time, so renaming a branch orphaned everything recorded
-- under the old name, and deleting a branch freed the name for a new cut that
-- then inherited the dead one's history.
--
-- branch_id is the identity; `stream` stays as the human label the row was
-- written with (and remains the only marker for the 'trunk' stream, which is
-- not a branch). Deliberately no foreign key: an audit row must outlive the
-- branch it refers to, and a cascade would delete history on branch removal.
ALTER TABLE audit_logs ADD COLUMN branch_id INTEGER NULL;

-- Best effort for rows written before this column existed: match the recorded
-- label against current branch names. Rows whose branch was already renamed or
-- deleted keep branch_id NULL and stay reachable by their recorded label.
UPDATE audit_logs
SET branch_id = (SELECT id FROM sanshain_branches WHERE name = audit_logs.stream)
WHERE stream IS NOT NULL
  AND stream <> 'trunk'
  AND EXISTS (SELECT 1 FROM sanshain_branches WHERE name = audit_logs.stream);

CREATE INDEX idx_audit_logs_branch ON audit_logs(branch_id) WHERE branch_id IS NOT NULL;
