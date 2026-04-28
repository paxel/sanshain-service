-- Deduplicate existing NULL-endpoint dependency rows, keeping the newest inserted row.
DELETE FROM dependencies
WHERE endpoint_id IS NULL
  AND id NOT IN (
    SELECT MAX(id)
    FROM dependencies
    WHERE endpoint_id IS NULL
    GROUP BY client_id, requested_service_id, requested_branch_name, api_type, requested_path, requested_method
  );

-- Add partial unique index for dependencies with NULL endpoint_id
-- SQLite treats NULLs as distinct for UNIQUE constraints, so the table-level
-- UNIQUE(client_id, endpoint_id, ...) never fires ON CONFLICT when endpoint_id IS NULL.
-- This partial index ensures at most one row per (client, service, branch, api_type, path, method)
-- when the endpoint has not been resolved yet.
CREATE UNIQUE INDEX IF NOT EXISTS idx_dependencies_null_endpoint
ON dependencies (client_id, requested_service_id, requested_branch_name, api_type, requested_path, requested_method)
WHERE endpoint_id IS NULL;
