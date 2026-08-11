---
status: accepted
---

# AsyncAPI subscribe harvesting: consumer edges from provides, expectations validated at GA

Today a Producer that provides an AsyncAPI document with `subscribe` operations has them silently
dropped — `parse_spec_endpoints` logs a server-side warning the Producer never sees ("declare them
as requires in `sanshain.yaml` instead") and moves on. Consumer relationships reach the dependency
graph only when a team remembers to hand-write the require. The KAFKA virtual node and the graph
miss consumer edges, and drift between a consumer's local schema copy and the producer's contract
is never detected.

This ADR settles how Sanshain harvests those subscriptions. It re-scopes
[ai/improvements.md #6](../../ai/improvements.md) to the post-ADR-0003 world (versions, not
branches; contracts keyed `(channel, message-name)` with no branch; GA-only contract registration
from [ADR-0004](0004-trunk-flag-and-graph-views.md) / item #20). No code lands with this ADR — it
is the design pass the item was gated on.

## The model this rests on: PUB is truth, SUB is expectation

Decided already and unchanged here: **only a `publish` operation ever registers a contract; a
`subscribe` operation is a declaration of expectation.** A consumer changing what it reads breaks
nobody downstream, so a SUB is never authoritative and is never protected. Storing SUB operations
as a second, competing per-channel schema (the producer's PUB plus each consumer's copy under keys
nothing unifies) was rejected. A harvested SUB becomes a **require** — the same thing a hand-written
`sanshain.yaml` require is — validated against the producer's PUB contract, never a source of truth.

## A harvested subscription is a version-less edge, not a version pin

The require/pin stores are **version-keyed**: a manual `/require` supplies an exact producer
version, and `get_report` reads that version off the dependency row straight into the graph edge. A
`subscribe` operation carries no version, and the channel-message contract it validates against is
itself version-less (keyed `(channel, message-name)`, last GA wins). Forcing a version onto a
subscription — the producer's "current" version, say — would invent a fact the document never
states and silently re-point every time the producer releases. And a subscription whose producer
does not exist yet has no version *to* point at.

So a harvested subscription is modelled as its own thing: **a version-less consumer edge**, stored
in a dedicated `harvested_subscriptions` table keyed `(consumer, channel, message-name)`, recording
the resolved owner (`owner_service_id`, or null when no GA PUB provider exists yet — the unfulfilled
case, kept visible rather than hidden). Owner name is stored **by value** (ADR-0005 convention), so
a deleted owner dangles visibly instead of vanishing. Because harvested edges live in their own
store, they are inherently distinct from hand-declared requires — there is nothing to tag or
reconcile against in the pin tables, and manual requires are untouched by construction.

This is a deliberate departure from the first draft of this ADR, which said harvest would reuse the
`/require` application path and leave `graph.js` unchanged. It cannot: the require path hard-fails on
a missing producer (404/410) and pins a version a subscription doesn't have. The report and the
graph instead learn to emit a version-less consumer edge from the harvested store.

## Harvest on every provide, carrying the provide's trunk-ness

On **every** AsyncAPI provide (snapshot or GA), each `subscribe` operation in the submitted document
is harvested: resolve its owner against the **GA** contract store and record/refresh a
`harvested_subscriptions` row for the providing service. The store is append-only in the ADR-0005
sense — the open row (`valid_to IS NULL`) per `(consumer, channel, message-name)` is current, a
retracted subscription is closed on the next provide, and closed rows are the timeline. Manual
requires in the pin stores are never touched.

Harvesting on every provide — not GA-only — is deliberate: the item exists to make consumer edges
**visible**, and GA-only would hide a service's consumption for its entire pre-release life, moving
the silent miss later instead of fixing it. What makes "every provide" safe rather than noisy is
that **a harvested edge inherits the trunk-ness of the provide that produced it** (a `trunk` flag on
the row):

- A `trunk: true` provide's harvested edges appear in the main graph.
- A plain snapshot provide's harvested edges appear only in the dev view, which is exactly where
  development churn belongs (ADR-0004).

So the main graph stays quiet; only the dev view flickers with CI activity, as designed.

**Harvested requires are a provide-owned set.** They are tagged as harvested and distinguishable
from requires a team declares by hand. Each AsyncAPI provide reconciles that subset wholesale — a
subscription dropped from the document retracts its edge on the next provide — while
manually-declared requires are never touched. The two never clobber each other.

## Drift validation blocks at GA, is advisory on snapshots

When a PUB contract exists for `(channel, message-name)`, the consumer's subscribe payload is
checked against it by `check_expectation_satisfied(contract, expectation)` — the role-swapped
sibling of `check_schema_compatible`. A consumer expecting **less** than the contract guarantees is
fine; a consumer expecting a property, or a type, the contract does **not** guarantee is drift.

Drift is enforced asymmetrically by provide kind:

- On the consumer's **GA provide**, drift is a blocking **`409`** naming channel, message, and the
  offending property. This is the one authoritative moment — no service ships a GA spec whose SUB
  expectations contradict a producer's GA contract.
- On a **snapshot provide**, drift is recorded and surfaced (see below) but does **not** block, so
  development iterates freely against expectations that are not yet satisfiable.

The failure mode only ever affects the **consumer submitting the spec** — it is the consumer's own
provide that is rejected, for its own inconsistency. This bounds the cross-service coupling: a
producer's breaking GA change can block a consumer only at the consumer's next GA cut, never
mid-development. Pure-blocking (409 on every provide) was rejected for wedging every snapshot CI run
on other services' current GA contracts; pure-advisory was rejected for letting drift accumulate
silently. GA-only blocking matches how #20 already treats GA as the authoritative line.

## The perspective convention, stated loudly

Sanshain reads AsyncAPI 2.x `publish` / `subscribe` from the **application's** perspective:
`publish` means *this service publishes*. The official 2.x specification defines those keywords from
the **client's** perspective (inverted). Sanshain deliberately uses the application-perspective
reading, which matches its AsyncAPI 3.x `send` / `receive` mapping. This must be documented
prominently — in `docs/sanshain-yaml.md`, the client-plugin docs, and the provide response — because
a Producer who assumes the spec-literal reading will harvest exactly backwards.

## Surface

The AsyncAPI provide response gains a `harvested_subscriptions` list — each entry a channel and
message name with its resolved/missing status and, when a contract exists, its drift status —
replacing today's log-only warning. `api.yaml` is updated to match. This is what makes the harvest
visible to the Producer at the moment they provide, which the current server-side log never was.

## What this does not do

- It does not store SUB operations as endpoints or contracts. The only artifact is a version-less
  consumer edge in `harvested_subscriptions`.
- It does not harvest on `tag` (release-branch) provides — only trunk/dev. Branch graphs are cut
  artifacts (ADR-0005), not a place development subscriptions accrue.
- It does not add branch mechanics. Resolution is by `(channel, message-name)` against the GA
  contract store; there is no protected-branch fallback (there are no branches).
- It does not reuse the `/require` application path, and it does change the report/graph: the
  version-less consumer edge is a new shape the KAFKA view learns to render (see the version-less
  edge section above — this supersedes an earlier "graph.js unchanged" intent).

## Consequences

- Consumer edges appear automatically from AsyncAPI provides; teams no longer have to remember to
  hand-declare a require for something their own spec already states.
- The dependency graph and KAFKA node stop silently under-counting consumption.
- A consumer whose schema copy drifts from the producer's contract learns at its next GA cut,
  precisely, instead of never.
- Harvested and manual requires coexist for the same service; the reconcile is per-provide and
  scoped to the harvested subset, so neither erases the other.
