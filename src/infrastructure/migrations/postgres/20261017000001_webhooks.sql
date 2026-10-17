-- #47 (ADR-0009): signed outbound events with outbox delivery.
--
-- `webhooks` holds the registered hooks. A hook is global (`scope_kind =
-- 'global'`) or belongs to one system (`producer` or `consumer`, with the
-- name in `scope_name`). The signing secret is stored in full because the
-- worker signs with it; `secret_hint` is the prefix of its SHA-256 the
-- listing shows. `event_types` is a comma-separated list, NULL = every type.
CREATE TABLE webhooks (
    id BIGSERIAL PRIMARY KEY,
    scope_kind TEXT NOT NULL,
    scope_name TEXT NULL,
    url TEXT NOT NULL,
    format TEXT NOT NULL,
    event_types TEXT NULL,
    producer_glob TEXT NULL,
    secret TEXT NOT NULL,
    secret_hint TEXT NOT NULL,
    paused BOOLEAN NOT NULL DEFAULT FALSE,
    allow_list_exempt BOOLEAN NOT NULL DEFAULT FALSE,
    created_by TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_webhooks_scope ON webhooks(scope_kind, scope_name);

-- One row per event, inserted in the same transaction as the fact it
-- describes. `data_json` is the envelope's `data`; the envelope itself is
-- assembled at delivery.
CREATE TABLE webhook_outbox (
    id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    producer TEXT NULL,
    occurred_at TEXT NOT NULL,
    data_json TEXT NOT NULL
);

-- One row per hook per event — the fan-out, decided when the event is
-- written. `next_attempt_at` is when the worker may claim it next; a claim
-- (`FOR UPDATE SKIP LOCKED`) moves it forward as a lease so two replicas
-- never send the same row.
CREATE TABLE webhook_deliveries (
    id BIGSERIAL PRIMARY KEY,
    hook_id BIGINT NOT NULL REFERENCES webhooks(id) ON DELETE CASCADE,
    event_id TEXT NOT NULL REFERENCES webhook_outbox(id) ON DELETE CASCADE,
    status TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TEXT NOT NULL,
    last_response_code INTEGER NULL,
    last_error TEXT NULL,
    delivered_at TEXT NULL,
    -- Set by a manual redeliver: the row gets one more attempt even past
    -- the 24 h cutoff.
    redelivered_at TEXT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_webhook_deliveries_due ON webhook_deliveries(status, next_attempt_at);
CREATE INDEX idx_webhook_deliveries_hook ON webhook_deliveries(hook_id, id);
