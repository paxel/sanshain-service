-- The graph's "(unrequired)" box needs trunk's producers at a past instant,
-- but `spec_versions.trunk_provided_at` is a single mutable stamp (refreshed
-- on every trunk provide, cleared by the trunk TTL) with no history. This is
-- trunk's member-version store, shaped like branch_member_versions: the open
-- row per (service, api_type) is the version trunk carries for that line; a
-- trunk provide of a different version closes it and inserts, the same
-- version refreshes `last_provided_at`. Closed by the trunk TTL, the retire of
-- an API family, a version or producer delete. The producer is by value and
-- without a foreign key, so history survives a delete (#26B convention).
CREATE TABLE trunk_member_versions (
    id BIGSERIAL PRIMARY KEY,
    service_id BIGINT NOT NULL,
    service_name TEXT NOT NULL,
    api_type TEXT NOT NULL,
    major INTEGER NOT NULL,
    minor INTEGER NOT NULL,
    patch INTEGER NOT NULL,
    valid_from TEXT NOT NULL,
    last_provided_at TEXT NOT NULL,
    valid_to TEXT NULL
);
CREATE UNIQUE INDEX idx_trunk_member_versions_open
    ON trunk_member_versions(service_id, api_type)
    WHERE valid_to IS NULL;

-- Backfill from today's markers: per line, the most recently trunk-provided
-- version. Its true start is unknown, so `valid_from` is the marker itself —
-- the earliest instant the membership is known for; earlier instants show it
-- missing rather than claiming a start that was never recorded.
INSERT INTO trunk_member_versions
    (service_id, service_name, api_type, major, minor, patch, valid_from, last_provided_at)
SELECT sv.service_id, s.name, sv.api_type, sv.major, sv.minor, sv.patch,
       sv.trunk_provided_at, sv.trunk_provided_at
FROM spec_versions sv
JOIN services s ON s.id = sv.service_id
WHERE sv.trunk_provided_at IS NOT NULL
  AND NOT EXISTS (
      SELECT 1 FROM spec_versions o
      WHERE o.service_id = sv.service_id
        AND o.api_type = sv.api_type
        AND o.trunk_provided_at IS NOT NULL
        AND (o.trunk_provided_at > sv.trunk_provided_at
             OR (o.trunk_provided_at = sv.trunk_provided_at AND o.id > sv.id)));
