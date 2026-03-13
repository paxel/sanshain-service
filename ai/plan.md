# Implementation Plan: SanShain Service

SanShain is a service designed to manage and distribute OpenAPI specifications for microservices, tracking client-service dependencies and generating usage reports.

## 1. Project Initialization & Infrastructure
- [x] Initialize Rust project with `axum` or `actix-web` for the REST API.
- [x] Set up a database (e.g., PostgreSQL or MongoDB) to store:
    - Service definitions.
    - Branch-specific OpenAPI specs.
    - Per-endpoint splits (YAML + DTOs).
    - Client registration and endpoint usage tracking.
    - **Protected Branch Configuration**.
- [x] **DDD Hexagonal / Onion Architecture**:
    - [x] Refactor the project structure to follow DDD principles (Domain, Application, Infrastructure layers).
    - [x] Decouple database logic from Axum handlers using Port/Adapter pattern.
- [x] Implement OpenAPI parsing and splitting logic (likely using `openapiv3` crate).
- [x] **Flexible DB Layer**:
    - [x] Moved SQLite migrations into `src/infrastructure/migrations/sqlite/` (adapter-owned).
    - [x] Added `run_migrations()` to `SqliteSpecRepository` so each adapter manages its own migrations.
    - [ ] Add PostgreSQL adapter with its own migrations in `src/infrastructure/migrations/postgres/`.

## 2. Core API Implementation

### 2.1. `POST /provide`
- [x] **Arguments**: `servicename`, `branch`, `openapi_yaml` (file or string).
- [x] **Logic**:
    1. Parse the full `openapi.yaml`.
    2. Split the specification into individual endpoints.
    3. Extract necessary DTOs (components/schemas) for each endpoint.
    4. Store the mapping (Service -> Branch -> Endpoint -> YAML) in the DB.
- [x] **Advanced /provide logic**:
    - [x] Idempotency: If the same YAML is uploaded for the same branch and the endpoints are identical, the request succeeds without modification (idempotency).
    - [x] **Conditional Immutability**:
        - [x] If an endpoint already exists for a given (service, branch, path, method) but the YAML content (DTOs/schema) has changed, the request must fail.
        - [x] **Feature Branch Exception**: This immutability only applies to **protected branches** (configurable via Admin). On feature branches, the latest version can be modified.
    - [x] Versioning via Path: The service relies on the user to change the path (e.g., `api/v1.0/users` to `api/v2.0/users`) when DTOs change, as the endpoint definition for a specific path is immutable within a branch.

### 2.2. `GET /require`
- [x] **Arguments**: `clientname`, `servicename`, `branch`, `path`, `method`, `timeout` (optional).
- [x] **Logic**:
    1. Retrieve the specific YAML snippet for the requested endpoint from the DB.
    2. **Wait Logic**: If the snippet is not yet available and `timeout` is provided, the server should wait (long-polling) until it appears or the timeout expires.
    3. **Feature Branch Fallback**: If the snippet is not found on a feature branch, fallback to the `master` version.
    4. **Client Versioning Workaround**: Allow manual entering of a client version. The master will then be forced to implement the client version.
    5. Record the client's dependency: "Client X uses Endpoint Y on Service Z (Branch B)".
    6. Return the YAML to the client, or a 404 (or 204/timeout indicator) if it doesn't appear in time.

### 2.3. `GET /report`
- [x] **Arguments**: `branch`.
- [x] **Logic**:
    1. Query the DB for all client-service-endpoint relationships on the given branch.
    2. Generate a JSON report representing the dependency graph.
    3. Include validation:
        - Identify endpoints provided but never required (unused).
        - Identify endpoints required by clients but not provided in the current branch (missing).
- [x] **Markdown Report**:
    - [x] `GET /report/markdown` implemented to provide a human-readable version of the report.

## 3. Web UI
- [x] Implement a simple frontend to navigate and search for services and clients.
- [x] Service overview: List branches and endpoints.
- [x] Client overview: List used services and endpoints (integrated in branch view).
- [x] Set the root URL (`/`) to redirect to the dashboard.
- [x] Dependency graph visualization (further refinement).

## 4. Advanced Features (Optional/Phase 2)
- [ ] Add a "Release" flag to client requirements to distinguish between development/feature branch usage and production-ready dependencies.

## 4. Phase 3: Ecosystem & Tooling (Maven & Gradle Plugins)
All client-side implementations (plugins, CLIs, hooks) must support a **timeout/retry mechanism** (configurable timeout and retry interval) when calling `require`, to handle asynchronous build orders where a consumer might build before a provider.

- [ ] **Maven Plugin**:
    - [ ] `sanshain-provide`: Goal to upload a service's full OpenAPI spec to the SanShain service during the build (likely in `package` or `deploy` phase).
    - [ ] `sanshain-require`: Goal to download required endpoint snippets before the code generation phase. Should support a configuration file listing required endpoints.
- [ ] **Gradle Plugin**:
    - [ ] `sanshainProvide`: Task to upload the OpenAPI spec.
    - [ ] `sanshainRequire`: Task to download required snippets. Configurable to run before `openApiGenerate` (if using the OpenAPI Generator Gradle Plugin).
- [ ] **Rust**:
    - [ ] `cargo-sanshain` custom command or `build.rs` integration for `provide` and `require`.
- [ ] **C / C++**:
    - [ ] `Makefile` or `CMake` integration (likely via `curl` and `jq` or a small CLI tool) to fetch snippets before compilation.
    - [ ] **Python-based Buildchains**:
        - [ ] **Conan**: Custom generator or `conanfile.py` hook to manage OpenAPI dependencies.
        - [ ] **Meson**: `run_command` or custom script integration to fetch snippets during configuration.
        - [ ] **SCons**: Custom builder for downloading and managing OpenAPI snippets.
- [ ] **Go**:
    - [ ] `go generate` integration for downloading required snippets.
- [ ] **Python**:
    - [ ] `pip` / `poetry` / `hatch` hook or a standalone script to fetch dependencies.
- [ ] **JavaScript / TypeScript**:
    - [ ] `npm` / `yarn` / `pnpm` `preinstall` or `prebuild` scripts.
- [ ] **Swift**:
    - [ ] `Swift Package Manager` (SPM) plugin for `provide` and `require`.
    - [ ] `Xcode` Build Phases integration for downloading snippets before compilation.
- [ ] **Ruby**:
    - [ ] `Rake` tasks or `Bundler` hooks to fetch OpenAPI snippets.
- [ ] **PHP**:
    - [ ] `Composer` scripts (`pre-install-cmd`, `pre-update-cmd`) for dependency management.
- [ ] **Kotlin / Mobile**:
    - [ ] `Gradle` (covered in general Java section, but specifically for Kotlin Multiplatform or Android).

## 5. Phase 4: Deployment & Infrastructure
- [x] **Docker**:
    - [x] Create a multi-stage `Dockerfile` to optimize image size (build in Rust image, run in Alpine).
    - [x] `docker-compose.yaml` for local development including the service and a persistent volume for the SQLite DB.
    - [x] `.dockerignore` to optimize build context.
- [x] **Health Endpoint**:
    - [x] `GET /health` returns `200 OK` for liveness/readiness probes.
- [ ] **Kubernetes**:
    - [ ] Standard manifests: `Deployment`, `Service`, `Ingress`.
    - [ ] `ConfigMap` for environment variables and `Secret` for sensitive data (if any).
    - [x] Liveness and Readiness probes using a `/health` endpoint.
- [ ] **Helm Chart**:
    - [ ] Package the Kubernetes manifests into a reusable Helm chart for different environments (staging, production).
- [x] **CI/CD**:
    - [x] **GitHub Actions**: Pipeline to build, test, and push Docker images to a registry (GHCR).
    - [x] Automated database migrations during deployment (handled by `run_migrations()` on startup).
- [ ] **Observability**:
    - [ ] **Metrics**: Integrate `prometheus` exporter for tracking request counts, latencies, and DB pool stats.
    - [ ] **Logging**: Ensure structured JSON logging for better log aggregation (ELK/Loki).
    - [ ] **Tracing**: OpenTelemetry support for distributed tracing (optional but recommended for microservices).

## 6. Documentation & Guidelines
- [x] Create `README.md` in the service root explaining use cases, API usage, and example reports.
- [x] Finalize `.junie/guidelines.md` with build, test, and development instructions specific to SanShain.
- [x] Add AGPL-3.0 License.
- [x] Add `CHANGELOG.md` following Keep a Changelog format, linked from README.
- [x] Add guidelines to maintain DDD structure, keep README and CHANGELOG updated, and keep all units tested.

## 7. Phase 5: Administration & Cleanup
- [x] **Admin Page with Authentication**:
    - [x] Session-based authentication with Argon2 password hashing and random session tokens.
    - [x] Automatic root admin user creation on first start (password printed to stderr).
    - [x] Auth endpoints: login, logout, me, change-password.
    - [x] Dev mode toggle: admin can open/close non-admin API endpoints without auth.
    - [ ] Web dashboard for administrative tasks.
    - [x] **Protected Branch Configuration UI**: Toggle which branches are considered protected (immutable).
- [ ] **Data Management**:
    - [x] Delete branches, clients, and services via the admin API.
    - [x] List services, branches, and clients via the admin API.
    - [ ] Configure a maximum age for branches before they are automatically cleaned out.
- [ ] **Auto-Cleanup Logic**:
    - [ ] If a client stops requesting a URL (stale dependency), remove it from the database after a configurable period.
    - [ ] If a service stops providing a URL in its latest OpenAPI upload, remove it from the DB for that branch (pruning).
- [ ] **Maintenance Tasks**:
    - [ ] Implement a background job or periodic task to handle branch expiration and dependency cleanup.

## 8. Verification
- [x] Create integration tests for `provide`, `require`, and `report` flows.
- [x] Verify that the OpenAPI splitting logic correctly handles shared schemas/DTOs.
- [x] Unit tests for OpenAPI splitting logic (`openapi::tests`).
- [x] Unit tests for application services with mock repository (`application::services::tests`).
- [x] Unit tests for markdown report rendering.

## 9. Web Frontend Architecture Decisions

### 9.1 REST-Only Communication
- The service currently uses only REST endpoints. This is sufficient for the current scope.
- SSE could replace long-polling in `/require` for better efficiency.

### 9.2 Static Frontend Security
- Static HTML + JS is acceptable for the current scope.
- [ ] Add security headers: CSP, X-Content-Type-Options, X-Frame-Options, Referrer-Policy.
- [ ] Add CSRF protection for state-changing endpoints (POST/DELETE).
- [ ] Consider server-side templates (Tera/Askama) only if XSS surface becomes a concern.

### 9.3 TLS Strategy
- **Decision**: Rely on a reverse proxy (Traefik/Caddy/Nginx) for TLS termination.
- [ ] Add reverse proxy configuration to `docker-compose.yaml` for local HTTPS.
- [ ] Document TLS setup in README for production deployments.

### 9.4 Future: Real-Time Communication
- If the UI grows in complexity:
  - [ ] Implement SSE for server-to-client push (e.g., `/require` wait, live report updates).
  - [ ] Add WebSocket support only when bidirectional real-time communication is needed.
  - A full event bus (NATS/RabbitMQ) is not warranted until the architecture becomes multi-service.
- **Security for WebSockets/SSE**:
  - Authenticate on connection upgrade (validate session token).
  - Validate `Origin` header to prevent cross-site hijacking.
  - Rate-limit connections and messages.
  - Sanitize all inbound WebSocket messages.
  - Use `wss://` via reverse proxy TLS.
  - Implement idle connection timeouts.