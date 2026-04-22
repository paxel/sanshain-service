<p align="center">
  <img src="static/images/sonne.png" alt="Sanshain Logo" width="320">
</p>

# [Sanshain Service](https://github.com/paxel/sanshain-service)

Sanshain (Japanese for "Sunshine") is a specialized service designed to manage, split, and distribute API specifications (OpenAPI, AsyncAPI, and gRPC/Proto). It acts as a central repository that microservices use to "provide" their full API definitions and clients to "require" only the specific snippets (endpoints, channels, or methods) they actually use **at build time**.

## Use Cases

- **Contract-First Development**: Services publish their API contracts, and clients consume exactly what they need.
- **Dependency Tracking**: Automatically track which clients are using which specific endpoints across different branches.
- **Impact Analysis**: Identify unused endpoints or missing requirements before they cause issues in production.
- **Client Generation**: Provide minimal, focused YAML snippets to clients for lightweight code generation in any language.
- **Web Dashboard**: Explore the dependency graph and API availability through a browser.

## Getting Started

### Prerequisites
- Rust (latest stable)
- SQLite (default) or PostgreSQL

### Installation

#### From Source
```bash
cargo build --release
./target/release/sanshain_service
```

#### From GitHub Release
Download the binary for your architecture from the [Releases](https://github.com/paxel/sanshain-service/releases) page (`sanshain-linux-x86_64` or `sanshain-linux-aarch64`):
```bash
chmod +x sanshain-linux-x86_64
./sanshain-linux-x86_64
```

#### Docker (GitHub Container Registry)
```bash
docker pull ghcr.io/paxel/sanshain-service:latest
docker run -p 3000:3000 -v sanshain-data:/data ghcr.io/paxel/sanshain-service:latest
```

Or use the `Dockerfile` and `docker-compose.yaml` included in each [release](https://github.com/paxel/sanshain-service/releases) to run with Docker Compose.

### Initial Admin Setup

On first start (empty database), Sanshain creates a `root` admin account and prints a **random password to stderr**:

```
[INITIAL SETUP] Admin user created. Username: root, Password: <random>
[INITIAL SETUP] Change this password immediately at http://localhost:3000/admin.html
```

**Important:** Copy this password from the startup logs immediately. It is only shown once.

1. Open `http://localhost:3000/admin.html` in your browser.
2. Log in with username `root` and the generated password.
3. Change the password using the admin dashboard or via the API:
   ```bash
   # Login
   TOKEN=$(curl -s -X POST http://localhost:3000/auth/login \
     -H "Content-Type: application/json" \
     -d '{"username":"root","password":"<initial-password>"}' | jq -r .token)

   # Change password
   curl -X POST http://localhost:3000/auth/change-password \
     -H "Authorization: Bearer $TOKEN" \
     -H "Content-Type: application/json" \
     -d '{"old_password":"<initial-password>","new_password":"<new-secure-password>"}'
   ```

If you lose the initial password, delete the database file and restart the service to regenerate it.

## Service URL
The service is available at: `http://localhost:3000` (default when running locally).
This URL serves the Web Dashboard directly.

## Core Features
- **Spec Splitting**: Automatically splits OpenAPI, AsyncAPI, and Proto files into per-endpoint/channel/method snippets.
- **Backward Compatibility Checking**: Protected branches accept backward-compatible schema changes (OpenAPI) and reject breaking changes.
- **Endpoint Version History**: Every accepted update on a protected branch is versioned with full content and unified diffs.
- **Dependency Tracking**: Records which clients depend on which service endpoints across different protocols.
- **Dependency Reports**: Identifies unused endpoints and missing dependencies.
- **Web Dashboard**: Navigate services, branches, and client dependencies visually, with dark/light mode toggle.
- **Dry-Run Mode**: Validate specs and dependencies without persisting data — ideal for CI pipelines.

### Performance Benchmarks

Sanshain is optimized for high-performance API specification processing. Micro-benchmarks (via `criterion.rs`) show the following results on a typical development machine:

| Operation | Time | Notes |
|---|---|---|
| `split_openapi` | ~900 µs | Splitting an OpenAPI spec into per-endpoint snippets |
| `merge_endpoint_yamls` | ~850 µs | Bundling multiple endpoint snippets with shared components |
| `split_asyncapi` | ~119 µs | Splitting an AsyncAPI spec into per-channel snippets |
| `split_proto` | ~8 µs | Splitting a Proto file into per-method snippets |
| `normalize_path` | ~2.5 µs | Normalizing 4 endpoint paths (collapsing slashes, unifying variables) |
| `generate_diff` | ~22 µs | Generating a unified diff between two endpoint YAML snippets |
| `check_backward_compatibility` | ~913 µs | Checking backward compatibility between two OpenAPI specs |

Run benchmarks locally with `cargo bench`. Results above were measured on a development machine using [Criterion.rs](https://github.com/bheisler/criterion.rs).

These optimizations — including static regex compilation via `LazyLock` — ensure that even large-scale API changes are processed in sub-millisecond time, maintaining a fast feedback loop in CI/CD pipelines.

## Client API Contract

The client-facing API (`/provide` and `/require`) is formally specified in [`api.yaml`](api.yaml) as an OpenAPI 3.0.3 document. This file serves as the contract for all client plugins (Maven, Gradle, Cargo, Go, npm, etc.) and can be used with [OpenAPI Generator](https://openapi-generator.tech/) to produce client SDKs in any supported language.

## API Usage

### 1. `POST /provide`
Provide an OpenAPI specification for a service branch. For AsyncAPI, use `/provide/asyncapi`. For Proto, use `/provide/grpc`.

**Payload (OpenAPI):**
```json
{
  "servicename": "UserService",
  "branch": "main",
  "openapi_yaml": "...",
  "dry_run": false
}
```

**Payload (AsyncAPI):**
```json
{
  "servicename": "EventService",
  "branch": "main",
  "asyncapi_yaml": "...",
  "dry_run": false
}
```

**Payload (gRPC):**
```json
{
  "servicename": "GreeterService",
  "branch": "main",
  "proto_content": "...",
  "dry_run": false
}
```

| Field | Required | Description |
|---|---|---|
| `servicename` | Yes | Name of the service providing the specification. |
| `branch` | Yes | Branch name for the specification. |
| `openapi_yaml` | Yes | The full OpenAPI specification as a YAML string. |
| `dry_run` | No | When `true`, validates the spec (parsing, conflict detection) without storing anything. Defaults to `false`. |

**Protected Branch Behavior:** On protected branches (e.g., `main`, `master`), Sanshain performs a **backward compatibility check** when an endpoint's schema changes. Backward-compatible changes — such as adding optional fields, new schemas, or new endpoints — are accepted. Breaking changes — such as removing fields, changing types, or removing response codes — are rejected with `409 Conflict` and a descriptive error message. Each accepted update records a new version in the endpoint's version history (see `GET /endpoint-versions` below).

On **feature branches**, endpoint schemas can be freely overwritten without compatibility checks.

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

### 6. Authentication

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

**Local User Registration:** Disabled by default. An admin can enable it via `POST /admin/settings/local-users` with `{"enabled": true}`. When enabled, anyone can register via `POST /auth/register` with `{"username": "...", "password": "..."}`. Newly registered users must be **approved by an admin** before they can log in.

**LDAP Authentication:** Sanshain supports delegating authentication to an external LDAP/Active Directory server. Configure via the admin dashboard (Authentication section) or the `PUT /admin/auth-config` API endpoint. When LDAP mode is active, users authenticate against the LDAP server and are auto-provisioned as local shadow accounts (so sessions and API tokens work unchanged). Admin status can be derived from LDAP group membership. See `docs/administration.md` for detailed configuration instructions.

**Branch Max-Age Auto-Cleanup:** Non-protected branches are automatically deleted after a configurable period of inactivity (default: 30 days). A background task runs every hour. Configure via `GET/POST /admin/settings/branch-max-age` (body: `{"days": 30}`). Trigger immediate cleanup via `POST /admin/settings/branch-cleanup`. Protected branches (e.g., `main`, `master`) are never cleaned up.

#### API Tokens (for CI/Programmatic Access)

Approved users can create long-lived API tokens for use in CI pipelines (Jenkins, Maven, Gradle, etc.) without embedding credentials.

**Endpoints:**
- `POST /auth/tokens` — Create a token: `{"name": "jenkins-ci", "expires_in_days": 365}`. Returns the raw token **once** (prefixed `san_`).
- `GET /auth/tokens` — List user's tokens (name, dates, last used — never the raw token).
- `DELETE /auth/tokens/{id}` — Revoke a token.

**Usage:** Send the token as a Bearer header: `Authorization: Bearer san_xxxxxxxxxxxx`

**Maven `settings.xml`:**
```xml
<server>
  <id>sanshain</id>
  <username>ignored</username>
  <password>san_xxxxxxxxxxxx</password>
</server>
```

**User Account Page:** Visit `/account.html` to register, log in, change your password, and create/manage API tokens via the web UI.

### Web Pages

- `/` — Landing page with service info, version, and links to all pages.
- `/service.html` — Service overview: registered services, dependency graphs, compatibility reports, and endpoint version history with diff viewer.
- `/admin.html` — Admin dashboard: manage services, branches, clients, protected branches, and settings.
- `/account.html` — Account management: sign in, register, profile, and API tokens.

All pages include a 🌙/☀️ **dark/light mode toggle** in the navigation bar. The preference is stored in a cookie and persists across sessions.

### Version

`GET /version` returns the service version as JSON: `{"version": "0.8.1"}`.

### 7. Admin API (Session-Based Authentication)
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
- `GET /admin/settings/local-users` — Check if local user registration is enabled.
- `POST /admin/settings/local-users` — Enable/disable local user registration: `{"enabled": true}`.

### 8. `GET /report`
Generate a dependency report for a specific branch.

**Query Parameters:**
- `branch`: Branch name to report on.

## Demo Script

The `demo.sh` script populates a running Sanshain instance with sample data so you can explore the web UI immediately after deployment. It demonstrates 13 feature scenarios with 6 realistic microservices (user-service, order-service, notification-service, payment-service, inventory-service, and web-frontend), including:

- Services acting as both providers and consumers
- Circular dependencies (order↔payment, order↔inventory)
- Backward-compatible updates on protected branches
- Breaking change rejection
- Endpoint version history with diffs
- Require-bundle (multi-endpoint fetch)
- Dry-run mode
- Dependency reports (JSON + Markdown)
- Feature-branch fallback to main

**Prerequisites:** `curl` and `jq` must be installed.

```bash
# Against a local instance (dev mode must be enabled or a token provided)
./demo.sh

# Against a remote instance with an API token
SANSHAIN_URL=https://sanshain.example.com SANSHAIN_TOKEN=san_xxxx ./demo.sh
```

| Variable | Default | Description |
|---|---|---|
| `SANSHAIN_URL` | `http://localhost:3000` | Base URL of the Sanshain instance |
| `SANSHAIN_TOKEN` | *(empty)* | API token or session token for authentication |

After the script completes, open the service overview page to browse the dependency graph and drill into individual services and clients.

### Additional Demo Scenarios

Two larger demo scripts are shipped alongside `demo.sh` to showcase how the dependency graph scales, and how the service handles multiple protocols:

- **`demo_protocols.sh`** — demonstrates **AsyncAPI (Kafka)** and **gRPC (Proto)** support. It registers a scenario where a REST gateway calls a gRPC user-service, which in turn publishes events to a Kafka event-bus. An analytics service consumes all events. All protocols are tracked and displayed in a unified dependency graph.
- **`demo2.sh`** — a synthetic large-scale scenario with **30 services** (Frontend → API Middleware → Backend, a complex ETL pipeline with central orchestration and multiple enrichers, an ML system, multiple ingestion sources and exports, and infra wrappers for Postgres / Elastic / Redis / Kafka). Registered on the `main` branch.
- **`demo3.sh`** — reproduces the publicly documented **[Google Cloud "Online Boutique" (Hipster Shop)](https://github.com/GoogleCloudPlatform/microservices-demo)** microservices demo: `frontend`, `cartservice`, `productcatalogservice`, `currencyservice`, `paymentservice`, `shippingservice`, `emailservice`, `checkoutservice`, `recommendationservice`, `adservice` and `loadgenerator`, with dependencies taken straight from the upstream architecture diagram. Registered on a dedicated `google` branch (override with `DEMO3_BRANCH=<name>`).

```bash
./demo_protocols.sh               # AsyncAPI and gRPC/Proto scenario
./demo2.sh                        # 30-service synthetic scenario on `main`
./demo3.sh                        # Google Online Boutique on `google`
```

In the service overview UI, use the **branch switcher** to flip between `main` (demo/demo2) and `google` (demo3) — the graph re-renders with the topology of the selected branch, so each published scenario can be explored independently.

## Development

### Prerequisites
- Rust (2024 edition)
- SQLite (default) or PostgreSQL

### Configuration

| Variable | Default | Description |
|---|---|---|
| `DATABASE_URL` | `sqlite:sanshain.db?mode=rwc` | Database connection string. Use `postgres://user:pass@host:5432/dbname` for PostgreSQL. |
| `BIND_ADDRESS` | `0.0.0.0:3000` | Address and port to listen on. |
| `MAX_POSTGRES_CONNECTIONS` | `20` | Max connection pool size for PostgreSQL. |
| `MAX_SQLITE_CONNECTIONS` | `1` | Max connection pool size for SQLite. |
| `SQLITE_BUSY_TIMEOUT_MS` | `5000` | SQLite busy timeout in milliseconds. |
| `CLEANUP_INTERVAL_SECS` | `3600` | Interval for background cleanup tasks (branches, dependencies). |
| `CSRF_MAX_AGE_HOURS` | `24` | Maximum age of CSRF tokens before they are pruned. |
| `LOG_BUFFER_SIZE` | `100` | Number of messages kept in the in-memory log buffer per level. |
| `SPEC_UPDATED_CHANNEL_SIZE` | `100` | Size of the broadcast channel for specification updates. |
| `CAPTURE_LOG_FILTER` | `sanshain_service=debug,tower_http=debug` | Log level filter for the in-memory log capture buffer. |
| `PROMETHEUS_ENDPOINT` | `/metrics` | Path for Prometheus metrics. |
| `STATIC_DIR` | `static` | Directory containing static web assets. |
| `LOGIN_SESSION_DURATION_HOURS` | `24` | Duration of user login sessions in hours. |
| `INITIAL_ADMIN_USERNAME` | `root` | Username for the initial admin account. |
| `INITIAL_ADMIN_PASSWORD` | *random* | Pre-defined password for the initial admin account. |
| `INITIAL_ADMIN_TOKEN` | *random* | Pre-defined session token for the initial admin account. |
| `INSTANCE_ID` | *random UUID* | Unique ID for this service instance. |
| `CACHE_MEMORY_MB` | `256` | In-memory cache size in MB. Set to `0` to disable caching entirely. Configurable at runtime via admin UI. |
| `LOG_FORMAT` | `text` | Log output format (`text` or `json`). |
| `RUST_LOG` | `sanshain_service=info,tower_http=info` | Log level filter (e.g., `sanshain_service=debug,tower_http=debug` for verbose output). |

### Running the service
```bash
cargo run
```
On first start, the root admin credentials are printed to stderr. The service listens on `0.0.0.0:3000` by default. Override with `BIND_ADDRESS`.

### Docker
The existing `Dockerfile` builds the service from source (multi-stage build). This is useful if you don't have a Rust toolchain installed:
```bash
docker build -t sanshain .
docker run -p 3000:3000 -v sanshain-data:/data sanshain
```

For production use, pull the pre-built image from GHCR or use the `Dockerfile` and `docker-compose.yaml` from the [release assets](https://github.com/paxel/sanshain-service/releases).

The SQLite database is persisted in a Docker volume at `/data/sanshain.db`. For PostgreSQL, set `DATABASE_URL`:
```bash
docker run -p 3000:3000 -e DATABASE_URL=postgres://user:pass@host:5432/sanshain sanshain
```

### Health Check
`GET /health` returns `200 OK` when the service is running. Used by Docker `HEALTHCHECK` and Kubernetes probes.

### Database
The service uses SQLite by default. The database file `sanshain.db` will be created automatically on the first run, and migrations will be applied. Each database adapter owns its migrations under `src/infrastructure/migrations/<db>/`.

**PostgreSQL:** Set `DATABASE_URL` to a PostgreSQL connection string to use PostgreSQL instead:
```bash
DATABASE_URL=postgres://user:password@localhost:5432/sanshain cargo run
```
The backend is auto-detected from the URL prefix (`postgres://` or `postgresql://`). Migrations run automatically on startup. The current database backend is visible in the admin dashboard under **Database Configuration**.

---

### Running Tests
```bash
cargo test
```

## Documentation
See the [`docs/`](docs/) directory for detailed hands-on user guides (getting started, usage, administration, CI integration).

For an in-depth look at the architecture, security internals, and implementation details of every subsystem, see the [Developer Guide](docs/developer-guide.md).

## Changelog
See [CHANGELOG.md](CHANGELOG.md) for a detailed history of changes.
