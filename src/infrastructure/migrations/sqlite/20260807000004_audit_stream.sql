-- ADR-0005: audit entries for provides/requires record the declared stream —
-- 'trunk', a sanshain-branch's tag name, or NULL for plain calls — so "who
-- changed Release Maribou, when?" is one filtered query. Branch lifecycle
-- operations carry their own actions (BRANCH_CREATED/RENAMED/DELETED).
ALTER TABLE audit_logs ADD COLUMN stream TEXT NULL;
