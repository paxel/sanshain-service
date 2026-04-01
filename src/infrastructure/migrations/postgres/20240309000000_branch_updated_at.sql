-- Add updated_at column to branches for max-age auto-cleanup
ALTER TABLE branches ADD COLUMN updated_at TEXT NOT NULL DEFAULT '1970-01-01T00:00:00Z';

-- Set existing branches to current time so they don't get immediately cleaned up
UPDATE branches SET updated_at = to_char(now() AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"');

-- Default branch max-age: 30 days
INSERT INTO settings (key, value) VALUES ('branch_max_age_days', '30') ON CONFLICT DO NOTHING;
