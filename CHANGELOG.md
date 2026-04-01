# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.5.0]

### Added
- **Web frontend modularisation (Phase 1)**: extracted shared Askama layout template (`templates/layout.html`) with nav, footer, and common CSS/JS. Landing page (`index.html`) and dashboard now extend the layout. Created `static/js/common.js` with shared fetch helpers, CSRF token management, HTML escaping, and confirm modal logic. Refactored `admin.html`, `account.html`, and `service.html` to use `common.js` instead of duplicating ~60 lines of JS each.
- **Endpoint pruning on provide**: when a new spec is uploaded via `POST /provide`, endpoints present in the previous spec but missing from the new one are now removed. On protected branches, removed endpoints are soft-deleted (marked `deleted`) so that re-introducing them later is rejected as a contract violation. On feature branches, removed endpoints are hard-deleted and can be freely re-added.
- **Stale dependency pruning**: dependency rows are now timestamped with `last_seen_at` (updated on every `/require` or `/require-bundle` call). A background task (hourly, alongside branch cleanup) deletes dependency rows older than a configurable max-age (default 30 days). Admins can configure via `GET/POST /admin/settings/dependency-max-age` and trigger immediate cleanup via `POST /admin/settings/dependency-cleanup`.

### Fixed
- **Duplicate dependencies**: added UNIQUE constraint on the dependencies table and `INSERT OR IGNORE` / `ON CONFLICT DO NOTHING` to prevent duplicate endpoint entries when clients re-register the same dependency.
- **Graph branch selector**: replaced hardcoded `branch=main` in the dependency graph with a dynamic branch dropdown populated from all known service branches, fixing "No dependencies found" when the branch name differs.
- **Node.js 20 deprecation warnings**: set `FORCE_JAVASCRIPT_ACTIONS_TO_NODE24=true` in the release workflow to silence GitHub Actions Node.js 20 deprecation annotations.

## [0.4.4]

### Fixed
- **CI build cache fix**: removed `target/` directory from the cargo cache to prevent stale proc-macro artifacts (`zerofrom_derive`) from causing cross-compilation failures.

## [0.4.3]

### Fixed
- **aarch64 cross-compilation CI fix**: replaced manual musl cross-compiler download (musl.cc is unreliable) with `cross-rs/cross`, the standard Rust cross-compilation tool that handles all toolchains via Docker containers.

## [0.4.2]

### Changed
- **Release workflow tag trigger**: the GitHub Actions release workflow now triggers automatically on `vX.X.X` version tags in addition to manual `workflow_dispatch`.

## [0.4.1] - 2026-04-01

### Fixed
- **Cross-compilation fix**: aarch64-unknown-linux-musl builds now use the correct musl cross-compiler toolchain instead of glibc, fixing linker errors with undefined symbols (`open64`, `stat64`, etc.).

## [0.4.0] - 2026-04-01

### Added
- **Branch max-age auto-cleanup**: non-protected branches are automatically deleted after a configurable period of inactivity (default: 30 days). A background task runs hourly. Admins can configure the max-age via `GET/POST /admin/settings/branch-max-age` and trigger immediate cleanup via `POST /admin/settings/branch-cleanup`.

### Fixed
- **Gzip request decompression**: the server now decompresses gzip-encoded request bodies (`Content-Encoding: gzip`), fixing failures when clients (e.g., Maven plugin) upload specs with compression enabled.

### Changed
- **Multi-architecture release builds**: GitHub Release now publishes Linux binaries for both `x86_64` and `aarch64`. Docker images are built as multi-platform manifests (`linux/amd64`, `linux/arm64`) and pushed to GHCR. Windows and macOS are not supported.
- **Release binary naming**: binaries use standard architecture names (`sanshain-linux-x86_64`, `sanshain-linux-aarch64`) instead of Docker-style `amd64`/`arm64`.

## [0.3.0]

### Added
- **`POST /require-bundle` endpoint**: request multiple endpoints from a single service in one call and receive a single merged OpenAPI YAML with deduplicated schemas/components. Solves the problem of duplicate DTOs when clients (e.g., Java/Maven) require multiple endpoints that share the same models.
- **`sanshain.yaml` client configuration format**: documented standard config file format for client plugins (Maven, Gradle, Cargo, etc.) covering provide, require, and require-bundle settings. See `docs/sanshain-yaml.md`.
- **Server-side gzip compression**: responses are automatically gzip-compressed when clients send `Accept-Encoding: gzip`. Enabled via `tower-http` `CompressionLayer`. The `compression: true` option in `sanshain.yaml` is now functional.


## [0.2.0] - 2026-03-26

### Changed
- **OpenAPI splitting now includes only referenced schemas**: each endpoint snippet contains only the `components/schemas` (and other component types) that are actually referenced by that operation, resolved transitively. Previously all schemas were included in every snippet.

### Added
- **LDAP authentication support**: new Auth Mode selector (Dev / Local / LDAP) on the admin dashboard.
- `AuthMode` enum and `LdapConfig` model in the domain layer; `AuthProvider` port trait for pluggable authentication.
- `LdapAuthProvider` adapter (ldap3 crate) with bind-and-search authentication and group-based admin detection.
- `LocalAuthProvider` adapter wrapping existing Argon2 password verification.
- Auth-mode and LDAP config CRUD services in the application layer (`get_auth_mode`, `set_auth_mode`, `get_ldap_config`, `set_ldap_config`, `test_ldap_connection`, `login_with_provider`).
- `GET /api/admin/auth-config`, `PUT /api/admin/auth-config`, and `POST /api/admin/auth-config/test` admin API endpoints.
- Login handler dispatches to the active auth provider; LDAP users are auto-provisioned as shadow accounts.
- Admin UI: Authentication section with radio buttons, LDAP configuration form, and "Test Connection" button.
- Bind password is redacted (`****`) in API responses; re-submitting `****` preserves the stored password.
- Unit tests for auth mode, LDAP config validation, provider dispatch (mock), and connection testing.
- Integration test for auth-config API endpoints (CRUD, validation, auth requirement).
- PostgreSQL database backend as an alternative to SQLite, selectable via `DATABASE_URL` environment variable.
- `PostgresSpecRepository` adapter with full feature parity to the SQLite adapter.
- `DatabaseRepo` enum-based dispatch layer for runtime database backend selection.
- PostgreSQL migration script (`src/infrastructure/migrations/postgres/`).
- Admin dashboard "Database Configuration" section showing current backend and (masked) connection URL.
- `GET /admin/settings/database` endpoint returning current database backend info.
- Documentation: PostgreSQL setup guide in `docs/getting-started.md` (Docker, Docker Compose, bare metal) and Database Configuration section in `docs/administration.md`.

## [0.1.1] - 2026-03-14

### Added
- `ai/ai-rules.md` with Rust and service development best practices for AI-assisted development.
- `docs/` Detailed User doc started.
- `docs/user-guide.md` — Day-to-day usage guide covering concepts, web UI navigation, dependency reports, and build tool plugin overview (links to individual tools will be added as they become available).

### Changed
- `demo.sh` now registers multiple services and clients (with feature branches) to populate a realistic dependency graph for UI testing; verbose YAML output replaced with a concise line-count summary.
- `ai/plan.md` consolidated from ~200 lines to ~50 lines; completed items grouped into a summary, only open tasks listed individually.

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
