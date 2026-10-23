-- The name shown for a person: from the directory or the OIDC provider at
-- every sign-in, or set by a local user (or, for a service account, by an
-- administrator). Null shows the username.
ALTER TABLE user_profiles ADD COLUMN display_name TEXT NULL;
