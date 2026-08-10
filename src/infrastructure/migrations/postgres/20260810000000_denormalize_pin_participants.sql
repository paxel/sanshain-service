-- ai/improvements.md #26 Problem B: the append-only pin stores claim "closed
-- records are the timeline", but client_id/service_id carried ON DELETE
-- CASCADE, so deleting a Producer or Consumer erased its rows including the
-- closed history the timeline reconstructs from. The pin already references the
-- pinned version by value; this makes it reference the participant by value
-- too, so a deleted participant's history survives. The client/service foreign
-- keys lose their cascade (branch_id keeps it — deleting a branch should remove
-- its rows).

ALTER TABLE trunk_dependencies ADD COLUMN client_name TEXT;
ALTER TABLE trunk_dependencies ADD COLUMN service_name TEXT;
UPDATE trunk_dependencies t
   SET client_name = COALESCE((SELECT name FROM clients WHERE id = t.client_id), '(deleted)'),
       service_name = COALESCE((SELECT name FROM services WHERE id = t.service_id), '(deleted)');
ALTER TABLE trunk_dependencies ALTER COLUMN client_name SET NOT NULL;
ALTER TABLE trunk_dependencies ALTER COLUMN service_name SET NOT NULL;
ALTER TABLE trunk_dependencies DROP CONSTRAINT trunk_dependencies_client_id_fkey;
ALTER TABLE trunk_dependencies DROP CONSTRAINT trunk_dependencies_service_id_fkey;

ALTER TABLE branch_dependencies ADD COLUMN client_name TEXT;
ALTER TABLE branch_dependencies ADD COLUMN service_name TEXT;
UPDATE branch_dependencies b
   SET client_name = COALESCE((SELECT name FROM clients WHERE id = b.client_id), '(deleted)'),
       service_name = COALESCE((SELECT name FROM services WHERE id = b.service_id), '(deleted)');
ALTER TABLE branch_dependencies ALTER COLUMN client_name SET NOT NULL;
ALTER TABLE branch_dependencies ALTER COLUMN service_name SET NOT NULL;
ALTER TABLE branch_dependencies DROP CONSTRAINT branch_dependencies_client_id_fkey;
ALTER TABLE branch_dependencies DROP CONSTRAINT branch_dependencies_service_id_fkey;

ALTER TABLE branch_member_versions ADD COLUMN service_name TEXT;
UPDATE branch_member_versions m
   SET service_name = COALESCE((SELECT name FROM services WHERE id = m.service_id), '(deleted)');
ALTER TABLE branch_member_versions ALTER COLUMN service_name SET NOT NULL;
ALTER TABLE branch_member_versions DROP CONSTRAINT branch_member_versions_service_id_fkey;
