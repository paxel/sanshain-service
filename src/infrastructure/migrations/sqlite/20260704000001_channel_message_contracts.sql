-- Message-level AsyncAPI channel contracts (ai/improvements.md item #20).
-- Kafka topic names are a global namespace, so contract identity is the
-- message -- keyed by (branch_name, channel, message_name) -- not the whole
-- channel payload. Only PUB (publish) messages register a contract; the first
-- providing service owns the message schema. Enforced on all branches.
CREATE TABLE IF NOT EXISTS channel_message_contracts (
    branch_name TEXT NOT NULL,
    channel TEXT NOT NULL,
    message_name TEXT NOT NULL,
    owner_service_id INTEGER NOT NULL,
    payload_yaml TEXT NOT NULL,
    PRIMARY KEY (branch_name, channel, message_name),
    FOREIGN KEY (owner_service_id) REFERENCES services(id) ON DELETE CASCADE
);
