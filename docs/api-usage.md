# API Usage

Sanshain Service provides a set of API endpoints for providing specifications, requiring endpoints, and managing the service.

### TL;DR
- **Contract**: The full API is defined in [`api.yaml`](../api.yaml).
- **Provide**: `POST /provide` (OpenAPI), `/provide/asyncapi`, or `/provide/grpc`.
- **Require**: `GET /require` (single) or `POST /require-bundle` (multiple).
- **Auth**: Use `Authorization: Bearer <token>` (get tokens at `/account.html`). Tokens are accepted **only** in this header — never as a URL/query parameter, to avoid leaking them through logs, history, and referrers.

---

## Client API Contract

The client-facing API is formally specified in [`api.yaml`](../api.yaml) as an OpenAPI 3.0.3 document. Use this with [OpenAPI Generator](https://openapi-generator.tech/) to produce client SDKs.

---

## Core Endpoints

### 1. `POST /provide`
Upload a specification for a service branch.

**Endpoints:**
- `/provide` (OpenAPI)
- `/provide/asyncapi` (AsyncAPI)
- `/provide/grpc` (Protocol Buffers)

**Key Fields:**
- `producername`: Name of the producer providing the spec.
- `branch`: Branch name (e.g., `main`).
- `openapi_yaml` / `asyncapi_yaml` / `proto_content`: Spec content.
- `dry_run`: If `true`, validates without storing.
- `base_version`: Optional, for optimistic concurrency.

**Response (202 Accepted):** Returns `version`, `content_hash`, and a summary of `changes`.

---

### 2. `GET /require`
Request the snippet for a single endpoint.

**Query Parameters:**
- `consumername`: Your own name (the consumer recording the dependency).
- `producername`: Name of the producer that provides the endpoint.
- `branch`: Target branch.
- `path`: Endpoint path or channel.
- `method`: HTTP method or operation.
- `timeout`: Wait for the endpoint if it hasn't been published yet (long-polling).
- `dry_run`: Validate without recording a dependency.

---

### 3. `POST /require-bundle`
Request multiple endpoints in a single call. Returns a merged specification with deduplicated models.

**Payload:**
```json
{
  "consumername": "WebApp",
  "producername": "UserService",
  "branch": "main",
  "endpoints": [
    {"path": "/users", "method": "GET"},
    {"path": "/users/{id}", "method": "DELETE"}
  ]
}
```

---

## Naming: Producer and Consumer

Since 1.6.0 the two roles are called **Producer** (provides a spec) and **Consumer** (requires
endpoints); see [`CONTEXT.md`](../CONTEXT.md). Two things follow for callers written against 1.5.x
or earlier:

- **Request fields are aliased.** `/provide`, `/provide/asyncapi`, `/provide/grpc`, `/require`,
  `/require/asyncapi`, `/require/grpc` and `/require-bundle` still accept `servicename` for
  `producername` and `clientname` for `consumername`. Send one or the other, not both — supplying
  both spellings of the same value is rejected as a duplicate field. Prefer the new names; the
  aliases exist for compatibility only.
- **Admin routes are not aliased.** `/admin/services*` became `/admin/producers*`,
  `/admin/clients*` became `/admin/consumers*`, and `/admin/nuke/services` / `/admin/nuke/clients`
  became `/admin/nuke/producers` / `/admin/nuke/consumers`. The old paths return `404`. The same
  applies to the discovery pages: `services.html` → `producers.html`, `clients.html` →
  `consumers.html`.

---

## Advanced Behaviors

### Optimistic Concurrency & Caching
- **No-Op Skipping**: A Provide that adds, changes and removes no endpoint is skipped entirely — no version bump, no stored revision, no update notification, and `changes` comes back all zero. Reformatting counts as a no-op (reordered keys, whitespace, comments), as does an edit confined to document-level fields such as `info` or `servers`. The version tracks the API surface, so a version change always means an endpoint changed.
- **Conflict Detection**: Use `base_version` to prevent overwriting concurrent updates. Returns `409 Conflict` on mismatch.

### Backward Compatibility
- **Protected Branches**: Breaking changes (e.g., removing fields) are rejected with `409 Conflict`.
- **Feature Branch Fallback**: Clients can request endpoints from feature branches; if not found, Sanshain falls back to `main`.
- **Ownership**: On feature branches, the first service to modify an endpoint "owns" it, ensuring compatibility for subsequent updates.

### Admin & Auth
- **API Tokens**: Create tokens at `/account.html` for CI usage.
- **Developer Mode**: Bypasses authentication for local testing. Requires both enabling it (Admin Panel or `SANSHAIN_DEV_MODE=true`) and the `ALLOW_INSECURE_DEV_MODE=true` safety gate; it fails closed otherwise. Never use in production — see [Developer Mode](administration.md#enabling-dev-mode-local-only).
