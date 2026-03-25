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

### Administration
- [ ] External auth delegation (LDAP, Kerberos, Keycloak).
- [ ] Branch max-age auto-cleanup.
- [ ] Stale dependency pruning.
- [ ] Background maintenance job for expiration/cleanup.

### Web Frontend
- [ ] SSE for `/require` long-polling and live updates.
- [ ] WebSocket support (if bidirectional real-time needed).

### Documentation
- [ ] Detailed hands-on user documentation (in `docs/`).
