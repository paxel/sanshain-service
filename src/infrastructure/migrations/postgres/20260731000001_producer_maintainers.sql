-- Maintainer scope (GitHub issue #16).
--
-- Maintainership is deliberately not a role. `admin` and `user_manager` are held
-- instance-wide; being responsible for a Producer means nothing without the set
-- of Producers it is over, so it is stored as an assignment rather than granted.
--
-- Two tables rather than one with nullable columns: a row assigns either a user
-- or a group, never both, and splitting them lets the database say so instead of
-- leaving it to application code.

CREATE TABLE IF NOT EXISTS producer_user_maintainers (
    service_id BIGINT NOT NULL REFERENCES services(id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    assigned_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (service_id, user_id)
);

CREATE TABLE IF NOT EXISTS producer_group_maintainers (
    service_id BIGINT NOT NULL REFERENCES services(id) ON DELETE CASCADE,
    group_id BIGINT NOT NULL REFERENCES user_groups(id) ON DELETE CASCADE,
    assigned_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (service_id, group_id)
);

-- The hot question is "does this caller maintain this Producer?", asked per
-- request, so both directions are indexed by the caller's side.
CREATE INDEX IF NOT EXISTS idx_producer_user_maintainers_user
    ON producer_user_maintainers(user_id);
CREATE INDEX IF NOT EXISTS idx_producer_group_maintainers_group
    ON producer_group_maintainers(group_id);
