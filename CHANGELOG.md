# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.2.0] - 2026-08-06

### Added
- AsyncAPI `subscribe` operations are now harvested on every provide as version-less consumer
  edges in the dependency graph — previously they were dropped with a server-side log the Producer
  never saw. Each resolves to the channel's PUB owner (or shows as pending when none exists yet),
  and is validated as an expectation: reading fewer fields than the contract is fine, but expecting
  a field the contract does not guarantee is rejected with `409` on a GA provide and reported as an
  advisory `harvested_subscriptions` note on a snapshot. See [ADR-0006](docs/adr/0006-asyncapi-subscribe-harvesting.md).
- A Producer can retire an API family it no longer provides —
  `POST /admin/producers/{name}/retire/{api_type}`: clears the messaging/grpc capability tag,
  drops the family from the current main graph (keeping its timeline history), and releases its
  AsyncAPI channel-message contracts for another Producer to claim, without deleting version
  history or breaking existing pins. Meant for the client plugin to call when a service's
  `sanshain.yaml` drops a protocol.
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
- Reports take a graph scope: `?scope=dev` (default, unchanged), `main[@instant]` (trunk pins),
  or `<branch>[@instant]` — so a release-scoped architecture or isolation report describes what
  production actually is. The reports page gained the matching selector.
- The graph page draws sanshain-branches: pick one from the new branch selector. A pin whose
  version was deleted renders as a dangling reference (dotted orange, own legend entry) and heals
  automatically when the number is re-provided; the delete-version confirmation now also names
  referencing sanshain-branches alongside pinned Consumers.
- Provide/require audit entries record their declared stream (`trunk`, a branch tag, or none),
  and the audit timeline filters by it — "who changed Release Maribou, when?" is one query.
- The graph page gained a timeline (ADR-0005): a slider over the change instants of the main
  graph or a branch renders the graph as it was at that date (`GET /admin/trunk/graph?at=…`,
  `…/timeline`), and releasers can "create branch here" — the retroactive release cut.
- Reverse lookup: producer version rows show chips naming every sanshain-branch referencing that
  version (`GET /admin/producers/{name}/branch-memberships`). Graph exports (SVG/PNG/mermaid)
  are stamped with their view, branch and instant; scoped reports carry a `Scope:` line.
- New metrics: `sanshain_branch_updates_total{branch}` and a `sanshain_branches` gauge, with the
  branch count shown on the observability page.
- Graph diffing (`GET /admin/graph/diff?left=…&right=…`, and a Compare panel on the graph page):
  a structured diff between any two graph selections — release vs release, release vs main, any
  at a past instant — naming added/removed services and added/removed/changed pins.

### Changed
- The dependency graph's messaging badge now marks only actual AsyncAPI providers, not every
  consumer of one, and the virtual broker node is named `BROKER` rather than asserting Kafka.
- Outdated pins are split into two tiers with their own colours and toggles — behind within the
  same major, and a whole major behind; previously one patch behind and three majors behind
  looked identical.
- A spec whose paths differ only in a path-parameter name (`/users/{id}` and `/users/{userId}`),
  a trailing slash, or doubled separators is now refused with `400` naming both — OpenAPI forbids
  them as identical and nothing downstream can tell them apart; previously both were stored and a
  Consumer pinning one of them got an arbitrary answer.
- UI messages and docs no longer explain resolution by contrast with removed 1.x mechanics
  ("fails immediately", "hard-fail", "no fallback and nothing waits"); they now state the
  current behavior plainly (a missing Pin answers 404, a missing endpoint 410).

### Fixed
- The observability page's audit panel no longer loses real changes to a burst of rejected
  provides: it reads a wider window and offers a "Hide rejected provides" toggle.
- `LOG_BUFFER_SIZE` now actually bounds how many log messages are kept per level; previously it
  only sized the initial allocation while the retention stayed fixed at 100.
- The observability page's log copy/download now includes the `[service@version]` context the
  on-screen view shows — previously the export dropped it, leaving provide lines without any
  reference to the producer they concern.
- The max-age settings (snapshot, dependency, trunk) reject `days` above 36500 (~100 years) with
  a `400`; previously any number was stored, and an absurd value could crash the cleanup task.
- A dry-run provide of unchanged content no longer refreshes the snapshot's use-based expiry —
  previously a pipeline that only ever dry-ran kept its snapshots alive indefinitely.
- Deleting a Producer or Consumer is refused with `409` while a sanshain-branch's recorded graph
  still references it, naming the branches to retire first. Its trunk history is preserved: the
  pin rows store the participant name by value, so a delete closes the participant's open trunk
  pins (it leaves the current main graph) but the timeline still reconstructs the era it was
  active, instead of the rows being erased.

### Security
- The Content-Security-Policy now sets `script-src 'self'` (previously `'unsafe-inline'` plus two
  CDN hosts) and adds `frame-ancestors 'none'`: all UI behaviour moved off inline event handlers
  onto a delegated dispatcher, so a stored/reflected HTML injection can no longer run inline
  script. `style-src` still allows `'unsafe-inline'` because the diagram library injects styles at
  render time.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
