-- Per-branch fallback target (ai/testing-findings-2026-07-23.md item #17, GitHub issue #1):
-- which protected branch a branch defers to when it has no data of its own.
-- Sticky: set once by whichever caller supplies it first, then only an admin
-- may change it (see application-layer write-once semantics).
ALTER TABLE branches ADD COLUMN source_protected_branch TEXT;
