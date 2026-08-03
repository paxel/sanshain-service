# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.0.0] - 2026-08-02

Sanshain 2.0 replaces branches with **producer-declared versions** (see
[ADR-0003](docs/adr/0003-versions-replace-branches.md)). You publish your spec
under the version written in the document itself and say whether it is an
overwritable `snapshot` or an immutable `ga` release; consumers pin the exact
version they build against. Nothing changes underneath you anymore — not by
inheritance, not by fallback, not by someone else's push.

### Breaking Changes
- **Every request changes shape.** A Provide carries `stability: snapshot | ga` and no version parameter — the version is read from the spec itself (`info.version`; for proto a mandatory `// sanshain-version: MAJOR.MINOR.PATCH` comment). A Require pins a required exact `version`. The 1.x parameters `branch`, `base_version`, `force`, `timeout`, `source_protected_branch` and `pull_from_branch` are rejected by name.
- **Upgrading drops all stored spec data.** Branches, endpoints, per-branch history and dependencies have no honest mapping onto version lines; accounts, tokens, roles, groups, maintainers, settings and the audit log survive. Back up the database, then roll out by republishing each Producer under a real version and repointing Consumers' pins.
- **Resolution is exact-pin only.** You get GA for your pinned version, else the same-numbered snapshot, else an immediate `404`; a version that exists but deliberately lacks the endpoint answers `410`. Previously (1.7.0) a branch without its own spec inherited from an ancestor, and an unknown endpoint could long-poll until `timeout`.
- **Versions must be strict semver.** `1.0`, `v2.0.0` and `1.2.0-SNAPSHOT` are rejected with an explanation — snapshot status is the stability flag, never part of the version string.
- Requests with a wrong-shaped JSON body (missing `stability`, 1.x fields) answer `422`; malformed specs and bad versions answer `400`, both as JSON. Previously (1.7.0) errors could be plain text.

### Added
- **You cannot forget to bump anymore.** Re-publishing a GA version with different content, publishing a snapshot for a released number, or hiding a breaking change behind a minor bump is rejected with `proposed_version` — the exact next version to publish instead. Fix the version, republish, done; no admin involved.
- **Promotion.** Publishing GA for a number that exists as a snapshot releases it in place — iterate on a snapshot, then promote the same version.
- **Snapshots clean themselves up.** A snapshot neither provided nor required for `snapshot_max_age_days` (default 30, `0` disables) is removed; GA versions are never touched, and anything a Consumer still builds against stays.
- `GET /producers/{producername}/versions`: every version of a Producer with stability, endpoint count, last provider and snapshot expiry — the "what can I upgrade to?" answer.
- **Delete-version** for administrators and maintainers: the one escape hatch from GA immutability. The Consumers pinned to the version are shown before you confirm and named in the audit log.
- **The dashboard thinks in versions.** Producers show one timeline per API type with GA/SNAPSHOT badges, diff between any two versions, and per-endpoint blame ("last changed in 1.3.0 by …"); Consumers show their pins, cross-linked; the graph highlights **Outdated** (pinned below the latest GA) and **Snapshot-pinned** dependencies with independent toggles.
- `maintenance.yaml`: the `/admin/*` surface has its own OpenAPI contract naming the permission each endpoint requires, kept in sync with the router by build-time checks.
- Roles, groups and maintainers: users hold `admin`, `user_manager` or `viewer` directly or through groups (with a directory-origin badge), and a user or group can be made maintainer of specific Producers — all administered from the dashboard. See [Administration](docs/administration.md).
- `SANSHAIN_ROOT_USERS`: usernames holding every permission by configuration, so root cannot be revoked from inside the application. Defaults to `INITIAL_ADMIN_USERNAME`. See [Configuration](docs/configuration.md).

### Changed
- Reports are instance-wide: `GET /report` and the markdown/isolation exports cover every recorded dependency with its pinned version and stability, instead of one branch at a time.
- Attribution is the authenticated token: the client-supplied `author` hint is gone from the contract. A promotion carrying byte-identical content keeps the snapshot's provider on the released version — the developer who built it stays credited — while the audit records the promoting Actor.
- Snapshot overwrites are last-writer-wins but never silent: the previous provider is named in the audit trail and shown in the UI.
- AsyncAPI message contracts are keyed globally by `(channel, message)` and enforced on GA publishes only, so work-in-progress can neither claim nor violate topic ownership.
- The audit log records the version where it recorded a branch (old entries stay readable), and its timeline filter parameter is `version`.

### Removed
- The entire branch model: protected-branch patterns, inheritance and fallback, pull-from, per-branch history, stale-branch cleanup, branch deletion/nuke, and `/branches/*`. Pinned versions carry the intent branches were being used to guess at.
- Breaking-change gatekeeping and review: pinned Consumers cannot be broken by a new version, so refusals and held submissions are gone — every rejection is now self-service. (1.7.0's `REJECTED_SPEC` refusals no longer occur.)
- `base_version` optimistic concurrency and require long-polling: GA immutability is the concurrency control, and a missing pinned version is a configuration error that fails immediately.
- Manual endpoint editing: stored content is immutable; the way out is delete-version and a republish.
- The pre-1.6 wire aliases `servicename`/`clientname`: only the canonical `producername`/`consumername` are accepted, and the aliases are rejected as unknown fields.

### Fixed
- A directory user's privileges now follow their directory groups. Previously (1.7.0) membership was read once at first login and never refreshed, so a promotion or demotion in the directory never took effect; it is now re-read and cached for `SANSHAIN_DIRECTORY_GROUP_TTL_SECS` (default 5 minutes).

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
