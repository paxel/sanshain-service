-- The directory's immutable ID of the person (objectGUID in Active Directory,
-- entryUUID in OpenLDAP), written at LDAP sign-in. It recognises them after a
-- rename and folds accounts that belong to the same entry into one.
ALTER TABLE user_profiles ADD COLUMN directory_id TEXT NULL;
CREATE UNIQUE INDEX idx_user_profiles_directory_id ON user_profiles (directory_id);
