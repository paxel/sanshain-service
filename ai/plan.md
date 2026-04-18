# Sanshain Service — Implementation Plan

## Completed

The following major milestones have been delivered and are fully functional:

- **Core API**: `POST /provide` (with idempotency, immutability on protected branches, feature-branch override), `GET /require` (with long-polling, feature-branch fallback, dependency tracking), `GET /report` + `/report/markdown`.
- **Architecture**: DDD Hexagonal/Onion (Domain → Application → Infrastructure → Presentation). SQLite adapter with auto-migrations.
- **OpenAPI Splitting**: Per-endpoint YAML snippets with per-operation schema filtering — each snippet includes only the schemas/components transitively referenced by that operation (`openapiv3` crate).
- **Authentication & Authorization**: Session-based auth (Argon2), root admin bootstrap, dev-mode toggle, local user registration with admin approval, API tokens (`san_` prefix, SHA-256 hashed).
- **Admin API**: Protected branches CRUD, service/branch/client CRUD (cascade delete), user management (list/approve/delete), settings (dev-mode, local-users).
- **Web UI**: Landing page, service overview (drill-down to endpoints + YAML), client overview, admin dashboard, account page (login/register/tokens), dependency graph (Mermaid, cycle detection, detail toggle).
- **DevOps**: Multi-stage Dockerfile, release Dockerfile template, docker-compose template, GitHub Actions CI + Release workflow (GHCR push), `.dockerignore`, health endpoint.
- **Security**: CSP/X-Content-Type-Options/X-Frame-Options headers, CSRF tokens, Askama server-side templates.
- **Observability**: Structured logging (warn/error for failures), `tower-http` TraceLayer, configurable `RUST_LOG`.
- **Testing**: Integration tests (provide/require/report), unit tests (OpenAPI splitting, application services with mocks, markdown rendering, token services).
- **Documentation**: README, CHANGELOG, `api.yaml` (OpenAPI 3.0.3 contract), demo script.

## Open

### Infrastructure
- [x] PostgreSQL adapter with migrations in `src/infrastructure/migrations/postgres/`.

### Require Bundle (Merged Multi-Endpoint Require)
- [x] `merge_endpoint_yamls` function in `openapi.rs` — merges per-endpoint YAML snippets with deduplicated schemas.
- [x] `require_bundle` service function in `services.rs` — resolves multiple endpoints, records dependencies, returns merged YAML.
- [x] `POST /require-bundle` handler and route in `main.rs`.
- [x] Unit tests (openapi merge, service bundle logic).
- [x] Integration test (provide → require-bundle → verify merged spec).
- [x] `sanshain.yaml` client configuration format specification (`docs/sanshain-yaml.md`).
- [x] CHANGELOG, README, plan.md updates.

### Server-Side Compression
- [x] Enable `compression-gzip` feature on `tower-http` in `Cargo.toml`.
- [x] Add `CompressionLayer` to Axum router in `main.rs`.
- [x] Integration tests for gzip compression.
- [x] CHANGELOG update.

### Multi-Architecture Builds
- [x] Release workflow builds Linux binaries for x86_64 and aarch64 (matrix strategy with cross-compilation).
- [x] Multi-platform Docker images (linux/amd64, linux/arm64) via Docker Buildx + QEMU.
- [x] Dockerfile.release.template uses `TARGETARCH` for architecture-specific binary selection.
- [x] Release binaries use standard architecture names (`x86_64`, `aarch64`) instead of Docker-style `amd64`/`arm64`.

### Deployment
- [ ] Kubernetes manifests (Deployment, Service, Ingress, ConfigMap, Secret).
- [ ] Helm chart.
- [ ] Reverse proxy TLS config + documentation.

### Observability (Advanced)
- [ ] Prometheus metrics exporter.
- [ ] Structured JSON logging.
- [ ] OpenTelemetry tracing.

### Authentication — External Auth Support (LDAP first)

The admin settings page gains an **Auth Mode** selector with three modes:
1. **Dev Mode** — no authentication required (existing).
2. **Local Users** — built-in user management with Argon2 passwords (existing).
3. **LDAP** — delegate authentication to an external LDAP/AD server (new).

#### Domain Layer
- [x] Add `AuthMode` enum (`Dev`, `Local`, `Ldap`) to `models.rs`.
- [x] Add `LdapConfig` model (server URL, bind DN, bind password, base DN, user filter, group filter, admin group, TLS toggle).
- [x] Define `AuthProvider` port trait in `ports.rs` with `authenticate(username, password) -> Result<AuthenticatedUser>` and `test_connection() -> Result<()>`.

#### Infrastructure Layer
- [x] Add `ldap3` crate dependency.
- [x] Implement `LdapAuthProvider` adapter in `src/infrastructure/ldap_provider.rs` implementing the `AuthProvider` port.
- [x] Implement `LocalAuthProvider` adapter wrapping existing Argon2 logic.
- [x] On LDAP login success, auto-provision a local `User` row (shadow account) so sessions/tokens work unchanged.

#### Application Layer
- [x] Add auth-mode setting helpers (`get_auth_mode`, `set_auth_mode`) in `services.rs`.
- [x] Add LDAP config CRUD helpers (store as JSON in `settings` table).
- [x] Refactor `login` service to dispatch to the active `AuthProvider` based on current auth mode.
- [x] Add `test_ldap_connection` service function.

#### Presentation / API
- [x] Add `GET /api/admin/auth-config` and `PUT /api/admin/auth-config` endpoints.
- [x] Add `POST /api/admin/auth-config/test` endpoint (test LDAP connectivity).
- [x] Update login handler to use the provider-based flow.

#### Admin UI — Auth Settings Panel
- [x] Add "Authentication" section to admin page with auth-mode radio buttons (Dev / Local / LDAP).
- [x] Show LDAP configuration form (server, bind DN, base DN, filters, TLS) when LDAP is selected.
- [x] Add "Test Connection" button that calls the test endpoint.
- [x] Persist changes via the new API endpoints.

#### Testing
- [x] Unit tests for `AuthProvider` dispatch logic (mock LDAP provider).
- [x] Unit tests for LDAP config validation (missing fields, bad URL format).
- [x] Integration tests for auth-config API endpoints.
- [x] Integration test: login with local provider, login with mock LDAP provider.

#### Documentation
- [x] Update `docs/administration.md` with LDAP configuration instructions.
- [x] Update `README.md` with LDAP feature mention.
- [x] Update `CHANGELOG.md`.

### Web Frontend — Modularisation

The static HTML files are growing (admin.html 674 LOC, service.html 840 LOC). Before adding more UI complexity, split into manageable pieces.

#### Phase 1 — Extract shared layout & components (server-side)
- [x] Create a shared HTML layout partial (`templates/layout.html`) with nav, header, footer, common CSS/JS.
- [x] Convert `index.html` and `dashboard.html` to Askama templates extending the layout; refactor `admin.html`, `account.html`, `service.html` to use shared JS.
- [x] Extract reusable JS modules (fetch helpers, CSRF, toast notifications) into `static/js/common.js`.

#### Phase 2 — htmx admin dashboard ✅
- [x] Migrated admin.html to htmx: Askama template with server-rendered HTML fragments, ~80% JS eliminated.
- [x] Added `/fragments/admin/*` routes for all dynamic sections (users, services, clients, settings, auth config, cleanup).
- [x] Integration test for fragment endpoints.
- [ ] Decision record in `docs/adr/` once a choice is made.

### Administration (other)
- [x] Branch max-age auto-cleanup (default 30 days, configurable via admin API, hourly background task, protects protected branches).
- [x] Endpoint pruning on provide (soft-delete on protected branches, hard-delete on feature branches, re-introduction rejected as contract violation).
- [x] Stale dependency pruning (time-based cleanup of `dependencies` rows via `last_seen_at` timestamp).
- [x] Background maintenance job for expiration/cleanup (branch cleanup runs hourly).

### Custom Dependency Graph Visualization
- [x] Custom dagre-based SVG graph as default view (MVP): topological layout, color-coded nodes, red cycle edges, hover tooltips, click-to-highlight, zoom/pan.
- [ ] Edge bundling / merge at endpoint entry points (polish phase).
- [ ] Export as PNG (polish phase).

### Web Frontend (other)
- [ ] SSE for `/require` long-polling and live updates.
- [ ] WebSocket support (if bidirectional real-time needed).

### 0.6.0 Features
- [x] Dry-run mode for `/provide`, `/require`, `/require-bundle` (validate without persisting).
- [x] Descriptive error messages for provide conflicts (409) and missing endpoints (404).
- [x] Phantom services/clients fix (only list entries with actual data).
- [x] YAML viewer copy & download buttons.
- [x] Graph view copy & download buttons (Mermaid code).
- [x] Documentation updates for 0.6.0 (README, api.yaml, user-guide, ci-integration, developer-guide).

### 0.7.0 Features — Backward Compatibility & Version History
- [x] Backward compatibility checker in `openapi.rs` (structural comparison: schemas, properties, types, response codes).
- [x] Protected branches now accept backward-compatible changes instead of rejecting all changes.
- [x] Endpoint version history: each compatible update on a protected branch records a version with YAML content and unified diff.
- [x] `GET /endpoint-versions` API endpoint for retrieving version history.
- [x] `endpoint_versions` migration for SQLite and PostgreSQL.
- [x] Domain model (`EndpointVersion`), port trait methods, and repository implementations.
- [x] Unit tests (backward-compatible allowed, breaking rejected, version recording).
- [x] Integration tests updated for new behavior.
- [x] CHANGELOG updated.
- [x] README.md updated with backward compatibility, endpoint-versions API, dark/light mode, demo.sh description.
- [x] `docs/user-guide.md` updated with backward compatibility, version history/diff viewer, dark mode sections.
### Documentation
- [x] Detailed hands-on user documentation (in `docs/`).
- [x] CI integration guide (`docs/ci-integration.md`) with dry-run examples and GitHub Actions workflow.
- [x] Updated screenshots for version history/diff viewer, dependency graph, and stale data banner added to `docs/user-guide.md`.

### 0.7.1 Features — Post-demo fixes
- [x] Fix admin page layout so config switches (e.g. dev mode) render immediately after login without a refresh.
- [x] Bump project version from 0.7.0 to 0.7.1 in `Cargo.toml` and update `CHANGELOG.md`.
- [x] Transactional specification updates: inserts, updates, and deletions in `/provide` are now atomic.
- [x] SQLite reliability tuning: enabled WAL mode, busy timeout (5s), and single writer pool for Mac Docker stability.

### Production Readiness & Security (Audit Findings)
- [x] Fix LDAP Injection vulnerability by escaping username in filters.
- [x] Resolve CSRF token memory leak (implement pruning/TTL).
- [x] Implement CSRF token expiration.
- [x] Bypass CSRF protection for API-token authenticated requests.
- [ ] Replace busy-wait long-polling with an event-driven mechanism (e.g., `tokio::sync::watch` or `broadcast`).
- [ ] Optimize OpenAPI splitting to include only necessary schemas (reduces DB bloat and increases performance).
- [ ] Replace manual date/time logic with `chrono`.
- [ ] Clean up `unwrap()` calls in critical paths.
