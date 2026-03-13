# Sanshain Service

Sanshain is a service designed to manage and distribute OpenAPI specifications for microservices, tracking client-service dependencies and generating usage reports.

## Service URL
The service is available at: `http://localhost:3000` (default when running locally).
This URL serves the Web Dashboard directly.

## Core Features
- **OpenAPI Splitting**: Automatically splits large OpenAPI files into per-endpoint snippets.
- **Dependency Tracking**: Records which clients depend on which service endpoints.
- **Dependency Reports**: Identifies unused endpoints and missing dependencies.
- **Web Dashboard**: Navigate services, branches, and client dependencies visually.

## API Usage

### 1. `POST /provide`
Provide an OpenAPI specification for a service branch.

**Payload:**
```json
{
  "servicename": "UserService",
  "branch": "main",
  "openapi_yaml": "..."
}
```

**Note:** Sanshain enforces an **Immutable Endpoint Path** policy within a branch. If the DTO/schema for an existing endpoint changes, you must increment the version in the path (e.g., `/api/v1/users` to `/api/v2/users`). Changes to the same path that alter the DTO will be rejected with `409 Conflict`.

### 2. `GET /require`
Request the OpenAPI snippet for a specific endpoint and record the dependency.

**Query Parameters:**
- `clientname`: Name of the client service.
- `servicename`: Name of the target service.
- `branch`: Branch name.
- `path`: Endpoint path.
- `method`: HTTP method (GET, POST, etc.).
- `timeout` *(optional)*: Long-polling timeout in seconds. If the endpoint is not yet available, the server will poll until it appears or the timeout expires.

**Feature Branch Fallback:** If the endpoint is not found on a non-protected (feature) branch, Sanshain automatically falls back to protected branches (e.g., `main`, `master`).

**Example:**
`GET /require?clientname=WebClient&servicename=UserService&branch=main&path=/users&method=GET`
`GET /require?clientname=WebClient&servicename=UserService&branch=feature/xyz&path=/users&method=GET&timeout=30`

### 3. Authentication

Sanshain uses session-based authentication with Argon2 password hashing.

**First Start:** On first launch (empty user table), a `root` admin user is created with a random password printed to stderr. The DevOps engineer must change this password immediately.

**Login:** `POST /auth/login` with `{"username": "...", "password": "..."}` returns a Bearer token (24h expiry).

**Endpoints:**
- `POST /auth/login` — Authenticate and receive a session token.
- `POST /auth/logout` — Invalidate the current session (requires `Authorization: Bearer <token>`).
- `GET /auth/me` — Get current user info (requires `Authorization: Bearer <token>`).
- `POST /auth/change-password` — Change password (requires `Authorization: Bearer <token>`, body: `{"old_password": "...", "new_password": "..."}`).

**Dev Mode:** By default, all API endpoints (`/provide`, `/require`, `/report`) are **locked** (return 403). An admin can enable "Dev Mode" via `POST /admin/settings/dev-mode` with `{"enabled": true}`, which opens all non-admin API endpoints without authentication.

### 4. Admin API (Session-Based Authentication)
All `/admin/*` endpoints require a valid admin session token (`Authorization: Bearer <token>`).

#### Protected Branches
Manage which branches enforce immutable endpoint paths. By default, `main` and `master` are protected. On non-protected (feature) branches, endpoint DTOs can be freely updated.

- `GET /admin/protected-branches` — List all protected branch patterns.
- `POST /admin/protected-branches` — Add a pattern: `{"pattern": "release"}`.
- `DELETE /admin/protected-branches/:pattern` — Remove a pattern.

#### Data Management
List and delete services, branches, and clients via the admin API. Deleting a service cascades to its branches, endpoints, and related dependencies.

- `GET /admin/services` — List all services.
- `DELETE /admin/services/:name` — Delete a service and all its branches, endpoints, and dependencies.
- `GET /admin/services/:name/branches` — List all branches for a service.
- `DELETE /admin/services/:name/branches/:branch` — Delete a branch and its endpoints and dependencies.
- `GET /admin/clients` — List all clients.
- `DELETE /admin/clients/:name` — Delete a client and all its dependencies.

### 5. `GET /report`
Generate a dependency report for a specific branch.

**Query Parameters:**
- `branch`: Branch name to report on.

## Development

### Prerequisites
- Rust (2024 edition)
- SQLite

### Configuration

| Variable | Default | Description |
|---|---|---|
| `DATABASE_URL` | `sqlite:sanshain.db?mode=rwc` | Database connection string |
| `BIND_ADDRESS` | `0.0.0.0:3000` | Address and port to listen on |

### Running the service
```bash
cargo run
```
On first start, the root admin credentials are printed to stderr. The service listens on `0.0.0.0:3000` by default. Override with `BIND_ADDRESS`.

### Docker
Build and run with Docker:
```bash
docker build -t sanshain .
docker run -p 3000:3000 -v sanshain-data:/data sanshain
```

Or use Docker Compose for local development:
```bash
docker compose up
```
The SQLite database is persisted in a Docker volume at `/data/sanshain.db`.

### Health Check
`GET /health` returns `200 OK` when the service is running. Used by Docker `HEALTHCHECK` and Kubernetes probes.

### Database
The service uses SQLite by default. The database file `sanshain.db` will be created automatically on the first run, and migrations will be applied. Each database adapter owns its migrations under `src/infrastructure/migrations/<db>/`.

---

Sanshain (Japanese for "Sunshine") is a specialized REST service designed to manage, split, and distribute OpenAPI specifications. It acts as a central repository that allows microservices to "provide" their full API definitions and clients to "require" only the specific snippets (endpoints and necessary DTOs) they actually use.

## Use Cases

- **Contract-First Development**: Services publish their API contracts, and clients consume exactly what they need.
- **Dependency Tracking**: Automatically track which clients are using which specific endpoints across different branches.
- **Impact Analysis**: Identify unused endpoints or missing requirements before they cause issues in production.
- **Client Generation**: Provide minimal, focused YAML snippets to clients for lightweight code generation in any language.
- **Web Dashboard**: Explore the dependency graph and API availability through a browser.

## Getting Started

### Prerequisites
- Rust (latest stable)
- SQLite (default)

### Installation

#### From Source
```bash
cargo build --release
./target/release/sanshain_service_bin
```

#### From GitHub Release
Download the `sanshain-linux-amd64` binary from the [Releases](../../releases) page:
```bash
chmod +x sanshain-linux-amd64
./sanshain-linux-amd64
```

#### Docker (GitHub Container Registry)
```bash
docker pull ghcr.io/<owner>/sanshainservice:latest
docker run -p 3000:3000 -v sanshain-data:/data ghcr.io/<owner>/sanshainservice:latest
```

Or use the `Dockerfile` and `docker-compose.yaml` included in each release.

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

### Running Tests
```bash
cargo test
```

## Changelog
See [CHANGELOG.md](CHANGELOG.md) for a detailed history of changes.
