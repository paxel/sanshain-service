
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

### Options

| Option                                | Description                                                                                                                                            | Pros                                                                                    | Cons                                                                                       |
|---------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------|--------------------------------------------------------------------------------------------|
| **A: Filter in `split_asyncapi`**     | Only emit `AsyncApiSpec` entries where `operation == "PUB"`. Skip SUB entirely.                                                                        | Simple, single change point. Benchmarks/tests stay fast.                                | Callers can't distinguish "no channels" from "only SUB channels". Harder to give feedback. |
| **B: Filter in `provide_spec_inner`** | `split_asyncapi` still emits both. `provide_spec_inner` filters to PUB-only for `ApiType::AsyncApi` before storing. Logs a warning for discarded SUBs. | Better observability — logs show what was ignored. Splitting logic stays spec-faithful. | Two places to reason about the semantics (splitter + service).                             |
| **C: Return structured result**       | `split_asyncapi` returns a struct with `publish: Vec<…>` and `subscribe: Vec<…>`. `provide_spec_inner` uses only `publish`, but can report stats.      | Most flexible; future uses can access both.                                             | More refactoring; changes the splitter's public API.                                       |

**Recommendation:** Option B — minimal change, good logging.

### Documentation Impact

- `docs/sanshain-yaml.md` must clarify that `provides` with `apiType: asyncapi` only registers
  PUB channels. SUB channels must be declared as `requires` entries.
- The "How Matching Works" section needs a callout box explaining the publisher-defines-contract model.
- `api.yaml` description for `/provide/asyncapi` must state PUB-only semantics.

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

### Options

| Option                                                          | Description                                                                                                                                                       | Pros                                                           | Cons                                                                                       |
|-----------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------|----------------------------------------------------------------|--------------------------------------------------------------------------------------------|
| **A: Detect & warn in reports**                                 | The dependency report flags channels published by >1 service as "conflicting publishers". No enforcement at provide time.                                         | Non-breaking. Gives visibility. Teams can resolve organically. | Doesn't prevent the problem.                                                               |
| **B: Detect & reject at provide time**                          | When a service provides PUB for a channel that another service already publishes on the same branch, reject with 409. First publisher owns the channel.           | Strong enforcement. Clear ownership.                           | Too rigid — some architectures legitimately have multiple publishers per topic.            |
| **C: Ownership registry**                                       | Add a channel ownership concept. First publisher auto-owns; can be transferred via admin. Other publishers are rejected unless ownership is shared.               | Flexible. Explicit.                                            | Significant new feature; adds complexity.                                                  |
| **D: Allow multiple publishers, validate schema compatibility** | Allow multiple publishers but check that their message schemas are structurally compatible (like backward compatibility for REST). Reject if schemas conflict.    | Handles legitimate multi-publisher scenarios.                  | Complex to implement. Schema compatibility for AsyncAPI messages is a non-trivial problem. |
| **E: Allow with advisory warnings**                             | Allow multiple publishers. Return a warning header or response field when a topic is already published by another service. Client plugin can surface the warning. | Low friction. Developers are informed but not blocked.         | Easy to ignore.                                                                            |

**Recommendation:** Option A (reports) as a first step, Option E (advisory warnings) as a quick follow-up.

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

### Options

| Option                                          | Description                                                                                                                                                                                                                                        | Pros                                                    | Cons                                                                              |
|-------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------|-----------------------------------------------------------------------------------|
| **A: Optimistic concurrency with spec version** | Add a monotonic `spec_version` counter per `(service, branch)`. Provide accepts an optional `base_version`. If supplied and doesn't match current version, reject with 409 ("outdated"). If omitted, accept unconditionally (backward compatible). | Standard pattern. Backward compatible. Clear semantics. | Requires schema migration. Clients must track version.                            |
| **B: Content-hash based**                       | Compute SHA-256 of the stored spec state. Client sends `base_hash` with provide. Server rejects if hash doesn't match.                                                                                                                             | No counter to manage. Hash is self-describing.          | Hash computation adds overhead. Harder to reason about "version N".               |
| **C: Both version + hash**                      | Return both a version number and a content hash. Client can use either for conflict detection.                                                                                                                                                     | Most flexible. Version for ordering, hash for caching.  | More complexity in API contract.                                                  |
| **D: Branch-level lock**                        | Add a short-lived advisory lock per `(service, branch)`. `provide` acquires it; if already held, reject with 423 Locked.                                                                                                                           | Simple conceptually. Prevents concurrent writes.        | Doesn't solve the "working from stale data" problem. Lock timeout/cleanup needed. |

**Recommendation:** Option A — it's the standard approach and backward compatible.

### Client Flow with Option A

```
1. Client calls POST /provide with base_version=9
2. Server checks: current version for (service, branch) is 9? 
   → Yes: accept, bump to version 10, return {version: 10}
   → No (current is 10): reject 409 "Outdated: your base version 9, current is 10. Pull latest changes."
3. Client without base_version (legacy): always accepted, version bumped.
```

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

### Options

| Option                                              | Description                                                                                                                      | Pros                                                                                                    | Cons                                                     |
|-----------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------|----------------------------------------------------------|
| **A: JSON response body**                           | Change provide response from bare 202 to `202 + JSON body` with `{version, content_hash, changes: {inserts, updates, deletes}}`. | Rich info. Clients can cache version + hash. Backward compatible (clients that ignore body still work). | Slightly larger response.                                |
| **B: Response headers only**                        | Add `X-Spec-Version` and `ETag` headers. Body stays empty.                                                                       | Minimal API change. HTTP-idiomatic.                                                                     | Harder for simple clients (curl) to parse. Easy to miss. |
| **C: JSON body for non-dry-run, 204 for no-change** | Return `202 + JSON` when spec changed, `204 No Content` when nothing changed, `200 + JSON` for dry-run results.                  | Semantic HTTP status codes. Clients can skip processing on 204.                                         | More status codes to handle.                             |

**Recommendation:** Option A — simple, backward compatible, gives clients everything they need.

### Proposed Response Schema (Option A)

```json
{
  "version": 10,
  "content_hash": "sha256:abc123...",
  "changes": {
    "inserts": 2,
    "updates": 1,
    "deletes": 0
  }
}
```

---

## Problem 5: No Client-Side Caching / Skip Mechanism

### Current Behavior

Client plugins must always push the full spec content, even if nothing has changed since the
last successful provide. There is no way to short-circuit.

### Options

| Option                                              | Description                                                                                                                                                                                                                                          | Pros                                                          | Cons                                                                                       |
|-----------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------|--------------------------------------------------------------------------------------------|
| **A: Client-side file hash + server version cache** | After a successful provide, client stores `{file_hash, version}` locally (e.g., `.sanshain-cache`). On next build: compute file hash → if unchanged AND stored version matches, skip provide. If file changed, provide with `base_version`.          | Fast local check. Reduces unnecessary network calls.          | Requires client plugin changes. Cache file must be gitignored.                             |
| **B: Conditional provide (If-None-Match)**          | Client sends `If-None-Match: <content_hash>` header. Server computes hash of current stored spec. If match → `304 Not Modified`. If no match → process normally.                                                                                     | HTTP-standard pattern. No client-side state needed.           | Server must compute hash on every request (can cache). Full spec still sent over the wire. |
| **C: HEAD endpoint for version check**              | Add `HEAD /provide` that returns current version + hash in headers without accepting a body. Client checks first, then decides whether to POST.                                                                                                      | Two-step but clean. No wasted bandwidth.                      | Extra round-trip.                                                                          |
| **D: Combine A + server response**                  | Server returns `{version, content_hash}` on provide (Problem 4 solution). Client caches this. On next run, client hashes the local file: if hash matches cached `content_hash`, skip entirely. If different, provide with `base_version` from cache. | Best UX. Minimal network traffic. Handles concurrent changes. | Requires both server and client changes.                                                   |

**Recommendation:** Option D — it naturally falls out of solving Problem 4 and Problem 3.

### Client Plugin Flow (Option D)

```
1. Compute SHA-256 of local spec file → local_hash
2. Read .sanshain-cache → {last_hash, last_version}
3. If local_hash == last_hash:
     → Skip provide (file unchanged, server already has it)
4. Else:
     → POST /provide with base_version=last_version, content
     → On 202: update cache with {local_hash, response.version}
     → On 409 "outdated": warn developer to pull branch changes,
       then re-provide (or fail build)
     → On 409 "incompatible": fail build with breaking-change details
```

---

## Cross-Cutting Concern: Require-Side Caching

The same version/hash mechanism can benefit the **require** side:
- `GET /require` and `POST /require-bundle` could return a version/hash in headers.
- Client plugins cache the received spec + hash.
- On next build, client sends `If-None-Match` → gets `304` if spec hasn't changed → uses cached file.
- Reduces build times when upstream specs are stable.

This is a natural extension but should be a separate task.

---

## Summary: Recommended Path

| Problem                     | Recommended Option                 | Effort         | Dependencies  |
|-----------------------------|------------------------------------|----------------|---------------|
| 1. PUB-only provide         | B (filter in service layer)        | Small          | None          |
| 2. Multi-publisher conflict | A (report warnings) → E (advisory) | Small → Medium | Problem 1     |
| 3. Concurrent overwrites    | A (optimistic concurrency)         | Medium         | Problem 4     |
| 4. No version in response   | A (JSON response body)             | Medium         | None          |
| 5. Client-side caching      | D (hash + version cache)           | Medium         | Problem 3 + 4 |

Suggested implementation order: **1 → 4 → 3 → 2 → 5**
(Problem 1 is independent; Problem 4 enables 3; Problem 3 enables 5; Problem 2 is incremental.)
