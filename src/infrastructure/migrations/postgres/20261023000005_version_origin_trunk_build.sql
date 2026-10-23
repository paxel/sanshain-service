-- Whether the build that sent the origin was the trunk build. Only its build
-- link is shown: branch builds are deleted after the merge. Every origin
-- stored before this is a branch build's as far as anyone knows.
ALTER TABLE spec_version_origin ADD COLUMN trunk_build BOOLEAN NOT NULL DEFAULT FALSE;
