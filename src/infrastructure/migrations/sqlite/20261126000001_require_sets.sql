-- san-1032: versions of a Consumer's require set (the `requires` section of
-- its sanshain.yaml, hashed by the client). Each new hash of a Consumer gets
-- the next number, from trunk and tag requires only; the Pins that arrived
-- under it are kept with it. Keyed by the Consumer's name; deleted with the
-- Consumer.
CREATE TABLE require_sets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    consumer TEXT NOT NULL,
    hash TEXT NOT NULL,
    number INTEGER NOT NULL,
    first_seen_at TEXT NOT NULL,
    UNIQUE (consumer, hash)
);
CREATE TABLE require_set_pins (
    set_id INTEGER NOT NULL REFERENCES require_sets(id) ON DELETE CASCADE,
    producer TEXT NOT NULL,
    api_type TEXT NOT NULL,
    version TEXT NOT NULL,
    PRIMARY KEY (set_id, producer, api_type, version)
);
