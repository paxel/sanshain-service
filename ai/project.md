# Sanshain Service — Project Summary

## Purpose

Sanshain ("Sunshine" in Japanese) is a REST service that acts as a central repository for OpenAPI specifications. Microservices **provide** their full API definitions, and clients **require** only the specific endpoint snippets (with necessary DTOs) they need at build time. It tracks cross-service dependencies, detects unused or missing endpoints, and offers a web dashboard for exploring the dependency graph.

## Architecture

The project follows **DDD Hexagonal (Onion) Architecture**:

- **Domain** — Pure business models and port traits (no framework dependencies).
- **Application** — Use-case services that orchestrate domain logic.
- **Infrastructure** — Adapter implementations (SQLite/PostgreSQL repositories, auth providers).
- **Presentation** — Thin Axum HTTP handlers in `main.rs`.

## Key Features

- Upload and store OpenAPI specs per service/branch.
- Structural backward compatibility checking on protected branches.
- Split specs into per-endpoint YAML snippets for clients.
- Track client→endpoint dependencies across branches.
- Generate dependency reports (unused endpoints, missing requirements).
- Authentication: Dev mode, local users, or LDAP.
- API token management for CI/CD integration.
- Protected branches with soft-delete semantics.
- Automatic cleanup of stale branches and dependencies.
- Web dashboard with dependency graph visualization.

## File Map

### Source Code (`src/`)

| File | Purpose |
|---|---|
| `src/main.rs` | Axum server setup, route definitions, and thin HTTP handlers. |
| `src/openapi.rs` | Parsing and splitting OpenAPI YAML specs into per-endpoint snippets. |
| **Domain (`src/domain/`)** | |
| `src/domain/mod.rs` | Module declarations for domain layer. |
| `src/domain/models.rs` | Domain models: `User`, `Session`, `ApiToken`, `EndpointRecord`, `DependencyReport`, `AuthMode`, `LdapConfig`, etc. |
| `src/domain/ports.rs` | Port traits: `SpecRepository` (all persistence operations) and `AuthProvider` (pluggable authentication). |
| **Application (`src/application/`)** | |
| `src/application/mod.rs` | Module declarations for application layer. |
| `src/application/services.rs` | Use-case functions: spec upload/sync, dependency recording, report generation, auth flows, cleanup jobs. |
| **Infrastructure (`src/infrastructure/`)** | |
| `src/infrastructure/mod.rs` | Module declarations for infrastructure layer. |
| `src/infrastructure/database.rs` | Database connection setup and migration runner (SQLite/PostgreSQL). |
| `src/infrastructure/cached_repository.rs` | In-memory cache decorator (`CachedSpecRepository`) using `moka` with write-through invalidation. |
| `src/infrastructure/sqlite_repository.rs` | `SpecRepository` implementation for SQLite. |
| `src/infrastructure/postgres_repository.rs` | `SpecRepository` implementation for PostgreSQL. |
| `src/infrastructure/local_auth_provider.rs` | `AuthProvider` implementation for local username/password auth. |
| `src/infrastructure/ldap_provider.rs` | `AuthProvider` implementation for LDAP. |
| `src/infrastructure/migrations/` | SQL migrations for both SQLite and PostgreSQL. |

### Migrations (`src/infrastructure/migrations/`)

| File | Purpose |
|---|---|
| `sqlite/20240308000000_initial_schema.sql` | Initial SQLite schema (services, branches, endpoints, users, sessions, settings, etc.). |
| `sqlite/20240309000000_branch_updated_at.sql` | Adds `updated_at` tracking to branches. |
| `sqlite/20240311000000_endpoint_soft_delete.sql` | Adds soft-delete support for endpoints. |
| `sqlite/20240312000000_dependency_last_seen.sql` | Adds `last_seen_at` to dependencies for staleness tracking. |
| `postgres/` | Equivalent migrations for PostgreSQL. |

### Tests

| File | Purpose |
|---|---|
| `tests/integration_test.rs` | Integration tests for the full API (spec upload, require, reports, auth, admin). |
| `src/application/auth_service.rs` (tests) | Unit tests for password hashing, login, registration, change password. |
| `src/application/admin_service.rs` (tests) | Unit tests for protected branches, services, fallback branches, cleanup. |
| `src/application/spec_service.rs` (tests) | Unit tests for provide (insert/update/skip), dry run, compatibility, parsing. |

### Web Frontend

| File | Purpose |
|---|---|
| `templates/layout.html` | Base HTML layout (shared header/nav). |
| `templates/index.html` | Landing/login page. |
| `templates/dashboard.html` | Main dashboard view. |
| `templates/admin.html` | Admin panel page. |
| `templates/fragments/admin/*.html` | HTMX fragments for admin panel sections (users, services, branches, clients, auth config, etc.). |
| `static/admin.html` | Static admin page assets. |
| `static/account.html` | Account management page. |
| `static/service.html` | Service detail page. |
| `static/js/graph.js` | Dependency graph visualization (JavaScript). |
| `static/js/common.js` | Shared JS utilities. |

### Configuration & Deployment

| File | Purpose |
|---|---|
| `Cargo.toml` | Rust project manifest and dependencies. |
| `Dockerfile` | Development Docker image. |
| `Dockerfile.release.template` | Release Docker image template. |
| `docker-compose.release.template.yaml` | Docker Compose template for deployment. |
| `api.yaml` | The service's own OpenAPI specification. |
| `demo.sh` | Demo script showcasing API usage. |
| `.github/workflows/quality.yml` | CI quality gate: Clippy, fmt, test, audit, tarpaulin, ESLint, Prettier. |
| `package.json` | Node.js dev-dependencies for JS linting (ESLint, Prettier). |
| `eslint.config.js` | ESLint 9 flat config for `static/js/*.js`. |
| `.prettierrc` | Prettier formatting rules. |

### Documentation

| File | Purpose |
|---|---|
| `README.md` | Project overview, installation, configuration, and API reference. |
| `CHANGELOG.md` | Change log following Keep a Changelog format. |
| `LICENSE` | Project license. |
| `docs/README.md` | Documentation index. |
| `docs/getting-started.md` | Quick start guide. |
| `docs/user-guide.md` | End-user guide. |
| `docs/developer-guide.md` | Developer/contributor guide. |
| `docs/administration.md` | Admin configuration guide. |
| `docs/ci-integration.md` | CI/CD pipeline integration guide. |
| `docs/sanshain-yaml.md` | Sanshain YAML config file format. |
| `docs/reports/demo.md` | Demo report documentation. |

### AI / Planning

| File | Purpose |
|---|---|
| `ai/plan.md` | Development task plan with status tracking. |
| `ai/ai-rules.md` | AI assistant rules and guidelines. |
| `ai/project.md` | This file — project summary for LLM context. |
| `ai/quality-hardening.md` | Quality hardening summary: tools, metrics, configuration. |
| `ai/skill-quality.md` | Reusable Junie skill for running quality checks. |

## Tech Stack

- **Language**: Rust (2024 edition)
- **Web Framework**: Axum
- **Database**: SQLite (default) or PostgreSQL via `sqlx`
- **Templating**: Askama / HTML templates with HTMX
- **Auth**: Local passwords (bcrypt), LDAP, dev-mode bypass
- **Frontend**: Vanilla JS + HTMX for dynamic admin fragments
