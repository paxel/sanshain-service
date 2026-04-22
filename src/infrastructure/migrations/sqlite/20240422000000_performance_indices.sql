-- Add indices for performance
CREATE INDEX IF NOT EXISTS idx_endpoints_branch_deleted ON endpoints(branch_id, deleted);
CREATE INDEX IF NOT EXISTS idx_branches_service_id ON branches(service_id);
CREATE INDEX IF NOT EXISTS idx_dependencies_requested_service_branch ON dependencies(requested_service_id, requested_branch_name);
CREATE INDEX IF NOT EXISTS idx_dependencies_client_id ON dependencies(client_id);
