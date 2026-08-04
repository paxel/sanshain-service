-- Remove the administrator flag (GitHub issue #19).
--
-- Authorisation is expressed in roles and permissions. The flag was the second
-- source of truth for the same question, which is where privilege bugs live.
--
-- The grant is written BEFORE the column is dropped, so an instance with
-- existing administrators keeps them. Getting this order wrong would lock every
-- administrator out of a live instance on upgrade, recoverable only through
-- SANSHAIN_ROOT_USERS.
--
-- Irreversible: once the column is gone the flag cannot be recovered from the
-- data. The role grants it produced are the record.
INSERT INTO user_roles (user_id, role)
SELECT id, 'admin' FROM users WHERE is_admin = TRUE
  AND NOT EXISTS (
    SELECT 1 FROM user_roles ur WHERE ur.user_id = users.id AND ur.role = 'admin'
  );

ALTER TABLE users DROP COLUMN is_admin;
