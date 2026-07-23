# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [1.5.2] - 2026-07-23

### Added
- The services overview now shows each branch's **last-published time** on its branch card (or "No publishes yet" when it has never received a publishing change). The timestamp reflects the last spec change published to that branch, not merely when it was last viewed.

### Fixed
- Opening an endpoint's history for a branch the server never published — for example one a client *requires* but the service only *provides* on `master` — now falls back to the service's configured fallback branch, then to any protected branch, and shows **that** branch's history with a note ("Branch X has no published spec — showing history from master"). Previously (1.5.1) the history view (`/admin/endpoint-versions`) queried the exact branch only and failed with a raw "Failed to load versions: 404". Only when no branch at all has the endpoint does it now show an explicit "not available" state. (The endpoint YAML view already resolved branches this way; the history view now matches it.)
- The "resolved → service" link on a client's endpoint now carries the endpoint's API type, so AsyncAPI and gRPC/proto endpoints open with the correct viewer instead of defaulting to OpenAPI.
- A branch's "last updated" time (`branches.updated_at`) is now advanced only when a spec change is actually published to it, not when the branch is merely viewed. Previously (1.5.1) any read that resolved the branch — viewing an endpoint's history, listing a branch's endpoints — bumped the timestamp, with two consequences: "last activity" reflected the last *view* instead of the last *publish*, and the stale-branch cleanup (which deletes branches whose `updated_at` is older than the retention window) had its timer reset just by someone looking at the branch, so a viewed-but-unpublished branch was never culled. Viewing no longer keeps a branch alive; only a publishing change does. (A no-op re-publish of an identical spec does not advance it either, consistent with it producing no change.)

### Changed
- Branches in the services overview are now ordered deterministically — protected branches (e.g. `master`/`main`) first, then by most recent publish (newest first), then alphabetically. Previously (1.5.1) the per-service branch list came back in an unspecified order (SQLite `GROUP_CONCAT`), so it could look unsorted and vary between requests.
- Viewing a branch no longer creates a "Generated report" audit entry. The branch view fetches `GET /report` to render, and that read is no longer audited — so the audit timeline stops filling with a report row on every view. Explicit report *exports* (`GET /report/markdown`, `/report/isolation`, `/report/merged`) are still audited. Previously (1.5.1) each branch view added a `REPORT`/`READ` audit entry.
- A `provide` that results in no endpoint changes (re-uploading an identical spec) no longer creates a `PROVIDE_SPEC` audit entry. Previously (1.5.1) every non-dry-run provide was audited even when it changed nothing (`+0, ~0, -0`), cluttering the audit timeline. Applies to OpenAPI, AsyncAPI, and gRPC/proto provides.


---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
