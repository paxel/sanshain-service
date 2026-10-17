-- #48: what a system's Backstage catalog-info.yaml says about it beyond its
-- periphery — owner (kind and name), description, source location and links.
--
-- One row per system, keyed by value with no foreign key (a pure Consumer
-- may push before it ever requires anything), replaced on every push and
-- deleted when the system has no role left. A present-day fact about the
-- system, not a graph fact: not tracked per stream or sanshain-branch, so
-- no timeline and no branch copy.
--
-- `links_json` is a JSON array of {url, title, type, icon}, at most 20.
CREATE TABLE system_context (
    system_name TEXT PRIMARY KEY,
    owner_kind TEXT NULL,
    owner_name TEXT NULL,
    description TEXT NULL,
    source_location TEXT NULL,
    links_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
