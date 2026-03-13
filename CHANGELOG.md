# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.0] - 2025-03-10

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
- `docker-compose.yaml` for local development with persistent SQLite volume.
- `.dockerignore` to optimize Docker build context.
- SourceHut CI pipeline (`.build.yml`): build and test on Alpine Linux.
- Per-adapter migration structure (`src/infrastructure/migrations/sqlite/`) with `run_migrations()` on `SqliteSpecRepository`.
- Session-based authentication with Argon2 password hashing and 256-bit random session tokens.
- Automatic root admin user creation on first start (password printed to stderr).
- Auth endpoints: `POST /auth/login`, `POST /auth/logout`, `GET /auth/me`, `POST /auth/change-password`.
- Dev mode toggle (`POST /admin/settings/dev-mode`): when enabled, non-admin API endpoints are open without authentication; when disabled (default), all API endpoints require a valid session.
- Admin endpoints always require a valid admin session token.
- Configurable bind address via `BIND_ADDRESS` environment variable (default `0.0.0.0:3000`).

- `api.yaml` — Hand-written OpenAPI 3.0.3 specification for the client-facing `/provide` and `/require` endpoints, intended as the contract for all client plugins (Maven, Gradle, Cargo, Go, npm, etc.).

### Changed
- Initial admin setup message now includes a URL (`http://localhost:3000/admin.html`) for changing the password.
- Upgraded all dependencies to latest releases: axum 0.7→0.8, sqlx 0.7→0.8, tower 0.4→0.5, tower-http 0.5→0.6, rand 0.8→0.9, openapiv3 2.0→2.2, argon2 0.5→0.6.0-rc.7.

### Fixed
- Admin dashboard now fetches and sends CSRF tokens for all state-changing requests (POST/DELETE).
- Fixed dev mode toggle payload key (`dev_mode` → `enabled`) to match backend API.
- Fixed change-password form field name (`current_password` → `old_password`) to match backend API.

### User Management
- Local user self-registration (`POST /auth/register`) — disabled by default, admin enables via `POST /admin/settings/local-users`.
- Registered users require admin approval before login (prevents unauthorized access).
- Admin endpoints for user management: list (`GET /admin/users`), approve (`POST /admin/users/{id}/approve`), delete (`DELETE /admin/users/{id}`).
- `GET /admin/settings/local-users` to check local users setting status.
- Admin dashboard UI: Local User Registration toggle, Users list with approve/delete actions and status badges (admin, approved, pending).

### DevOps
- GitHub Actions CI workflow: build and test on every push and pull request.
- GitHub Actions Release workflow: manually triggered via GitHub UI to create a release with Linux binary, Dockerfile, docker-compose.yaml, and CHANGELOG.
- Docker image built and pushed to GitHub Container Registry (ghcr.io) on release.
- README updated with initial admin password setup procedure and installation options (source, binary, Docker).
