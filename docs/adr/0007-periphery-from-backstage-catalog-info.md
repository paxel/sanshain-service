---
status: accepted
---

# The graph's periphery comes from each system's Backstage catalog-info.yaml

The network graph knows only Producers and Consumers (and, until 2.4.0, a virtual BROKER node).
Nothing outside Sanshain — the databases, buckets and queues a system depends on — has a node, so the
graph cannot show that two systems share a database. Architects asked for Backstage support to
describe this periphery (#46).

## Data flows Backstage → Sanshain, through the build

Each system's build pushes its own `catalog-info.yaml` (`PUT /catalog/{systemname}`, optional,
pointed to by `catalogInfo` in `sanshain.yaml`). Sanshain does not poll a Backstage instance and
exports nothing to Backstage: no instance is required, and the file arrives at the moment the
system is built, alongside its provides and requires. A separate endpoint, rather than a field on
`provide` or `require`, because pure Consumers never provide and `require` is called once per
endpoint.

## Only `resource:` refs in the system's own `dependsOn`

Sanshain reads the `Component` whose `metadata.name` equals the system name, and of it only
`spec.dependsOn`; only `resource:` refs become **Peripheral Resources**. A file without that
Component is rejected (`400`) so a name mismatch fails the build instead of silently showing
nothing.

`consumesApis` and `component:` refs are ignored on purpose. Edges between systems are Sanshain's
own — recorded by `require`, versioned and pinned. Reading them from Backstage too would draw the
same connection twice, and matching Backstage names (`orders-api`) to Sanshain names
(`orders-service`) needs either a fragile heuristic or a mapping every team maintains. A SaaS that
should appear in the graph is modelled as a Resource.

Refs follow Backstage's string-reference form `[kind:][namespace/]name`. A ref without a kind is
read as `resource` (the only kind drawn); a ref without a namespace takes the Component's own
namespace, `default` when it declares none — Backstage's rule. Refs are lower-cased, because
Backstage compares them case-insensitively and a shared resource must merge into one node: the
full ref is the node's identity.

## Storage: append-only, per track, keyed by value

`peripheral_dependencies` follows ADR-0005: the open row per (track, branch, system, resource) is
current, a push closes what is gone and opens what is new, and closed rows are the timeline.

- **Tracks follow the trunk flag (ADR-0004).** A trunk push replaces the `trunk` set *and* the
  `dev` set — the dev view is all activity, and a system built only on trunk must not lose its
  periphery there. A plain push replaces only `dev`. One row with a trunk flag (as harvested
  subscriptions use) was rejected: a later non-trunk push would drop the resource from the main
  graph.
- **Sanshain-branches get a copy at the cut**, as of the cut's instant (retroactive cuts
  included), and from a source branch when branching off one. There is no `tag=` on the push; a
  release's periphery is what trunk had when it was cut.
- **Keyed by system name, by value, without a foreign key.** A pure Consumer may push before it
  ever requires anything, and a branch copy must survive a later participant delete (#26B).
- **Participant deletes close, never delete.** When a Producer or Consumer delete leaves no
  participant of that name, its `dev` and `trunk` rows are closed so no stray node remains.
  Branch copies are untouched.
- **No TTL.** Without a push the last state stays; resources change rarely, and an expiry would
  need its own setting and stale-highlight phase.

## Surface

- `/report` carries `peripheral_dependencies` for its scope; `GET /admin/periphery?scope=` answers
  the same for `dev`, `main[@instant]` and `<branch>[@instant]`, which the graph's main, branch and
  timeline views fetch. A past instant always gets the periphery as it was then, never today's.
- The graph diff reports `periphery_added` / `periphery_removed`; periphery changes are timeline
  instants.
- The graph draws a Peripheral Resource as its own node kind with dashed edges, with a switch to
  hide them.

## Deliberately not done (yet)

Owner, description, lifecycle and links on system nodes; `System`/`Domain` grouping (a Producer's
`domain` already groups); a node symbol by `spec.type`. Each can be added later without changing
the push format.

## Amendment (2026-10-15): a mismatch warns instead of failing the build

The `400` for a file Sanshain cannot read as the system's Component is withdrawn. The periphery is
informational and must never fail a build. Invalid YAML, a document that is no Backstage entity, no
or more than one matching Component, and a malformed `dependsOn` now answer **`200` and record
nothing** — the system's previous periphery stays exactly as it was, rather than being cleared as
if the file had declared none. The body keeps the push response's shape with `resources: []` and
`recorded: false` (a recorded push answers `recorded: true`), and a `Sanshain-Warning` header
carries the reason. A body of the wrong shape (missing `catalog_info_yaml`, unknown fields) stays
`422`; authentication, permission and size limits keep their status codes. Teams that want a
mismatch to fail the build turn on **strict mode** in their client, which fails the command on any
`Sanshain-Warning`.

## Amendment (2026-10-17): owner, description, source location and links are read too

Decision 14 ("only the periphery: no owner, description, lifecycle or links on system nodes") is
relaxed (#48): of the same Component, Sanshain now also reads `spec.owner`, `metadata.description`,
the `backstage.io/source-location` annotation and `metadata.links`, and shows them on the system's
node panel and on its Producer and Consumer pages. Nothing else is read; nothing becomes a node.
The context is stored once per system (`system_context`), replaced on every push and removed when
the system has no role left — a present-day fact about the system, not a graph fact, so it is not
tracked per track or copied into sanshain-branches, and an unchanged push is no event.
