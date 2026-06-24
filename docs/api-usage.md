# API Usage

Sanshain Service provides a set of API endpoints for providing specifications, requiring endpoints, and managing the service.

## Client API Contract

The client-facing API (`/provide` and `/require`) is formally specified in [`api.yaml`](../api.yaml) as an OpenAPI 3.0.3 document. This file serves as the contract for all client plugins (Maven, Gradle, Cargo, Go, npm, etc.) and can be used with [OpenAPI Generator](https://openapi-generator.tech/) to produce client SDKs in any supported language.

## API Usage

### 1. `POST /provide`
Provide an OpenAPI specification for a service branch. For AsyncAPI, use `/provide/asyncapi`. For Proto, use `/provide/grpc`.

**Payload (OpenAPI):**
```json
{
  "servicename": "UserService",
  "branch": "main",
  "openapi_yaml": "...",
  "dry_run": false,
  "base_version": 5
}
```

**Payload (AsyncAPI):**
```json
{
  "servicename": "EventService",
  "branch": "main",
  "asyncapi_yaml": "...",
  "dry_run": false,
  "base_version": 2
}
```

**Payload (gRPC):**
```json
{
  "servicename": "GreeterService",
  "branch": "main",
  "proto_content": "...",
  "dry_run": false,
  "base_version": 1
}
```

| Field | Required | Description |
|---|---|---|
| `servicename` | Yes | Name of the service providing the specification. |
| `branch` | Yes | Branch name for the specification. |
| `openapi_yaml` | Yes | The full OpenAPI specification as a YAML string. |
| `dry_run` | No | When `true`, validates the spec (parsing, conflict detection) without storing anything. Defaults to `false`. |
| `base_version` | No | Optional base version for optimistic concurrency control. |

**Response (202 Accepted):**
```json
{
  "version": 6,
  "content_hash": "6a3501...",
  "changes": {
    "inserts": 2,
    "updates": 1,
    "deletes": 0
  }
}
```

**Optimistic Concurrency & Caching:**
- **Content-Based Skipping**: Sanshain calculates a SHA-256 hash of every specification. If you provide a spec that is identical to the current one, the server skips processing and returns the current version without a version bump.
- **Version Tracking**: Each service branch has a monotonic version number. Clients should store the `version` and `content_hash` returned by the server.
- **Conflict Detection**: Use `base_version` to prevent overwriting concurrent changes from other developers or CI jobs. If the provided `base_version` does not match the current version on the server, the request is rejected with `409 Conflict`.

**Protected Branch Behavior:** On protected branches (e.g., `main`, `master`), Sanshain performs a **backward compatibility check** when an endpoint's schema changes. Backward-compatible changes — such as adding optional fields, new schemas, or new endpoints — are accepted. Breaking changes — such as removing fields, changing types, or removing response codes — are rejected with `409 Conflict`.

**Feature Branch Behavior:** On feature branches, endpoint schemas can be freely overwritten, but **multi-publisher conflict detection** is enforced:
- The first `provide` on a branch establishes the "source" version.
- Subsequent modifications must be backward-compatible with the source.
- If a service modifies an endpoint, it becomes the "owner"; other services must then be compatible with the owner's version.
- If the owner reverts to the source version, ownership is cleared.

**Auto-Skip for New Services:** If a service has **no endpoints on any protected branch** (i.e., it has never been "released"), the shared contract backward-compatibility check is automatically skipped on feature branches. This allows developers to freely iterate on a brand new API until the first merge to `main`/`master`. No special parameter is needed — the behavior is transparent.

**Force Mode (`force`):** When `force` is set to `true` in the provide request body, the shared contract `source_yaml` and `current_yaml` are both reset to the new content, bypassing all backward-compatibility checks. This is useful when a developer needs to start fresh on a feature branch after a fundamentally broken API iteration. Force mode is **blocked on protected branches** (returns `400 Bad Request`).

### 2. `GET /require`
Request the OpenAPI snippet for a specific endpoint and record the dependency. For AsyncAPI, use `/require/asyncapi`. For Proto, use `/require/grpc`.

**Query Parameters:**
- `clientname`: Name of the client service.
- `servicename`: Name of the target service.
- `branch`: Branch name.
- `path`: Endpoint path.
- `method`: HTTP method (GET, POST, etc.).
- `timeout` *(optional)*: Long-polling timeout in seconds. If the endpoint is not yet available, the server will poll until it appears or the timeout expires.
- `dry_run` *(optional)*: When `true`, validates the endpoint lookup without recording a dependency. Defaults to `false`.

If the requested endpoint is not found, the response includes a descriptive error message listing the service name, branch, and endpoint details.

**Feature Branch Fallback:** If the endpoint is not found on a non-protected (feature) branch, Sanshain automatically falls back to protected branches (e.g., `main`, `master`).

**Example:**
`GET /require?clientname=WebClient&servicename=UserService&branch=main&path=/users&method=GET`
`GET /require?clientname=WebClient&servicename=UserService&branch=feature/xyz&path=/users&method=GET&timeout=30`
`GET /require?clientname=WebClient&servicename=UserService&branch=main&path=/users&method=GET&dry_run=true`

### 3. `POST /require-bundle`
Request multiple endpoint snippets merged into a single OpenAPI spec and record the dependencies.

**Payload:**
```json
{
  "clientname": "WebClient",
  "servicename": "UserService",
  "branch": "main",
  "endpoints": [
    { "path": "/users", "method": "GET" },
    { "path": "/users/{id}", "method": "POST" }
  ],
  "timeout": 30,
  "dry_run": false
}
```

| Field | Required | Description |
|---|---|---|
| `clientname` | Yes | Name of the client registering the dependencies. |
| `servicename` | Yes | Name of the service that provides the endpoints. |
| `branch` | Yes | Branch name to retrieve the endpoints from. |
| `endpoints` | Yes | List of `{path, method}` objects to include in the merged spec. |
| `timeout` | No | Long-polling timeout in seconds. |
| `dry_run` | No | When `true`, validates all endpoints without recording dependencies. Defaults to `false`. |

If some requested endpoints are missing, the response returns `404` with a descriptive message listing all missing endpoints.

### 4. `GET /endpoint-versions`
Retrieve the version history for an endpoint on a protected branch, including the full YAML at each version and a unified diff from the previous version.

**Query Parameters:**
- `servicename`: Name of the service.
- `branch`: Branch name.
- `path`: Endpoint path.
- `method`: HTTP method.

**Example:**
`GET /endpoint-versions?servicename=UserService&branch=main&path=/users&method=POST`

**Response:** An array of version objects, each containing:
- `version_number` — Sequential version number.
- `yaml_content` — Full YAML content at that version.
- `diff_from_previous` — Unified diff from the previous version (empty for version 1).
- `created_at` — Timestamp of the version.

### 5. Dry-Run Mode (CI Validation)

All three endpoints (`/provide`, `/require`, `/require-bundle`) support a `dry_run` parameter. When set to `true`, the request runs full validation (YAML parsing, conflict detection, endpoint lookup) but **does not persist any data** — no specs are stored, no client dependencies are recorded.

This enables CI pipelines to test whether a feature branch would be valid against the main branch before allowing a PR to be merged, without polluting the database with temporary data.

## Authentication

Sanshain uses session-based authentication with Argon2 password hashing.

**First Start:** On first launch (empty user table), a `root` admin user is created with a random password printed to stderr. The DevOps engineer must change this password immediately.

**Login:** `POST /auth/login` with `{"username": "...", "password": "..."}` returns a Bearer token (24h expiry).

**Endpoints:**
- `POST /auth/login` — Authenticate and receive a session token.
- `POST /auth/logout` — Invalidate the current session (requires `Authorization: Bearer <token>`).
- `GET /auth/me` — Get current user info (requires `Authorization: Bearer <token>`).
- `POST /auth/change-password` — Change password (requires `Authorization: Bearer <token>`, body: `{"old_password": "...", "new_password": "..."}`).
- `POST /auth/register` — Register a new local user (requires local users to be enabled by admin, body: `{"username": "...", "password": "..."}`).

**Dev Mode:** By default, all API endpoints (`/provide`, `/require`, `/report`) are **locked** (return 403). An admin can enable "Dev Mode" via `POST /admin/settings/dev-mode` with `{"enabled": true}`, which opens all non-admin API endpoints without authentication.

**Local User Registration:** Permitted when the system is set to **Local Users** authentication mode (configured in the admin dashboard). When active, anyone can register via `POST /auth/register` with `{"username": "...", "password": "..."}`. Newly registered users must be **approved by an admin** before they can log in, unless the admin enables automatic acknowledgement/approval via `POST /admin/settings/auto-approve` with `{"enabled": true}`.

**LDAP Authentication:** Sanshain supports delegating authentication to an external LDAP/Active Directory server. Configure via the admin dashboard (Authentication section) or the `PUT /admin/auth-config` API endpoint. When LDAP mode is active, users authenticate against the LDAP server and are auto-provisioned as local shadow accounts (so sessions and API tokens work unchanged). Admin status can be derived from LDAP group membership. See `docs/administration.md` for detailed configuration instructions.

**Branch Max-Age Auto-Cleanup:** Non-protected branches are automatically deleted after a configurable period of inactivity (default: 30 days). A background task runs every hour. Configure via `GET/POST /admin/settings/branch-max-age` (body: `{"days": 30}`). Trigger immediate cleanup via `POST /admin/settings/branch-cleanup`. Protected branches (e.g., `main`, `master`) are never cleaned up.

#### API Tokens (for CI/Programmatic Access)

Approved users can create long-lived API tokens for use in CI pipelines without embedding credentials.

**Endpoints:**
- `POST /auth/tokens` — Create a token: `{"name": "jenkins-ci", "expires_in_days": 365}`. Returns the raw token **once** (prefixed `san_`).
- `GET /auth/tokens` — List user's tokens (name, dates, last used — never the raw token).
- `DELETE /auth/tokens/{id}` — Revoke a token.

**Usage:** Send the token as a Bearer header: `Authorization: Bearer san_xxxxxxxxxxxx`

## Admin API

All `/admin/*` endpoints require a valid admin session token (`Authorization: Bearer <token>`).

#### Protected Branches
Manage which branches enforce immutable endpoint paths. By default, `main` and `master` are protected. On non-protected (feature) branches, endpoint DTOs can be freely updated.

- `GET /admin/protected-branches` — List all protected branch patterns.
- `POST /admin/protected-branches` — Add a pattern: `{"pattern": "release"}`.
- `DELETE /admin/protected-branches/:pattern` — Remove a pattern.

#### Read-Only Endpoints (UI Data Access)
Dedicated read-only endpoints used by the web UI to fetch endpoint data without side effects. These replace the previous approach of calling business-logic endpoints (`/require`) from the frontend.

- `GET /admin/endpoint-yaml?servicename=...&branch=...&path=...&method=...` — Fetch the YAML content for a specific endpoint (with feature-branch fallback).
- `GET /admin/endpoint-versions?servicename=...&branch=...&path=...&method=...` — Fetch the version history for a specific endpoint.
- `GET /admin/services/:name/branches/:branch/endpoints` — List all (non-deleted) endpoints for a service branch.

#### Data Management
List and delete services, branches, and clients via the admin API. Deleting a service cascades to its branches, endpoints, and related dependencies.

- `GET /admin/services` — List all services.
- `DELETE /admin/services/:name` — Delete a service and all its branches, endpoints, and dependencies.
- `GET /admin/services/:name/branches` — List all branches for a service.
- `DELETE /admin/services/:name/branches/:branch` — Delete a branch and its endpoints and dependencies.
- `GET /admin/clients` — List all clients.
- `DELETE /admin/clients/:name` — Delete a client and all its dependencies.

#### User Management
- `GET /admin/users` — List all users (id, username, is_admin, approved).
- `POST /admin/users/:id/approve` — Approve a pending user.
- `DELETE /admin/users/:id` — Delete a user.
- `GET /admin/settings/auto-approve` — Check if new local users are auto-approved.
- `POST /admin/settings/auto-approve` — Enable/disable automatic approval for newly registered local users: `{"enabled": true}`.

### `GET /report`
Generate a dependency report for a specific branch.

**Query Parameters:**
- `branch`: Branch name to report on.
