-- ADR-0004: a trunk-flagged Provide marks the version entry as "trunk's
-- current version" (as opposed to a release-branch GA backport). NULL means
-- never trunk-provided. Refreshed on every real trunk provide, including the
-- idempotent no-op; cleared again by the trunk TTL cleanup when stale.
ALTER TABLE spec_versions ADD COLUMN trunk_provided_at TEXT NULL;
