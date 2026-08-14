---
status: accepted
---

# A trunk flag distinguishes the main graph from development noise

Sanshain 2.x records dependencies as `(client, pinned spec version, endpoint)` with the Consumer
as one flat identity. The dependency graph is therefore the union of every pin any build of any
branch happened to record recently, thinned by age-based expiry — it cannot distinguish the pins
of a release stream from development churn, and edges flicker with CI activity.

ADR-0003 removed branches deliberately; this ADR does **not** bring them back. Instead, clients
may declare a single, branch-independent boolean with their calls: **`trunk=true`**
(`sanshain.trunk=true` in `sanshain.yaml`, typically set by CI on trunk pipelines only).

- **On a Provide**, `trunk` is stored as an attribute of the version-line entry: "this version is
  what trunk currently produces" — as opposed to, say, a GA backport provided from a release
  branch. It changes nothing about version rules: version declaration, stability, GA immutability
  and the mislabel checks are untouched.
- **On a Require**, a trunk-flagged pin is recorded **last-writer-wins per
  (client, producer, api type, endpoint)**: the newest record defines the current trunk pin.
  Last-writer-wins is a *view* rule, not a storage rule — writes append, history is preserved
  (ADR-0005 builds its timeline on exactly this).

The graph page then offers two views:

- **Main graph**: producers at their newest trunk-flagged version, edges from trunk pins.
  Deterministic after every trunk build. Conflicts are highlighted — e.g. a consumer whose trunk
  pin lags the producer's trunk version by a major.
- **Dev graph**: today's view, unchanged — latest recorded activity including snapshots, culled by
  `dependency_max_age_days`. Snapshot stability already marks what is in flux.

## Trunk data expires slowly, and visibly

Trunk rows do not live forever: a producer that is quietly scrapped, or a consumer that keeps a
require it no longer uses, is a *forgotten thing*, and the graph's job is to show that rather
than present stale truth as current. Trunk provides and requires therefore carry a last-seen
timestamp with their own month-scale, configurable TTL (`trunk_max_age_days`, independent of the
much shorter dev expiry). Entries approaching the TTL are highlighted as stale in the main graph
first; only then are they culled.

## Considered and rejected

- **Consumer-declared branch context per require.** Fails for trunk-based workflows where release
  branches are cut but never rebuilt — the release's requires would simply never happen.
- **Version-to-version edges (consumer sends its own version) as implicit grouping.** A worked
  example showed the release *set* is not deducible from pairwise version edges once lines move
  at different speeds; the grouping fact lives outside Sanshain (e.g. in a Helm chart).
- **Server-side snapshot/freeze of the live graph.** Solves release naming but not the primary
  problem (a trustworthy main graph); parked instead of rejected — see below.

## Phase 2: named release graphs

Designed in [ADR-0005](0005-sanshain-branches-and-timeline.md): sanshain-branches created from
the main graph (at any past date — the append-only storage above is what makes that possible)
and updated by `tag=<name>`-flagged hotfix builds.
