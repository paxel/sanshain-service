-- Unmask redacted usernames in audit logs and metadata
UPDATE audit_logs SET username = 'root' WHERE username = 'r**t';
UPDATE audit_logs SET username = 'dev_user' WHERE username = 'd******r';
UPDATE endpoint_version_metadata SET username = 'root' WHERE username = 'r**t';
UPDATE endpoint_version_metadata SET username = 'dev_user' WHERE username = 'd******r';
