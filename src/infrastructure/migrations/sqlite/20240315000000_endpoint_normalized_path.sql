-- Add normalized_path to endpoints table
ALTER TABLE endpoints ADD COLUMN normalized_path TEXT NOT NULL DEFAULT '';

-- Add index for fast lookup
CREATE INDEX IF NOT EXISTS idx_endpoints_normalized_path ON endpoints(normalized_path);

-- Add requested_normalized_path to dependencies table
ALTER TABLE dependencies ADD COLUMN requested_normalized_path TEXT NOT NULL DEFAULT '';

-- Add index for fast lookup
CREATE INDEX IF NOT EXISTS idx_dependencies_requested_normalized_path ON dependencies(requested_normalized_path);
