# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [Unreleased]

### Added
- **Versions replace branches** (ADR-0003): a Producer publishes its complete spec under a version read from the document itself — `info.version` for OpenAPI/AsyncAPI, a mandatory `// sanshain-version: MAJOR.MINOR.PATCH` comment for proto — with a declared stability, `snapshot` (overwritable work-in-progress) or `ga` (immutable; the number is permanently claimed, and promotion of a same-numbered snapshot happens in place). Consumers pin an exact version per dependency.
- A Provide refused by the version rules answers `409` with `proposed_version` — the next free number, bumped by what actually changed (breaking → major, additive → minor, otherwise patch). A GA whose changes against the previous GA are breaking without a major bump is refused the same way; snapshots are never compatibility-checked.
- `GET /producers/{producername}/versions`: a Producer's version lines — version, stability, timestamps, last provider, endpoint count and snapshot expiry — for the UI and "what can I upgrade to?" tooling.
- Delete-version (`DELETE /admin/producers/{name}/versions/{api_type}/{version}`): the sole escape hatch from GA immutability, for administrators and maintainers of the Producer. The Consumers pinned to the version are listed before and audited after.
- Snapshot cleanup: snapshots neither provided nor required for `snapshot_max_age_days` (default 30, `0` disables) are removed by the background task; GA versions are never age-culled.
- Graph: two independent highlights — **Outdated** (a pin below the line's latest GA) and **Snapshot-pinned** (a dependency currently served from overwritable content).
- Producer and Consumer views are rebuilt around version lines: one timeline per API type with SNAPSHOT/GA badges, diff between any two versions, per-endpoint blame ("last changed in version X by author Y"), and cross-links between the pinning Consumer and the pinned version.
- `maintenance.yaml`: the `/admin/*` administrative surface now has its own OpenAPI contract, documenting the permission each endpoint requires. `api.yaml` is left as the Producer/Consumer contract alone. Build-time checks keep both in sync with the router.
- Admin dashboard: role assignment on each user, group management with an origin badge distinguishing Sanshain's own groups from those mirrored from the directory, and maintainer assignment per Producer. Root is shown as configuration-held and cannot be granted or revoked from the UI.
- `SANSHAIN_ROOT_USERS`: usernames holding every permission, read from configuration and never stored, so root cannot be revoked from inside the application. Defaults to `INITIAL_ADMIN_USERNAME`. See [Configuration](docs/configuration.md).
- Maintainers: a user or group can be made responsible for specific Producers, administered under `/admin/producers/{name}/maintainers`. A Producer-scoped action admits either the instance-wide permission or maintainership of that Producer. See [Administration](docs/administration.md).
- Roles and groups: users can hold `admin`, `user_manager` or `viewer` directly or through a group, administered under `/admin/roles`, `/admin/users/{id}/roles` and `/admin/groups`. Groups record whether their membership is Sanshain's own or mirrored from the directory. See [Administration](docs/administration.md).

### Changed
- **Breaking:** `api.yaml` is now the 2.0 contract. `/provide*` requests carry `stability` and no version parameter (the spec document is authoritative); `/require*` requests pin a required exact `version`. The 1.x parameters `branch`, `base_version`, `force`, `timeout`, `source_protected_branch` and `pull_from_branch` are rejected as unknown fields. Previously (1.7.0) every request addressed a branch.
- **Breaking:** resolution is exact-pin only — GA preferred, else the same-numbered snapshot, else an immediate `404`; a version that exists but deliberately lacks the endpoint answers `410`. Previously (1.7.0) a branch without its own spec inherited from an ancestor, and an unknown endpoint could long-poll until `timeout`.
- **Breaking:** the 2.0 migration drops all branch-era spec data (branches, endpoints, per-branch history, dependencies, held submissions) — there is no honest mapping onto version lines. Users, tokens, roles, groups, maintainer scopes, settings and the audit log survive. Take a database backup before upgrading; the rollout is: republish each Producer under a real version, then repoint Consumers.
- Reports are instance-wide: `GET /report` (and the markdown/isolation exports) cover every recorded dependency with its pinned version and stability, instead of one branch at a time.
- The audit log records a version where it recorded a branch (column renamed in place; branch-era entries stay readable). The timeline filter parameter is `version` accordingly.
- AsyncAPI channel-message contracts are keyed globally by `(channel, message)` — Kafka topic names are one namespace — and are registered and enforced on GA provides only, so work-in-progress cannot claim or violate topic ownership.
- Every error response is now JSON — `{"error": "..."}` — where previously (1.7.0) errors returned a plain-text body. Tooling that reads the reason out of the raw body must now read the `error` field.

### Removed
- The branch model: protected-branch patterns and their admin surface, branch inheritance and fallback resolution, `source_protected_branch`, `pull_from_branch`, per-branch history and reset-history, stale-branch cleanup, branch deletion and nuke, `/branches/*`, and the merged branch-vs-target report. Pinned versions carry the intent branches were being used to guess at.
- Breaking-change gatekeeping on protected branches (1.7.0's `409` refusals and `REJECTED_SPEC` audit action). Pinned Consumers cannot be broken by a new version, so the check that remains is semver honesty on GA publishes.
- Optimistic concurrency (`base_version`) on Provide: GA immutability is the concurrency control, and snapshot overwrites are last-writer-wins with the previous provider named in the audit trail.
- Long-polling on `/require*` and the `timeout` parameter: with exact pins there is nothing meaningful to wait for — a missing version is a configuration error and fails immediately.
- Manual endpoint editing (`/admin/endpoints/update` and the editor UI): stored content is immutable; the escape hatch is delete-version and a republish.

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
