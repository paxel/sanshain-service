-- ADR-0005: a tag-flagged Provide marks "this is the member version of this
-- producer within that sanshain-branch" (the hotfix act). Same append-only
-- interval shape as the pin stores: the open record per (branch, service,
-- api_type) is the current member version; a different version closes it and
-- inserts. Versions by value.
CREATE TABLE branch_member_versions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    branch_id INTEGER NOT NULL,
    service_id INTEGER NOT NULL,
    api_type TEXT NOT NULL,
    major INTEGER NOT NULL,
    minor INTEGER NOT NULL,
    patch INTEGER NOT NULL,
    valid_from TEXT NOT NULL,
    valid_to TEXT NULL,
    FOREIGN KEY (branch_id) REFERENCES sanshain_branches(id) ON DELETE CASCADE,
    FOREIGN KEY (service_id) REFERENCES services(id) ON DELETE CASCADE
);
CREATE INDEX idx_branch_member_versions_open
    ON branch_member_versions(branch_id, service_id, api_type)
    WHERE valid_to IS NULL;
