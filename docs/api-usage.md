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
- `servicename`: Name of the service.
- `branch`: Branch name (e.g., `main`).
- `openapi_yaml` / `asyncapi_yaml` / `proto_content`: Spec content.
- `dry_run`: If `true`, validates without storing.
- `base_version`: Optional, for optimistic concurrency.

**Response (202 Accepted):** Returns `version`, `content_hash`, and a summary of `changes`.

---

### 2. `GET /require`
Request the snippet for a single endpoint.

**Query Parameters:**
- `clientname`: Your service name.
- `servicename`: Target service name.
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
  "clientname": "WebApp",
  "servicename": "UserService",
  "branch": "main",
  "endpoints": [
    {"path": "/users", "method": "GET"},
    {"path": "/users/{id}", "method": "DELETE"}
  ]
}
```

---

## Advanced Behaviors

### Optimistic Concurrency & Caching
- **Content-Based Skipping**: Identical specs are detected by hash and skipped (no version bump).
- **Conflict Detection**: Use `base_version` to prevent overwriting concurrent updates. Returns `409 Conflict` on mismatch.

### Backward Compatibility
- **Protected Branches**: Breaking changes (e.g., removing fields) are rejected with `409 Conflict`.
- **Feature Branch Fallback**: Clients can request endpoints from feature branches; if not found, Sanshain falls back to `main`.
- **Ownership**: On feature branches, the first service to modify an endpoint "owns" it, ensuring compatibility for subsequent updates.

### Admin & Auth
- **API Tokens**: Create tokens at `/account.html` for CI usage.
- **Developer Mode**: Bypasses authentication for local testing. Requires both enabling it (Admin Panel or `SANSHAIN_DEV_MODE=true`) and the `ALLOW_INSECURE_DEV_MODE=true` safety gate; it fails closed otherwise. Never use in production — see [Developer Mode](administration.md#enabling-dev-mode-local-only).
