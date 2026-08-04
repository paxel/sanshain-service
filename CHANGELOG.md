# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.1.0] - 2026-08-04

### Security
- Publishing `stability: ga` now requires the `release_ga` permission (new `releaser` role; admins
  and root hold it implicitly) — previously any authenticated token could release, and snapshots are
  unaffected. **On upgrade, grant `releaser` to whatever pushes GA today — typically your CI user —
  or releases fail with an instructive `403`.**

### Added
- `releaser`: a global role conferring exactly `release_ga`, grantable to users, native groups and
  directory groups. Like `admin`, it is admin-guarded — only an admin (or root) may grant or revoke
  it.
- Refused release attempts are recorded: a real (non-dry-run) GA Provide without the permission
  writes a `VERSION_REJECTED` audit entry and increments
  `sanshain_version_rejected_total{reason="ga_requires_releaser"}`. Dry-runs get the same `403`
  without the telemetry.
- **One-click promote.** Snapshot entries on the producers page and the admin dashboard offer a
  "Promote to GA" button to holders of `release_ga`
  (`POST /admin/producers/{name}/versions/{api_type}/{version}/promote`). It releases the stored
  content in place — same gate, attribution and audit trail as re-providing it as GA; promoting an
  already-GA version is a harmless no-op.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
