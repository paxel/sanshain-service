# Sanshain Service — Implementation Plan

## Completed

The following major milestones have been delivered and are fully functional:

- **Core API**: `POST /provide` (with idempotency, immutability on protected branches, feature-branch override), `GET /require` (with long-polling, feature-branch fallback, dependency tracking), `GET /report` + `/report/markdown`.
- **Architecture**: DDD Hexagonal/Onion (Domain → Application → Infrastructure → Presentation). SQLite adapter with auto-migrations.
- **OpenAPI Splitting**: Per-endpoint YAML snippets with shared schema extraction (`openapiv3` crate).
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

### Ecosystem & Tooling (Client Plugins)
- [ ] Maven plugin (`sanshain-provide`, `sanshain-require`).
- [ ] Gradle plugin (`sanshainProvide`, `sanshainRequire`).
- [ ] Rust (`cargo-sanshain` or `build.rs`).
- [ ] Go (`go generate`), Python, JS/TS, Swift, Ruby, PHP, C/C++ integrations.

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
- [ ] Create a shared HTML layout partial (`templates/layout.html`) with nav, header, footer, common CSS/JS.
- [ ] Convert `admin.html`, `account.html`, `service.html` to Askama templates extending the layout.
- [ ] Extract reusable JS modules (fetch helpers, CSRF, toast notifications) into `static/js/common.js`.

#### Phase 2 — Evaluate full UI framework (future)
- [ ] Evaluate lightweight options (htmx, Alpine.js, Leptos) for progressive enhancement.
- [ ] Decision record in `docs/adr/` once a choice is made.

### Administration (other)
- [ ] Branch max-age auto-cleanup.
- [ ] Stale dependency pruning.
- [ ] Background maintenance job for expiration/cleanup.

### Web Frontend (other)
- [ ] SSE for `/require` long-polling and live updates.
- [ ] WebSocket support (if bidirectional real-time needed).

### Documentation
- [x] Detailed hands-on user documentation (in `docs/`).
