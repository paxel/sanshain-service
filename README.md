# SanShain Service

SanShain is a service designed to manage and distribute OpenAPI specifications for microservices, tracking client-service dependencies and generating usage reports.

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

**Note:** SanShain enforces an **Immutable Endpoint Path** policy within a branch. If the DTO/schema for an existing endpoint changes, you must increment the version in the path (e.g., `/api/v1/users` to `/api/v2/users`). Changes to the same path that alter the DTO will be rejected with `409 Conflict`.

### 2. `GET /require`
Request the OpenAPI snippet for a specific endpoint and record the dependency.

**Query Parameters:**
- `clientname`: Name of the client service.
- `servicename`: Name of the target service.
- `branch`: Branch name.
- `path`: Endpoint path.
- `method`: HTTP method (GET, POST, etc.).
- `timeout` *(optional)*: Long-polling timeout in seconds. If the endpoint is not yet available, the server will poll until it appears or the timeout expires.

**Feature Branch Fallback:** If the endpoint is not found on a non-protected (feature) branch, SanShain automatically falls back to protected branches (e.g., `main`, `master`).

**Example:**
`GET /require?clientname=WebClient&servicename=UserService&branch=main&path=/users&method=GET`
`GET /require?clientname=WebClient&servicename=UserService&branch=feature/xyz&path=/users&method=GET&timeout=30`

### 3. Protected Branches (Admin)
Manage which branches enforce immutable endpoint paths. By default, `main` and `master` are protected. On non-protected (feature) branches, endpoint DTOs can be freely updated.

- `GET /admin/protected-branches` — List all protected branch patterns.
- `POST /admin/protected-branches` — Add a pattern: `{"pattern": "release"}`.
- `DELETE /admin/protected-branches/:pattern` — Remove a pattern.

### 4. Data Management (Admin)
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

### Running the service
```bash
cargo run
```
The service listens on `0.0.0.0:3000` by default.

### Database
The service uses SQLite. The database file `sanshain.db` will be created automatically on the first run, and migrations will be applied.

---

SanShain (Japanese for "Sunshine") is a specialized REST service designed to manage, split, and distribute OpenAPI specifications. It acts as a central repository that allows microservices to "provide" their full API definitions and clients to "require" only the specific snippets (endpoints and necessary DTOs) they actually use.

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
```bash
cargo build --release
```

### Running Tests
```bash
cargo test
```

## Changelog
See [CHANGELOG.md](CHANGELOG.md) for a detailed history of changes.
