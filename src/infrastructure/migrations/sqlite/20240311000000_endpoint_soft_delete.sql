-- Add soft-delete support for endpoints.
-- On protected branches, removed endpoints are marked deleted rather than hard-deleted,
-- so that re-introducing them later is detected as a contract violation.
ALTER TABLE endpoints ADD COLUMN deleted BOOLEAN NOT NULL DEFAULT FALSE;
