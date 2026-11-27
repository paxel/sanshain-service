-- san-1029: the git branch a version was built from, and the branch a pull or
-- merge request build merges into, as the client read them from the runner.
-- Part of the origin; what the admin's naming patterns read from the branch
-- is computed when shown, never stored.
ALTER TABLE spec_version_origin ADD COLUMN git_branch TEXT NULL;
ALTER TABLE spec_version_origin ADD COLUMN git_target_branch TEXT NULL;
