-- Add fallback_branch column to services
ALTER TABLE services ADD COLUMN fallback_branch TEXT;
