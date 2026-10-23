-- How a session signed in: 'local', 'ldap' or 'oidc'. Null for a session
-- started before this was recorded.
ALTER TABLE sessions ADD COLUMN sign_in TEXT NULL;
