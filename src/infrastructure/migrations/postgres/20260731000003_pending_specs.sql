-- Quarantined Provides (GitHub issue #22, docs/adr/0002).
--
-- Outside onboarding, a Provide carrying a breaking change on a protected branch
-- is held here rather than discarded. Refusal used to destroy the very thing a
-- human would need in order to overrule it: the audit log recorded that
-- something was rejected and why, but the submitted spec was gone.
--
-- One row per Producer, branch and API type, latest replacing previous. Without
-- that, CI pushing on every commit would file the same problem dozens of times
-- and the entry eventually reviewed would be stale by dozens of pushes. With it,
-- what is held is at most one push old — which is also why no cleanup job is
-- needed: the row count is bounded by the number of branches, and a successful
-- Provide deletes the row outright.
CREATE TABLE IF NOT EXISTS pending_specs (
    id BIGSERIAL PRIMARY KEY,
    service_id BIGINT NOT NULL REFERENCES services(id) ON DELETE CASCADE,
    branch TEXT NOT NULL,
    api_type TEXT NOT NULL,
    content TEXT NOT NULL,
    reason TEXT NOT NULL,
    submitted_by TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (service_id, branch, api_type)
);
