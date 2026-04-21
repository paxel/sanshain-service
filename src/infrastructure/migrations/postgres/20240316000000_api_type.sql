-- Add api_type to endpoints and dependencies, and update unique constraints.
ALTER TABLE endpoints ADD COLUMN api_type TEXT NOT NULL DEFAULT 'openapi';
ALTER TABLE dependencies ADD COLUMN api_type TEXT NOT NULL DEFAULT 'openapi';

-- Drop and recreate unique constraints for endpoints
-- The name depends on the system, but usually it's [table]_[columns]_key
ALTER TABLE endpoints DROP CONSTRAINT IF EXISTS endpoints_branch_id_path_method_key;
ALTER TABLE endpoints ADD CONSTRAINT endpoints_branch_id_api_type_path_method_key UNIQUE (branch_id, api_type, path, method);

-- Drop and recreate unique constraints for dependencies
ALTER TABLE dependencies DROP CONSTRAINT IF EXISTS dependencies_client_id_endpoint_id_requested_service_id_reques_key;
ALTER TABLE dependencies ADD CONSTRAINT dependencies_client_id_endpoint_id_requested_service_id_branch_api_type_path_method_key UNIQUE (client_id, endpoint_id, requested_service_id, requested_branch_name, api_type, requested_path, requested_method);
