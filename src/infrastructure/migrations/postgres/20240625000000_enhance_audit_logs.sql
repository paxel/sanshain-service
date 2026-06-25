-- Enhance audit logs with service, branch and action_type for better filtering
ALTER TABLE audit_logs ADD COLUMN service TEXT;
ALTER TABLE audit_logs ADD COLUMN branch TEXT;
ALTER TABLE audit_logs ADD COLUMN action_type TEXT;
ALTER TABLE audit_logs ADD COLUMN diff TEXT;

-- Create indices for better performance on filtered queries
CREATE INDEX idx_audit_logs_service ON audit_logs(service);
CREATE INDEX idx_audit_logs_branch ON audit_logs(branch);
CREATE INDEX idx_audit_logs_action_type ON audit_logs(action_type);
