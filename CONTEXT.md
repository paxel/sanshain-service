# Sanshain

Sanshain is a registry for API specifications. **Producers** publish versioned specs; **Consumers**
pin the exact version they build against. This document is the shared vocabulary — in particular,
how a request for a pinned version is answered.

## Language

### Who is who

**Sanshain**:
This registry itself — the server that stores and distributes specs. Where a longer name is
needed, *Sanshain Service*.
_Avoid_: the service (that word names a role below), the server, the backend

**Producer**:
A system that publishes an API spec to Sanshain. Identified on the wire by `producername`.
_Avoid_: service, provider, publisher, API owner

**Consumer**:
A system that requires endpoints from a Producer, and is recorded as depending on them.
Identified on the wire by `consumername`.
_Avoid_: client, subscriber, requester

**Tooling**:
Software that speaks HTTP to Sanshain on a Producer's or Consumer's behalf — the Maven plugin,
CLIs, build integrations. Never a domain entity: tooling acts *for* a Producer or Consumer.
_Avoid_: client, Sanshain client

> Producer and Consumer are **roles, not entity types**. One system is usually both: it produces
> its own spec and consumes others'. Sanshain records the two roles separately, so such a system
> appears once in each list.

### Who may do what

**Permission**:
The unit an authorisation check tests — a single thing an Actor may do. Checks name permissions,
never roles, so what a route requires stays readable without knowing who holds what.
_Avoid_: right, privilege, capability

**Role**:
A named bundle of permissions, held instance-wide. Fixed: the set of roles is part of the product,
not something an operator composes.
_Avoid_: profile, permission set, access level (also: not a Producer/Consumer role — those are
positions in an exchange, not grants)

**Group**:
A set of users that roles and maintainer scopes attach to. Membership either originates in the
directory Sanshain authenticates against, or is Sanshain's own; a group always knows which.
_Avoid_: team, org unit

**Maintainer**:
An Actor responsible for a set of Producers, holding powers over those and no others. Scoped by
definition — unlike a role, it means nothing without the Producers it is over.
_Avoid_: owner, admin, service owner

**Root**:
The Actor whose authority comes from configuration rather than from stored grants, and which
therefore cannot be revoked from inside Sanshain. Holds every permission, including permissions
that do not exist yet.
_Avoid_: superuser, owner, initial admin

### Versions

**Version**:
The Producer-declared identity of a spec: strict `MAJOR.MINOR.PATCH`, read from the spec file
itself (`info.version`; for proto, a mandatory `// sanshain-version:` comment). Never assigned by
Sanshain, never a suffix-carrying string.
_Avoid_: branch, revision, baseVersion, Instance version (that is Sanshain's own build)

**Version line**:
The ordered set of Versions one Producer has published for one API type. All history, diffing,
compatibility checking and cleanup happen within a single line — a Producer's OpenAPI and proto
lines evolve independently.
_Avoid_: branch, timeline

**Stability**:
Which of exactly two states a stored Version is in — Snapshot or GA. Declared by the caller on
every Provide; how the caller decides (typically from its VCS branch) is Tooling's business and
never Sanshain's.
_Avoid_: channel (an AsyncAPI term here), maturity, branch type

**Snapshot**:
A Version in its mutable state: re-providing it overwrites it, last writer wins, and it expires
once neither provided nor required for the configured window. The same entry in the line until it
is promoted or gone.
_Avoid_: draft, pre-release, feature version, WIP version

**GA**:
A Version in its immutable state. Re-providing identical content is a no-op; different content is
rejected with a proposed next Version. A GA permanently claims its number: no Snapshot may ever
exist for it again, and it is never removed by cleanup.
_Avoid_: release, stable, final

**Promotion**:
The GA Provide of a number that existed as a Snapshot: the entry becomes GA in place and the
Snapshot content is gone. Only the same-numbered Snapshot is affected.
_Avoid_: release (as a verb), publish

**Instance version**:
Sanshain's own build version, reported by `/version`. Always qualified as *instance* version to
keep it apart from spec Versions.
_Avoid_: version (unqualified)

### Publishing and requiring

**Provide**:
A Producer's submission of a complete spec for one Version of one API type, with declared
Stability. Because it is complete, an endpoint absent from it is absent from that Version's API.
_Avoid_: push, upload, publish

**Require**:
A Consumer's request for specific endpoints at its Pin, which also records the dependency —
but only when it succeeds. Resolution never creates Producers, Versions or dependencies.
_Avoid_: fetch, pull, consume

**Pin**:
A Consumer's exact Version choice for one dependency, written in its own configuration. There are
no ranges, no "latest", and no default: what a Consumer builds against changes only when someone
edits the Pin.
_Avoid_: requirement, target version, constraint

**Actor**:
The authenticated caller behind a request. Always what the audit log records; never overridable.
The Actor is also the attribution shown on a Version — the last provider — with one exception:
a Promotion carrying byte-identical content keeps the Snapshot's provider, so the person who
built it stays on the released Version.
_Avoid_: user, author (a client-supplied author hint no longer exists — the token defines the user)

### Answering a request

**Resolution**:
Answering "what is this endpoint at this Pin?" for a given Producer, API type, Version, path and
method. GA is preferred, the Snapshot is the only alternative, and there is no fallback beyond
that. Every resolution yields exactly one resolution state and names the Stability it was served
from.
_Avoid_: lookup, fallback

**Served**:
Resolution state: the pinned Version exists and contains the endpoint. The response says whether
GA or Snapshot answered.
_Avoid_: published, resolved, hit

**Absent**:
Resolution state: the pinned Version exists and did not include this endpoint, so it is
deliberately not part of that Version's API. A definitive "no" — nothing else is consulted.
_Avoid_: not found, missing, deleted

**Unknown**:
Resolution state: the Producer's line has no such Version in either Stability. A configuration
error on the Consumer's side, answered immediately — nothing waits for a Version to appear.
_Avoid_: not found, pending

**Outdated**:
A dependency whose Pin is semantically below the latest GA of its line. A display state, never a
resolution input: being Outdated changes nothing about what is served.
_Avoid_: stale, behind, deprecated

**Snapshot-pinned**:
A dependency currently served from a Snapshot because its Pin has no GA. Distinct from Outdated —
such a Pin may even be ahead of the latest GA; what it flags is building against overwritable
content.
_Avoid_: unstable, floating

## Note on storage

The physical schema still calls these `services` and `clients` — tables, and the columns that
reference them (`service_id`, `client_id`). That is deliberate: renaming them buys nothing and costs
a migration against live data. **Producer maps to `services`, Consumer maps to `clients`.**

The translation is confined to the repository layer, which is where the SQL already lives, so
identifiers that name a database column keep the database's word even in Rust. Everything above that
layer — wire parameters, routes, domain types, the UI, these docs — uses Producer and Consumer.
