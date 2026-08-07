# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.2.0] - 2026-08-06

### Added
- Version strings — `info.version`, the proto `// sanshain-version:` marker, and pinned `version`
  parameters — now accept an optional leading `v` and omitted MINOR/PATCH with implicit zeroes
  (`v2` → `2.0.0`); previously only exact `MAJOR.MINOR.PATCH` was accepted. Stored and answered
  versions stay canonical three-part.
- Version rows on the producers page now offer "View spec": the full stored document opens in the
  viewer with the same version-history sidebar, diff and blame as the endpoint view — previously
  the UI only offered it as a file download.
- When a GA re-provide differs only in bytes the splitter doesn't see (whitespace, line endings,
  comments), the `409` now says so instead of only asking whether you forgot to bump — content
  comparison is and stays byte-for-byte.
- Provides accept an optional `trunk: true` flag (ADR-0004): the version entry is marked as
  trunk's current version — shown as a badge on the producers page — with no effect on version
  rules or stability. Absent flag, nothing changes.
- Requires (and require-bundle) accept the same optional `trunk` flag: the pin is additionally
  recorded in a new append-only trunk store where the last write per endpoint defines the current
  trunk pin set, exposed as `trunk_graph` in the `/report` payload.
- The graph page gained a Main/Dev toggle (ADR-0004): the main view draws producers at their
  trunk version with the current trunk pins as edges, highlights major-lag conflicts, and the
  legend shows only markers the active view can draw.
- Sanshain-branches (ADR-0005): `POST /admin/branches` (releaser-gated, built for the release-cut
  script) creates a named copy of the trunk graph — or another branch — at a chosen instant,
  including retroactively; `GET /admin/branches` lists them and
  `GET /admin/branches/{name}/graph[?at=…]` answers a branch's pin set.
- Provides and requires accept an optional `tag=<branch>` (release-branch hotfixes): the call
  updates that sanshain-branch's graph and member versions instead of trunk. `trunk` and `tag`
  together answer `400`; an unknown tag answers an instructive `404` — no auto-create.
- Sanshain-branches can be renamed (repairing a botched name; membership and timeline survive)
  and deleted (freeing the name) — admin-only, audited, from the admin dashboard's new
  Sanshain-Branches section or `PUT`/`DELETE /admin/branches/{name}`.
- Trunk data ages visibly: a configurable month-scale TTL (`trunk_max_age_days`, default 90)
  closes trunk pins and clears trunk markers not refreshed in time — they leave the main graph
  but stay as history — and the main graph highlights entries as stale (amber, ⚠) once they pass
  half the TTL, so forgotten producers and dead requires surface before they vanish.
- Reports take a graph scope: `?scope=dev` (default, unchanged), `main` (trunk pins), or
  `<branch>[@instant]` — so a release-scoped architecture or isolation report describes what
  production actually is. The reports page gained the matching selector.
- The graph page draws sanshain-branches: pick one from the new branch selector. A pin whose
  version was deleted renders as a dangling reference (dotted orange, own legend entry) and heals
  automatically when the number is re-provided; the delete-version confirmation now also names
  referencing sanshain-branches alongside pinned Consumers.

### Changed
- UI messages and docs no longer explain resolution by contrast with removed 1.x mechanics
  ("fails immediately", "hard-fail", "no fallback and nothing waits"); they now state the
  current behavior plainly (a missing Pin answers 404, a missing endpoint 410).

### Fixed
- The observability page's log copy/download now includes the `[service@version]` context the
  on-screen view shows — previously the export dropped it, leaving provide lines without any
  reference to the producer they concern.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
