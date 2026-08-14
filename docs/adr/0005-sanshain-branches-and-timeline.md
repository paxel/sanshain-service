---
status: accepted
---

# Sanshain-branches: named release graphs on an append-only timeline

ADR-0004 gave the dependency graph a trustworthy **main view** fed by trunk-flagged traffic.
This ADR adds phase 2: **sanshain-branches** — named graphs ("Release Maribou") answering three
questions the main view cannot: *what exactly does a release consist of*, *which releases contain
producer X at version Y* (vulnerability/backport impact), and *what changed between two graphs* —
diffing is first-class, release vs release, release vs main, any graph at any date.

**This does not resurrect ADR-0003's branches.** Those were claims about Producers' git
repositories that Sanshain could never verify. A sanshain-branch is Sanshain's own artifact: a
named lineage of the dependency graph itself, created and updated through the same audited API as
everything else.

## The timeline: last-write-wins is a view rule, not a storage rule

Trunk (and branch) graph writes are **append-only**. "Last write wins" means the newest record
per (client, producer, api type, endpoint) defines the *current* graph; earlier records are never
destroyed. Consequences, all deliberate:

- The main graph is reconstructable **at any past date**; each change is a marker on a timeline
  slider in the UI.
- The phase-1 trunk TTL (`trunk_max_age_days`) *closes* an entry — it leaves the current view
  (after the stale-highlight phase), but stays in history.
- History is **unbounded by default**. Rows are small (ids, version strings, timestamps — spec
  content lives in `spec_versions` regardless); a retention setting can be added later if growth
  ever demands it, whereas culled history is gone forever. No rollback semantics anywhere: the
  past is only ever read.

## Creating a branch

`create(name, source?, as_of_date?)` — releaser permission (`release_ga`), from the admin page or
the release-cut pipeline. `source` defaults to trunk, `as_of_date` to now; branching off an
existing sanshain-branch uses the same call. The branch is born as a copy of the source graph's
state at that date — complete by construction, because the main graph contains every service
regardless of recent build activity.

**Retroactive creation is a first-class path**, not an afterthought: releases are cut from
protected branches and carry no code drift, so trunk-at-cut-date *is* the release, whenever
someone gets around to naming it. A forgotten release day is repaired by picking the date on the
timeline.

## Updating a branch

Release-branch hotfix builds send **`tag=<name>`** with their provides and requires — a second
optional wire parameter next to phase 1's `trunk`, **mutually exclusive** with it (both present
is a `400`): a build belongs to trunk or to a named branch, never both. Updates append to the
branch's own timeline exactly like trunk writes.

An **unknown tag is a `404`** with an instructive message — no auto-create. Creation stays a
deliberate, permissioned act; a typo'd pipeline cannot silently mint a half-empty release graph,
and the failure is recoverable (create the branch retroactively, re-run the build). Names are
unique among live branches; deleting a branch (admin permission, audited — deliberately a higher
bar than creation) frees its name.

## Version lifecycle interaction

- Branch members reference versions **by value** (`service, api_type, version`), never by stored
  row id. Deleting a version leaves a *visibly dangling* reference in every branch that pins it —
  highlighted as such, never silently dropped — and a later re-provide of that number (the
  documented delete-version escape hatch) **heals the reference automatically**. Faithfulness of
  such a rebuild is the producer's reproducibility, not Sanshain's promise.
- The delete-version confirmation lists referencing sanshain-branches alongside pinned Consumers.
  **Warnings, not hard blocks**: EOL cleanup must not become a strict ordering problem.
  **Amended 2026-08-08:** deleting a *Producer or Consumer* referenced by a branch's recorded
  graph is the one hard block (`409` naming the branches). A deleted version stays visible as a
  dangling reference and heals on re-provide, so a warning suffices; a deleted participant
  cascades its rows out of the branch entirely, so a warning the admin clicks through silently
  rewrites a release cut that already happened. Trunk presence does not block.
  **Amended 2026-08-10 (#26B):** pin rows now store the participant name by value and no longer
  cascade on a participant delete, so history survives the delete outright. A participant delete
  closes its open *trunk* pins (it leaves the current main graph; its closed rows stay as timeline
  history). The branch `409` above still stands, so a branch's frozen open state is never altered.
- Snapshot use-based expiry is untouched. A release pinning a snapshot is a smell the graph
  shows; it is not a reason to keep snapshots alive forever.

## Surrounding surfaces

- **Rename**: admin permission (same bar as delete), audited with old and new name. Safe because
  branch identity is the internal id — membership, timeline and audit stamps survive; a pipeline
  still sending the old name gets the instructive `404` until reconfigured, which is correct for
  a name wrong enough to rename. Rename to an existing live name is `409`.
- **Audit**: besides the branch lifecycle operations, provide/require audit entries record their
  declared stream (trunk / tag name / none), and the audit page can filter by it — "who changed
  Release Maribou?" is one query.
- **Reports**: report generation takes an optional graph scope — dev (default, unchanged), main,
  or a branch at an optional date — so a release-scoped architecture/isolation report is the same
  code path as today's.

## Considered and rejected

- **Auto-create on first tagged call** — the branch would be born empty or seeded from the wrong
  point in time, with nobody having chosen the one parameter that makes it meaningful.
- **Hard deletion protection for referenced versions** — turns EOL cleanup into an ordering
  problem and contradicts delete-version's role as the sole escape hatch.
- **A retention window for history from day one** — every culled day is timeline the retroactive
  path can no longer reach; add a knob only when data shows a need.
- **One unified `graph=` parameter with a reserved "trunk" value** — a magic name is a footgun;
  two explicit parameters keep the wire as clean as the model.
