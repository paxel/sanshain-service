-- What Sanshain knows about a person beyond the account itself, one row per
-- user, written at sign-in. last_sign_in is how they last signed in: 'local',
-- 'ldap' or 'oidc'; null until their first sign-in after this was recorded.
CREATE TABLE user_profiles (
    user_id INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    last_sign_in TEXT NULL
);
