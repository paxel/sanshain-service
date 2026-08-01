# Sanshain

Sanshain is a registry for API specifications. **Producers** publish their full spec for a branch;
**Consumers** require only the endpoints they need. This document is the shared vocabulary — in
particular, how a request against a branch is answered.

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

### Publishing and requiring

**Provide**:
A Producer's submission of a complete spec for one branch. Because it is complete, an endpoint
absent from it is absent from that branch's API.
_Avoid_: push, upload, publish

**Require**:
A Consumer's request for specific endpoints, which also records the dependency.
_Avoid_: fetch, pull, consume

**Onboarding**:
A Producer lifecycle state in which breaking changes are accepted rather than gatekept. A Producer
whose API is not yet stable passes through it; nothing about its branches changes, only whether
Sanshain refuses what they publish.
_Avoid_: dev mode, grace period, unprotected

**Pending spec**:
A Provide that was held for review instead of applied, retained in full so it can be judged on its
content. Not part of any branch's API until accepted — until then the branch answers as if it had
never been submitted.
_Avoid_: draft, queued spec, rejected spec

**Author**:
Consumer-supplied, unverified attribution for who wrote a change, used for blame display only.
_Avoid_: user, committer

**Actor**:
The authenticated caller behind a request. Always what the audit log records; never overridable.
_Avoid_: user, author

### Branches

**Branch**:
A named line of a Producer's API, mirroring the VCS branch its spec was built from.
_Avoid_: version, environment

**Protected branch**:
A branch matched by a protected-branch pattern (`*` and `?` are wildcards; everything else,
including `_`, is literal). Protected branches keep version history, reject breaking changes, and
are never culled by stale-branch cleanup.
_Avoid_: main branch, stable branch, release branch

**Authoritative branch**:
A branch that has published at least one spec. Its spec is the complete statement of that branch's
API, so an endpoint missing from it is missing *by choice*. A branch that has never published is
not authoritative and inherits instead.
_Avoid_: owning branch, primary branch

**Source protected branch**:
The protected branch a branch descends from and defers to when it has nothing of its own — e.g.
the release line a hotfix was cut from. Sticky: the first caller to supply it wins, and only an
admin may change it afterwards.
_Avoid_: parent branch, base branch, target branch

**Pull-from branch**:
A one-shot, non-persisted override naming the exact branch a single request must resolve against,
bypassing all inheritance.
_Avoid_: override branch, forced branch

### Answering a request

**Resolution**:
Answering "what is this endpoint on this branch?" for a given Producer, branch, API type, path and
method. Every resolution yields exactly one resolution state, and names the branch it came from.
_Avoid_: lookup, fallback (fallback is only one possible outcome)

**Published**:
Resolution state: the requested branch is authoritative and published this endpoint.

**Absent**:
Resolution state: the requested branch is authoritative and did not publish this endpoint, so it is
deliberately not part of that branch's API. A definitive "no" — resolution stops and must not
inherit an ancestor's version.
_Avoid_: not found, missing, deleted

**Inherited**:
Resolution state: the requested branch has never published, so the answer comes from another
branch, which is always named in the response.
_Avoid_: fallback, defaulted

**Unknown**:
Resolution state: no branch of the Producer has this endpoint. Unlike Absent it may appear later,
so it is the only state a long-poll waits on.
_Avoid_: not found, 404

## Note on storage

The physical schema still calls these `services` and `clients` — tables, and the columns that
reference them (`service_id`, `client_id`). That is deliberate: renaming them buys nothing and costs
a migration against live data. **Producer maps to `services`, Consumer maps to `clients`.**

The translation is confined to the repository layer, which is where the SQL already lives, so
identifiers that name a database column keep the database's word even in Rust. Everything above that
layer — wire parameters, routes, domain types, the UI, these docs — uses Producer and Consumer.
