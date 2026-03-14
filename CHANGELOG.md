# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.1] - 2026-03-14

### Changed
- `demo.sh` now registers multiple services and clients (with feature branches) to populate a realistic dependency graph for UI testing; verbose YAML output replaced with a concise line-count summary.

### Fixed
- Release workflow now builds with `x86_64-unknown-linux-musl` target so the binary runs correctly on Alpine-based Docker images (fixes "no such file or directory" error).

## [0.1.0] - 2025-03-13

### Added
- Initial release of Sanshain Service.
- `POST /provide` endpoint for uploading OpenAPI specifications.
- `GET /require` endpoint for retrieving per-endpoint YAML snippets with dependency tracking.
- `GET /report` and `GET /report/markdown` endpoints for dependency reports.
- OpenAPI splitting logic using the `openapiv3` crate.
- Immutable endpoint path policy with idempotency support.
- Feature branch fallback to `master` on `/require`.
- Long-polling support with configurable timeout on `/require`.
- Web Dashboard for browsing services, branches, and dependencies.
- SQLite database with automatic migrations.
- Integration tests for provide, require, and report flows.
- DDD Hexagonal/Onion Architecture with Domain, Application, and Infrastructure layers.
- `SpecRepository` trait (port) for persistence abstraction.
- `SqliteSpecRepository` adapter implementing the repository trait.
- Application service layer with use-case functions (`provide_spec`, `require_endpoint`, `generate_report`, `render_report_markdown`).
- Thin Axum handlers delegating to application services.
- Unit tests for OpenAPI splitting logic, application services (with mock repository), and markdown report rendering.
- Protected branch configuration: `main` and `master` are protected by default (immutable endpoints).
- Feature branch exception: on non-protected branches, endpoint DTOs can be updated freely.
- Admin API for managing protected branches (`GET/POST /admin/protected-branches`, `DELETE /admin/protected-branches/:pattern`).
- Admin API for data management: list and delete services (`GET/DELETE /admin/services/:name`), branches (`GET/DELETE /admin/services/:name/branches/:branch`), and clients (`GET/DELETE /admin/clients/:name`).
- AGPL-3.0 License.
- `GET /health` endpoint for liveness/readiness probes.
- Multi-stage `Dockerfile` (build in Rust Alpine, run in Alpine).
- `Dockerfile.release.template` for building Docker images from pre-compiled binaries during release.
- `docker-compose.release.template.yaml` for running the published Docker image with a database volume.
- `.dockerignore` to optimize Docker build context.
- SourceHut CI pipeline (`.build.yml`): build and test on Alpine Linux.
- Per-adapter migration structure (`src/infrastructure/migrations/sqlite/`) with `run_migrations()` on `SqliteSpecRepository`.
- Session-based authentication with Argon2 password hashing and 256-bit random session tokens.
- Automatic root admin user creation on first start (password printed to stderr).
- Auth endpoints: `POST /auth/login`, `POST /auth/logout`, `GET /auth/me`, `POST /auth/change-password`.
- Dev mode toggle (`POST /admin/settings/dev-mode`): when enabled, non-admin API endpoints are open without authentication; when disabled (default), all API endpoints require a valid session.
- Admin endpoints always require a valid admin session token.
- Configurable bind address via `BIND_ADDRESS` environment variable (default `0.0.0.0:3000`).
- Initial admin setup message now includes a URL (`http://localhost:3000/admin.html`) for changing the password.
- Upgraded all dependencies to latest releases: axum 0.7→0.8, sqlx 0.7→0.8, tower 0.4→0.5, tower-http 0.5→0.6, rand 0.8→0.9, openapiv3 2.0→2.2, argon2 0.5→0.6.0-rc.7.
- Admin dashboard now fetches and sends CSRF tokens for all state-changing requests (POST/DELETE).
- `api.yaml` — Hand-written OpenAPI 3.0.3 specification for the client-facing `/provide` and `/require` endpoints, intended as the contract for all client plugins (Maven, Gradle, Cargo, Go, npm, etc.).

#### API Token Management
- API token management for programmatic/CI access: `POST /auth/tokens` (create), `GET /auth/tokens` (list), `DELETE /auth/tokens/{id}` (revoke).
- Tokens use `san_` prefix (e.g., `san_a1b2c3...`) for easy identification; only SHA-256 hash stored in DB.
- Raw token shown only once at creation time; tokens have configurable expiry (1–3650 days, default 365).
- Auth middleware accepts both session cookies and `Authorization: Bearer san_...` API tokens.
- User dashboard page (`/dashboard`) with Askama server-side templates for token management UI.
- User account page (`/account.html`) — web frontend for self-registration, login, password change, and API token creation/management.
- Landing page (`/index.html`) with service info, version display, and links to Admin Dashboard, Account, and Service Overview pages.
- `GET /version` endpoint returning the service version from `Cargo.toml`.
- One-time token display modal with copy-to-clipboard and Maven `settings.xml` usage hint.
- Introduced Askama 0.13 for server-side HTML templating (compile-time checked, auto-escaped).

#### User Management
- Local user self-registration (`POST /auth/register`) — disabled by default, admin enables via `POST /admin/settings/local-users`.
- Registered users require admin approval before login (prevents unauthorized access).
- Admin endpoints for user management: list (`GET /admin/users`), approve (`POST /admin/users/{id}/approve`), delete (`DELETE /admin/users/{id}`).
- `GET /admin/settings/local-users` to check local users setting status.
- Admin dashboard UI: Local User Registration toggle, Users list with approve/delete actions and status badges (admin, approved, pending).

#### Observability
- Added structured logging for all failed REST endpoint calls (warn-level for client errors, error-level for internal errors) to aid debugging.
- Added HTTP request/response tracing via `tower-http` `TraceLayer` for full request lifecycle visibility.
- Default log level changed from `debug` to `info`; configurable via `RUST_LOG` environment variable (e.g., `RUST_LOG=sanshain_service=debug,tower_http=debug`).
- Service overview page now dynamically discovers all provided services and branches instead of only querying the `main` branch.
- Client drill-down view in service page: browse clients → branches → endpoints → YAML content, with search/filter support.
- Service drill-down view: browse services → branches → endpoints (with used/unused status and client counts) → paginated YAML preview, with search/filter support.
- Paginated YAML viewer for large OpenAPI files (80 lines per page) used in both service and client YAML views.
- New admin API endpoints: `GET /admin/clients/{name}/branches` and `GET /admin/clients/{name}/branches/{branch}/endpoints` for client dependency drill-down.

#### Graph View
- Redesigned dependency graph: unified node model where services that are also clients appear as a single node (no artificial client/service split).
- Top-down tree layout (`graph TD`) for clearer hierarchy visualization.
- Cycle detection with red-colored edges and a warning banner when circular dependencies exist.
- Detailed view toggle: labels each edge with endpoint path and HTTP method.
- Color-coded legend distinguishing client-only, service-only, and dual-role nodes.
- Improved mermaid theming with custom colors, fonts, and stroke styles.

#### DevOps
- GitHub Actions CI workflow: build and test on every push and pull request.
- GitHub Actions Release workflow: manually triggered via GitHub UI to create a release with Linux binary, Dockerfile, docker-compose.yaml, and CHANGELOG.
- Docker image built from release template (pre-compiled binary) and pushed to GitHub Container Registry (ghcr.io) on release.
- Release includes a ready-to-use `Dockerfile` and `docker-compose.yaml` (with correct image version) generated from templates.
- README updated with initial admin password setup procedure and installation options (source, binary, Docker).
