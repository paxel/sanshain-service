-- The Gravatar key of an address the person chose for their picture on the
-- account page. It wins over the provider's key and no sign-in touches it.
-- Only the SHA-256 is stored, never the address. Null when none is set.
ALTER TABLE user_profiles ADD COLUMN own_avatar_hash TEXT NULL;
