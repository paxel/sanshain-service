-- ai/improvements.md #6 / ADR-0006: a Producer's declared AsyncAPI
-- subscriptions, harvested on every provide as *version-less* consumer edges
-- (a subscription has no version, and its channel contract is version-less).
-- Stored in their own table so they are inherently distinct from hand-declared
-- requires — nothing in the pin tables is tagged or reconciled.
--
-- Append-only (ADR-0005): the open row (valid_to IS NULL) per
-- (client_id, channel, message_name) is the current subscription; a retracted
-- one is closed on the next provide, and closed rows are the timeline. The
-- resolved PUB owner is stored BY VALUE (owner_name) so a later owner delete
-- dangles visibly; owner_service_id is NULL when no GA PUB provider owns the
-- channel yet (unfulfilled, kept visible). `trunk` carries the provide's trunk
-- flag (ADR-0004 main vs dev view).
CREATE TABLE harvested_subscriptions (
    id BIGSERIAL PRIMARY KEY,
    client_id BIGINT NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
    client_name TEXT NOT NULL,
    channel TEXT NOT NULL,
    message_name TEXT NOT NULL,
    owner_service_id BIGINT NULL REFERENCES services(id) ON DELETE SET NULL,
    owner_name TEXT NULL,
    trunk BOOLEAN NOT NULL DEFAULT FALSE,
    valid_from TEXT NOT NULL,
    valid_to TEXT NULL
);
CREATE UNIQUE INDEX idx_harvested_subscriptions_open
    ON harvested_subscriptions(client_id, channel, message_name)
    WHERE valid_to IS NULL;
CREATE INDEX idx_harvested_subscriptions_owner
    ON harvested_subscriptions(owner_service_id);
