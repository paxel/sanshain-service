
# Sanshain Sync & Concurrency — Problem Analysis

This document catalogs open issues with AsyncAPI semantics and concurrent spec publishing,
along with solution options for each. It is a design exploration — no implementation decisions
are final until approved.

## Problem 1: Provide Should Only Store PUB Operations for AsyncAPI

### Current Behavior

`split_asyncapi()` in `src/asyncapi.rs` extracts **both** PUB and SUB operations from the
AsyncAPI spec and stores all of them as endpoints in the database.

For AsyncAPI v2, lines 28–33 emit one entry per `publish` and one per `subscribe`.
For AsyncAPI v3, lines 48–76 emit one entry per `send` (PUB) and `receive` (SUB) operation.

The `/provide/asyncapi` handler in `src/lib.rs:651` passes everything through to
`provide_spec_inner`, which stores it all.

### Desired Behavior

In Sanshain's model, **the publisher defines the contract**:
- A service that publishes to a topic **provides** the message schema via `/provide/asyncapi`.
- A service that subscribes to a topic **declares the dependency** in its `sanshain.yaml` `requires` section.

This mirrors how REST works: the server provides the API spec, clients declare dependencies.
For AsyncAPI, the publisher is the "server" and subscribers are "clients".

The `/provide/asyncapi` endpoint should therefore **only store PUB operations** and silently
discard (or warn about) SUB operations in the provided spec.

### implementation status
Implemented in v0.13.0 using PUB-only filtering in the application layer.

---

## Problem 2: Multiple Publishers for the Same Topic (Last-Write-Wins)

### Current Behavior

Endpoints are scoped to `(service, branch, api_type, path, method)`. If two different services
(`order-service` and `payment-service`) both provide PUB on channel `orders.created` on the
same branch, they are stored as **separate entries** under their respective services. This is
actually fine — they don't overwrite each other.

The real issue is **semantic**: who owns the contract for `orders.created`? If both define
different schemas for the same topic, subscribers don't know which one is authoritative. And
if someone requires `PUB orders.created` from `order-service`, they get order-service's schema,
even if payment-service defines it differently.

### expected solution
* when a provide to a none protected branch is made, we assume the first provide is the unmodified version from the source branch, master or a release or tag or whatever and the og version
* we store this as the "source" for the branch
* whenever a branch pushes the same as source we accept it
* when producer ALPHA pushes a modified version it MUST be backward compatible with the source otherwise rejected
  * this version is stored as current for the branch together with the owner: ALPHA
* when producer BETA pushes a different modified version it MUST be backward compatible with the current version if the owner is other than BETA or it is rejected
* when producer ALPHA pushes a modified version that is different from current but compatibel with source and the owner is ALPHA it replaces current version.
* when prodcuer ALPHA pushes a unmodified version that is equal to source, the current is removed if the owner is ALPHA.

### implementation status
Implemented in v0.13.0 using `shared_contracts` table and backward-compatibility tracking on non-protected branches.

---

## Problem 3: Concurrent Developers on Same Service/Branch Overwrite Each Other

### Current Behavior

`provide_spec_inner` (in `src/application/services.rs:92`) does a **full diff-and-replace**:
1. Fetches all existing endpoints for the branch.
2. Compares with the new spec.
3. Inserts new endpoints, updates changed ones, deletes missing ones.

There is **no optimistic concurrency control**. If Dev A provides at time T1 and Dev B provides
at time T2, Dev B's spec completely replaces Dev A's changes — even if Dev B was working from
an older version of the spec.

The `apply_spec_changes` call is transactional, but there's no version check to detect the conflict.

### implementation status
Implemented in v0.13.0 using monotonic `spec_version` and optional `base_version` in provide requests.

---

## Problem 4: Provide Returns No Version — Clients Can't Detect State

### Current Behavior

All three provide handlers (`provide`, `provide_asyncapi`, `provide_proto`) in `src/lib.rs`
return `StatusCode::ACCEPTED` (202) with **no response body**. The client has no way to know:
- What version was created
- What the current state hash is
- Whether anything actually changed

The endpoint version history exists (`endpoint_versions` table, `GET /endpoint/versions` API)
but is per-endpoint and read-only — designed for the UI diff viewer, not for client plugins.

### implementation status
Implemented in v0.13.0. All provide endpoints now return `202 Accepted` with a JSON body containing `version`, `content_hash`, and a `changes` summary.

---

## Problem 5: No Client-Side Caching / Skip Mechanism

### Current Behavior

Client plugins must always push the full spec content, even if nothing has changed since the
last successful provide. There is no way to short-circuit.

### implementation status
Implemented in v0.13.0. The server now returns the spec's `content_hash` in the provide response. If a subsequent provide is made with the same content, the server detects the identical hash and skips the database update and version increment, returning the current version.

---

## Cross-Cutting Concern: Require-Side Caching

The same version/hash mechanism can benefit the **require** side:
- `GET /require` and `POST /require-bundle` could return a version/hash in headers.
- Client plugins cache the received spec + hash.
- On next build, client sends `If-None-Match` → gets `304` if spec hasn't changed → uses cached file.
- Reduces build times when upstream specs are stable.

### implementation status
Implemented in v0.13.0. The server returns an `ETag` (SHA-256 hash of the generated content) on all require endpoints. If the client sends an `If-None-Match` header matching the current hash, the server returns `304 Not Modified`.

---

## Summary: Recommended Path

| Problem                     | Recommended Option                 | Effort         | Dependencies  |
|-----------------------------|------------------------------------|----------------|---------------|
| 1. PUB-only provide         | B (filter in service layer)        | Small          | None          |
| 2. Multi-publisher conflict | A (report warnings) → E (advisory) | Small → Medium | Problem 1     |
| 3. Concurrent overwrites    | A (optimistic concurrency)         | Medium         | Problem 4     |
| 4. No version in response   | A (JSON response body)             | Medium         | None          |
| 5. Client-side caching      | D (hash + version cache)           | Medium         | Problem 3 + 4 |

Suggested implementation order: **1 → 4 → 3 → 2 → 5 → 6**
(Problem 1 is independent; Problem 4 enables 3; Problem 3 enables 5; Problem 2 is incremental.)

---

## Open Issues

### Problem 6: Bundle Hash Stability

**Context:** The `ETag` for a bundle is currently a hash of the merged YAML content. The merging process is sensitive to the **order** of endpoints in the request. If a client reorders endpoints in `sanshain.yaml`, the generated YAML (and its hash) may change even if the set of endpoints is identical.

**Goal:** Ensure stable bundle hashes regardless of request order.

### Problem 7: Semantic Versioning for Specs

**Context:** Currently, Sanshain uses monotonic integers for spec versions.

**Goal:** Explore support for semantic versioning (SemVer) provided by the user, or automatic detection of MAJOR/MINOR/PATCH changes based on backward-compatibility analysis.
