# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [Unreleased]

### Added
- `maintenance.yaml`: the `/admin/*` administrative surface now has its own OpenAPI contract, documenting the permission each endpoint requires. `api.yaml` is left as the Producer/Consumer contract alone — the eight admin paths it used to carry moved across. Build-time checks keep both in sync with the router.
- Held Provides inbox: refused breaking changes are listed on the admin dashboard with their reason and the submitted spec, and can be accepted — applying the spec with the refusals waived — or rejected. Administrators see every Producer, maintainers see theirs. Both decisions are audited. See [Administration](docs/administration.md).
- Admin dashboard: role assignment on each user, group management with an origin badge distinguishing Sanshain's own groups from those mirrored from the directory, and maintainer assignment per Producer. Root is shown as configuration-held and cannot be granted or revoked from the UI.
- Producer onboarding: a Producer can be marked as still stabilising, and its Provides to a protected branch are then accepted instead of refused for breaking changes. Soft deletes, version history and version bumps are unaffected, and each accepted change is audited as `ACCEPTED_BREAKING`. See [Administration](docs/administration.md).
- `SANSHAIN_ROOT_USERS`: usernames holding every permission, read from configuration and never stored, so root cannot be revoked from inside the application. Defaults to `INITIAL_ADMIN_USERNAME`. See [Configuration](docs/configuration.md).
- Maintainers: a user or group can be made responsible for specific Producers, administered under `/admin/producers/{name}/maintainers`. A Producer-scoped action admits either the instance-wide permission or maintainership of that Producer. See [Administration](docs/administration.md).
- Roles and groups: users can hold `admin`, `user_manager` or `viewer` directly or through a group, administered under `/admin/roles`, `/admin/users/{id}/roles` and `/admin/groups`. Groups record whether their membership is Sanshain's own or mirrored from the directory. See [Administration](docs/administration.md).

### Changed
- **Breaking:** the administrator flag is gone. On upgrade, every account that had it is granted the `admin` role, and the column is dropped; the change is not reversible, and the grants are the record. `POST /auth/login` no longer returns `is_admin` and `GET /admin/users` no longer reports it — `GET /auth/me` reports `roles` and `permissions` instead. Recovery from a lockout is `SANSHAIN_ROOT_USERS`.
- The web UI now shows what you may do rather than whether you are an administrator: navigation links, the admin dashboard and the endpoint editor gate on permissions, so a user manager sees user administration without being offered the audit timeline. `GET /auth/me` reports `roles`, `permissions` and `is_root`, and `GET /admin/users` reports each user's roles.
- A breaking change on a protected branch is now retained for review instead of discarded, and the `409` carries `pending_id` and `status`. Previously (1.7.0) the submitted spec was thrown away and only an audit line survived, so the only way through was to delete the branch. The audit action `REJECTED_SPEC` is superseded by `QUARANTINED_SPEC`, which keeps the same `REJECT` timeline type.
- Administrative endpoints now require the specific permission they need rather than administrator status, so a `user_manager` reaches user administration and nothing else. Previously (1.7.0) every `/admin/*` route asked only whether the caller was an administrator.
- Every error response is now JSON — `{"error": "..."}` — where previously (1.7.0) errors returned a plain-text body. Tooling that reads the reason out of the raw body must now read the `error` field.

### Fixed
- A directory user's privileges now follow their directory groups. Previously (1.7.0) group membership was read at their first ever login and written to their account, and never refreshed — so a promotion or demotion in the directory never took effect. Membership is now re-read through the configured service account and cached for `SANSHAIN_DIRECTORY_GROUP_TTL_SECS` (default 5 minutes).

## [1.7.0] - 2026-07-29

### Added
- `EXTRA_CA_CERTS_DIR`: directory of CA certificates to trust for LDAPS, added to the platform trust store. Startup fails, naming the file, on a bad certificate. See [Configuration](docs/configuration.md).
- Helm chart: `extraVolumes` and `extraVolumeMounts`. Unset, the chart renders as in 1.6.2.
- Rejected Provides on a Protected branch are recorded in the audit log as `REJECTED_SPEC`, with the Producer, Branch and reason, and are filterable in the timeline under the new "Rejected" type. Previously (1.6.2) a refusal left only a transient log line — and the refusal to remove a non-deprecated endpoint left nothing at all — so there was no way to review what a Protected branch had blocked. The caller still receives the same `409` and message.

### Security
- The audit timeline (`GET /api/audit/timeline`) is now admin-only; previously (1.6.2) any signed-in account or API token could read it. Non-admins now get `403`, anonymous `401`, and API tokens no longer work on this route. The Audit nav link is hidden from non-admins.

### Changed
- A Provide that changes no endpoint no longer bumps the version, stores a revision, or notifies listeners. Previously (1.6.2) every Provide bumped the patch version, so re-publishing an unchanged spec produced a version per build.
- Consequently, an edit confined to `info`, `servers` or other document-level fields is no longer versioned or stored. The first Provide still establishes `1.0.0`.
- The logo images shrank from 3.2 MB to ~123 KB combined, and the favicon is its own small file. Previously (1.6.2) both logos were roughly 20x oversized for how they are drawn and the icon link pointed at the full-resolution logo, so every page load fetched 865 KB and the first dark-mode toggle a further 2.3 MB.

### Fixed
- The **Edit** button on the endpoint view now appears for admins, and the editor opens instead of answering "Admin access required." Editing an endpoint from the UI has never worked since the button was added; the auth helper resolved the signed-in Actor but did not pass it to the code gating the button.
- LDAPS no longer panics on first use with "Could not automatically determine the process-level CryptoProvider", which terminated the process in 1.6.2. Local authentication and plain LDAP were unaffected.
- LDAPS no longer silently trusts nothing when reading the platform certificates partly fails; previously (1.6.2) any such error left an empty trust store.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
