-- Roles and groups (GitHub issue #14).
--
-- Roles are named bundles of permissions defined in Rust, not here: only the
-- grant is stored, so the bundle cannot drift from what the code enforces.
--
-- A group's `source` records where its membership comes from. 'native' groups
-- are Sanshain's own and carry rows in group_members; 'ldap' groups mirror a
-- directory group, so their membership belongs to the directory and is resolved
-- at check time rather than stored. Uniqueness is on (name, source) so a native
-- and a directory group may share a name without colliding.

CREATE TABLE IF NOT EXISTS user_roles (
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    granted_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (user_id, role)
);

CREATE TABLE IF NOT EXISTS user_groups (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    source TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (name, source)
);

CREATE TABLE IF NOT EXISTS user_group_members (
    group_id BIGINT NOT NULL REFERENCES user_groups(id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (group_id, user_id)
);

CREATE TABLE IF NOT EXISTS user_group_roles (
    group_id BIGINT NOT NULL REFERENCES user_groups(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    PRIMARY KEY (group_id, role)
);

-- Resolving a caller's roles starts from their user id, so membership is read
-- by user far more often than by group.
CREATE INDEX IF NOT EXISTS idx_user_group_members_user ON user_group_members(user_id);
