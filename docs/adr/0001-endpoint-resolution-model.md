---
status: accepted
---

# A branch that has published owns its API; only a branch that never published inherits

A `/provide` submits a **complete** spec for one branch. So once a branch has published anything,
its spec is the whole truth about that branch: an endpoint missing from it is missing *by choice*,
and Sanshain answers `Absent` (410 on `/require`) rather than searching for the endpoint on a
protected branch. Only a branch that has **never** published inherits, and every response names the
branch it was actually served from.

## Why

The previous resolver fell back to a protected branch whenever a non-protected branch lacked an
endpoint, without asking whether that branch had published at all. It therefore could not tell
"this branch has no API" apart from "this branch has an API and deliberately dropped this
endpoint". A developer who deleted a deprecated endpoint on a feature branch still received it from
`master` — in the UI *and in their build* — so the deletion was silently undone.

The evidence was also being destroyed at exactly the wrong place. `soft_delete: is_protected` kept a
tombstone on protected branches (which never fall back) and hard-deleted the row on feature branches
(which do), leaving nothing to distinguish a deliberate removal from an endpoint that never existed.
Defining authority by "has published" needs no tombstone: absence *is* the answer.

## Considered and rejected

- **Tombstones on every branch.** Would make deliberate removal explicit, but a branch's *first*
  publish that simply omits an endpoint records no delete, so it would still silently inherit.
- **Per-API-type authority** (a branch authoritative only for types it published). More precise for
  mixed-protocol producers, but multiplies the states to reason about. Branch-level was chosen; the
  known edge is a build that pushes OpenAPI and fails before pushing proto, leaving proto `Absent`.
- **A deprecation window** — warn while still inheriting, enforce a release later. Unnecessary: no
  Consumer requires APIs through the tooling yet, so there is nothing to break.

## Consequences

- `Absent` **fails fast** instead of long-polling. A long-poll exists for the race where a Producer
  has not published yet; if it *has* published, waiting cannot change the answer.
- `410 Gone` distinguishes `Absent` from `404` `Unknown`, so tooling can tell "this branch dropped
  it" from "nobody has it" without parsing a body.
- Reads must not create branches. `list_service_endpoints` called `ensure_service`/`ensure_branch`,
  so merely browsing a mistyped branch minted a permanent empty one — which would then be
  non-authoritative and inherit, manufacturing the very state this model reasons about.
