-- Deduplicate existing NULL-endpoint dependency rows, keeping the one with the latest last_seen_at
DELETE FROM dependencies
WHERE endpoint_id IS NULL
  AND id NOT IN (
    SELECT MAX(id)
    FROM dependencies
    WHERE endpoint_id IS NULL
    GROUP BY client_id, requested_service_id, requested_branch_name, api_type, requested_path, requested_method
  );

-- Add partial unique index for dependencies with NULL endpoint_id
CREATE UNIQUE INDEX IF NOT EXISTS idx_dependencies_null_endpoint
ON dependencies (client_id, requested_service_id, requested_branch_name, api_type, requested_path, requested_method)
WHERE endpoint_id IS NULL;
