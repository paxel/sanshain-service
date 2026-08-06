# API Usage

Sanshain Service provides a set of API endpoints for providing specifications, requiring endpoints, and managing the service.

### TL;DR
- **Contract**: The full API is defined in [`api.yaml`](../api.yaml).
- **Provide**: `POST /provide` (OpenAPI), `/provide/asyncapi`, or `/provide/grpc` — with a declared `stability`, version read from the spec.
- **Require**: `GET /require` (single) or `POST /require-bundle` (multiple) — at an exact pinned `version`.
- **Auth**: Use `Authorization: Bearer <token>` (get tokens at `/account.html`). Tokens are accepted **only** in this header — never as a URL/query parameter, to avoid leaking them through logs, history, and referrers.

---

## Client API Contract

The client-facing API is formally specified in [`api.yaml`](../api.yaml) as an OpenAPI 3.0.3 document. Use this with [OpenAPI Generator](https://openapi-generator.tech/) to produce client SDKs.

---

## Core Endpoints

### 1. `POST /provide`
Upload a specification under its declared version.

**Endpoints:**
- `/provide` (OpenAPI)
- `/provide/asyncapi` (AsyncAPI)
- `/provide/grpc` (Protocol Buffers)

**Key Fields:**
- `producername`: Name of the producer providing the spec.
- `stability`: `snapshot` (overwritable, expires when unused) or `ga` (immutable). **Required.**
- `openapi_yaml` / `asyncapi_yaml` / `proto_content`: Spec content. The version is read from it — `info.version`, or a `// sanshain-version: MAJOR.MINOR.PATCH` comment for proto. `MAJOR[.MINOR[.PATCH]]` with an optional `v` prefix; omitted parts are zero, stored canonically as three-part. No suffixes.
- `dry_run`: If `true`, validates and classifies without storing.

**Response (202 Accepted):** Returns `version`, `stability`, `content_hash`, and a summary of `changes` (`inserts`/`updates`/`deletes`).

**Rejection (409 Conflict):** The body carries `proposed_version` — the next free version, bumped by what actually changed. Set your spec's version to it and republish; every rejection is self-service.

---

### 2. `GET /require`
Request the snippet for a single endpoint at a pinned version.

**Query Parameters:**
- `consumername`: Your own name (the consumer recording the dependency).
- `producername`: Name of the producer that provides the endpoint.
- `version`: The exact pinned version (`MAJOR.MINOR.PATCH`). No ranges, no `latest`. A leading `v` and omitted MINOR/PATCH are accepted and normalized (`v1` → `1.0.0`).
- `path`: Endpoint path or channel.
- `method`: HTTP method or operation.
- `dry_run`: Validate without recording a dependency.

**Resolution** is immediate — GA preferred, else the same-numbered snapshot:
- `404` **Unknown**: the producer or the pinned version does not exist — a Pin configuration error.
- `410` **Absent**: the pinned version exists and deliberately does not include this endpoint.

Responses carry `X-Sanshain-Version` (the pin) and `X-Sanshain-Stability` (`ga` or `snapshot` — what actually answered).

---

### 3. `POST /require-bundle`
Request multiple endpoints in a single call. Returns a merged specification with deduplicated models.

**Payload:**
```json
{
  "consumername": "WebApp",
  "producername": "UserService",
  "version": "2.1.0",
  "endpoints": [
    {"path": "/users", "method": "GET"},
    {"path": "/users/{id}", "method": "DELETE"}
  ]
}
```

If *any* requested endpoint is absent from the pinned version the whole bundle fails `410` naming the missing endpoints, and no dependency is recorded.

---

## Naming: Producer and Consumer

The two roles are called **Producer** (provides a spec) and **Consumer** (requires endpoints);
see [`CONTEXT.md`](../CONTEXT.md).

- **Only the canonical field names are accepted.** `producername` and `consumername` are the
  wire names everywhere; the pre-1.6 aliases `servicename` and `clientname` were removed in
  2.0.0 and are rejected as unknown fields.
- **Admin routes are not aliased.** `/admin/services*` became `/admin/producers*`,
  `/admin/clients*` became `/admin/consumers*`, and `/admin/nuke/services` / `/admin/nuke/clients`
  became `/admin/nuke/producers` / `/admin/nuke/consumers`. The old paths return `404`. The same
  applies to the discovery pages: `services.html` → `producers.html`, `clients.html` →
  `consumers.html`.

Note that 2.0 is a clean break with the 1.x branch model: `branch`, `base_version`, `force`,
`timeout` and `pull_from_branch` are no longer accepted anywhere and are rejected as unknown
fields.

---

## Advanced Behaviors

### Version Rules & Caching
- **Idempotency**: Re-providing byte-identical content is a no-op regardless of stability or caller — no stored change, and `changes` comes back all zero. CI re-runs of the same commit never fight.
- **GA immutability**: A Provide for an existing GA version with different content returns `409` with `proposed_version` (breaking → major, additive → minor, shape-identical → patch). This catches the forgot-to-bump mistake at the door.
- **Semver honesty**: A GA whose changes relative to the highest GA below it are breaking without a major bump is rejected `409` with the correct proposal. Snapshots are never compatibility-checked.
- **Promotion**: A GA Provide for a number that exists as a snapshot promotes it in place. A snapshot Provide for a number that has gone GA is rejected — a released number can never carry a snapshot again.
- **ETag caching**: All require endpoints support `ETag`/`If-None-Match`; unchanged content returns `304 Not Modified`.

### Discovering Versions
- `GET /producers/{producername}/versions` lists a Producer's version lines — version, stability, content hash, timestamps, endpoint count, and snapshot expiry. This is what "what can I upgrade to?" tooling reads.

### Admin & Auth
- **API Tokens**: Create tokens at `/account.html` for CI usage.
- **Developer Mode**: Bypasses authentication for local testing. Requires both enabling it (Admin Panel or `SANSHAIN_DEV_MODE=true`) and the `ALLOW_INSECURE_DEV_MODE=true` safety gate; it fails closed otherwise. Never use in production — see [Developer Mode](administration.md#enabling-dev-mode-local-only).
