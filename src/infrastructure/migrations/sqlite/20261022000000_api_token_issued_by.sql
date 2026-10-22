-- Who issued an API token for a service account: an administrator holding
-- `manage_users`, by username, kept when that account is deleted later.
-- Null for a token its owner created.
ALTER TABLE api_tokens ADD COLUMN issued_by TEXT NULL;
