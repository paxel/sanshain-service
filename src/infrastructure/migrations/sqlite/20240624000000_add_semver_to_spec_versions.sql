-- Add missing columns to services
ALTER TABLE services ADD COLUMN icon TEXT;
ALTER TABLE services ADD COLUMN domain TEXT;

-- Add missing external column to endpoints
ALTER TABLE endpoints ADD COLUMN external BOOLEAN NOT NULL DEFAULT 0;

-- Add SemVer columns to service_spec_versions
ALTER TABLE service_spec_versions ADD COLUMN major INTEGER NOT NULL DEFAULT 1;
ALTER TABLE service_spec_versions ADD COLUMN minor INTEGER NOT NULL DEFAULT 0;
ALTER TABLE service_spec_versions ADD COLUMN patch INTEGER NOT NULL DEFAULT 0;

-- Backfill existing versions: major = version, minor = 0, patch = 0
UPDATE service_spec_versions SET major = version, minor = 0, patch = 0;
