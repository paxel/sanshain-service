-- Add api_type to endpoints and dependencies, and update unique constraints.
ALTER TABLE endpoints ADD COLUMN api_type TEXT NOT NULL DEFAULT 'openapi';
ALTER TABLE dependencies ADD COLUMN api_type TEXT NOT NULL DEFAULT 'openapi';

-- Drop the old unique constraints by their actual constrained columns instead of
-- relying on PostgreSQL's auto-generated name truncation. Production databases may
-- have either the default names from the initial schema or explicitly named variants.
DO $$
DECLARE
    constraint_to_drop TEXT;
BEGIN
    FOR constraint_to_drop IN
        SELECT con.conname
        FROM pg_constraint con
        JOIN pg_class rel ON rel.oid = con.conrelid
        JOIN pg_namespace nsp ON nsp.oid = rel.relnamespace
        WHERE nsp.nspname = current_schema()
          AND rel.relname = 'endpoints'
          AND con.contype = 'u'
          AND ARRAY(
              SELECT att.attname
              FROM unnest(con.conkey) WITH ORDINALITY AS cols(attnum, ord)
              JOIN pg_attribute att ON att.attrelid = con.conrelid AND att.attnum = cols.attnum
              ORDER BY cols.ord
          ) = ARRAY['branch_id', 'path', 'method']
    LOOP
        EXECUTE format('ALTER TABLE %I.%I DROP CONSTRAINT %I', current_schema(), 'endpoints', constraint_to_drop);
    END LOOP;
END $$;
ALTER TABLE endpoints ADD CONSTRAINT endpoints_branch_id_api_type_path_method_key UNIQUE (branch_id, api_type, path, method);

DO $$
DECLARE
    constraint_to_drop TEXT;
BEGIN
    FOR constraint_to_drop IN
        SELECT con.conname
        FROM pg_constraint con
        JOIN pg_class rel ON rel.oid = con.conrelid
        JOIN pg_namespace nsp ON nsp.oid = rel.relnamespace
        WHERE nsp.nspname = current_schema()
          AND rel.relname = 'dependencies'
          AND con.contype = 'u'
          AND ARRAY(
              SELECT att.attname
              FROM unnest(con.conkey) WITH ORDINALITY AS cols(attnum, ord)
              JOIN pg_attribute att ON att.attrelid = con.conrelid AND att.attnum = cols.attnum
              ORDER BY cols.ord
          ) = ARRAY['client_id', 'endpoint_id', 'requested_service_id', 'requested_branch_name', 'requested_path', 'requested_method']
    LOOP
        EXECUTE format('ALTER TABLE %I.%I DROP CONSTRAINT %I', current_schema(), 'dependencies', constraint_to_drop);
    END LOOP;
END $$;
ALTER TABLE dependencies ADD CONSTRAINT dependencies_client_id_endpoint_id_requested_service_id_branch_api_type_path_method_key UNIQUE (client_id, endpoint_id, requested_service_id, requested_branch_name, api_type, requested_path, requested_method);
