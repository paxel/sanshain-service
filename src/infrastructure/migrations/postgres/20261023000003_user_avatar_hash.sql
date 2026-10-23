-- The Gravatar key of a person: the SHA-256 of their trimmed, lowercased
-- e-mail address from the directory or the OIDC provider, written at sign-in.
-- The address itself is not stored. Null when none is known.
ALTER TABLE user_profiles ADD COLUMN avatar_hash TEXT NULL;
