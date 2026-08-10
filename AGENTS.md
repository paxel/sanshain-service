# Sanshain Service — Agent Instructions

Sanshain ("Sunshine" in Japanese) is a Rust service that manages, splits, and distributes API
specifications (OpenAPI, AsyncAPI, gRPC/Proto). **Producers** "provide" their full spec; **Consumers**
"require" only the snippets they need (see `CONTEXT.md`).

## Core Technologies
- **Backend**: Rust (2024 edition), [Axum](https://github.com/tokio-rs/axum).
- **Runtime**: [Tokio](https://tokio.rs/) (async).
- **Database**: [SQLx](https://github.com/launchbadge/sqlx) — SQLite (default) or PostgreSQL.
- **Templates**: [Askama](https://github.com/djc/askama).
- **Caching**: [Moka](https://github.com/moka-rs/moka).
- **Auth**: Argon2, session tokens, optional LDAP/AD.
- **Frontend**: Vanilla JS + [Tailwind CSS](https://tailwindcss.com/) (built to `static/css/app.css` via `npm run build:css`; committed, since prod serves `static/` directly). HTMX for admin fragments.
- **Task runner**: [just](https://github.com/casey/just) (`justfile`) — `npm run` scripts wrap the same commands.

## Architecture (DDD Hexagonal/Onion)
Strict layering — maintain these boundaries when adding or changing code:
1. **Domain** (`src/domain/`): models (`models.rs`), port traits (`ports.rs`). **No framework deps** (no Axum, no SQLx).
   Enforced by `tests/architecture_boundaries_test.rs`, which fails the build when Domain or
   Application references a framework or an adapter — add a port instead of an import.
2. **Application** (`src/application/`): use-case/business logic (`services.rs`). Depends only on Domain.
3. **Infrastructure** (`src/infrastructure/`): port implementations — SQLx repositories, LDAP, telemetry, cache. Depends on Domain.
4. **Presentation** (`src/presentation/`, wired in `src/main.rs`/`src/lib.rs`): thin Axum handlers and middleware. Delegate business logic to Application; do not put logic here.

Other top-level modules: `src/openapi.rs`, `src/asyncapi.rs`, `src/proto.rs` (spec parsing/splitting), `src/infrastructure/migrations/` (SQL migrations for `sqlx`).

## Rust Standards
- **`unwrap()` is forbidden.** Handle all fallible operations with `?`, `match`, or `if let`.
- **Errors**: use `thiserror`; propagate with `?`, convert at boundaries.
- **Lint**: `cargo clippy -- -D warnings` must be clean. `#[allow(...)]` is forbidden — fix the lint instead.
- **Style**: `rustfmt` defaults; run `cargo fmt` before considering work done.
- **Ownership**: prefer borrowing over cloning; use `Arc<T>` for shared async state.
- **Testing**: 80-90% coverage target for Domain/Application; every public function needs a unit test. Add/update integration tests in `tests/` for API-level behavior.
  - Assert actual values, not just `is_ok()`.
  - Avoid randomness in tests — use fixed seeds/values.

## Security Policies (non-negotiable)
- No hardcoded auth-bypass literals (e.g. `"test-token"`) in production code paths, even for convenience. Test-only helpers must be behind `#[cfg(test)]`.
- All state-changing endpoints must go through CSRF validation (`validate_csrf` middleware); tests use real seeded tokens, not bypasses.
- Never log secrets, tokens, or passwords.
- Every `/admin/*` route must declare a `RouteGuard` (via the `require(...)` layer) — this is enforced by a compile-time-checked test in `src/lib.rs` (`all_admin_routes_declare_a_route_guard`); keep it passing when adding routes.
- All schema changes go through SQL migrations in `src/infrastructure/migrations/` — never hand-edit the DB shape elsewhere.

## Build, Test, Verify
```bash
cargo build
cargo run                    # SQLite by default
DATABASE_URL=postgres://user:password@localhost/dbname cargo run   # Postgres (needs sqlx feature already enabled in Cargo.toml)
```
- `just check` / `npm run check` — fmt-check + lint + test (fastest local gate; run before considering work complete).
- `cargo test` — unit + integration tests.
- `./scripts/itest.sh` — bash integration test suite (requires the service running).
- `npx playwright test tests/ui/smoke.test.js` — UI smoke tests (requires the service running).
- `cargo tarpaulin --out Xml` — coverage. CI enforces a ratcheting floor (`--fail-under` in
  `.github/workflows/quality.yml`): whenever coverage rises, raise the floor to (new value − 2)
  in the same PR. Never lower the floor.
- `cargo audit` — security audit.
- The Rust toolchain is pinned in `rust-toolchain.toml` and mirrored in the CI workflows
  (`dtolnay/rust-toolchain@<version>`). Toolchain bumps are deliberate, separate PRs that update
  the pin and all three workflow references together — never bump as a side effect of another change.

## Documentation Upkeep
Update these as part of the same change, not as a follow-up:
- **`README.md`**: new features, endpoints, or configuration options.
- **`CHANGELOG.md`**: every user-facing change, [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format, under the current unreleased/version heading.
- **`docs/ai/plan.md`**: check off / update task status when completing backlog items tracked there.
- **`ai/improvements.md`**: a standing backlog of known risks/gaps (P0 security, P1 features, P2 quality). Pick one item at a time, don't "fix everything" in one PR.
- **`api.yaml`**: the contract Producers and Consumers speak (provide/require/report). Keep in sync
  with what the service exposes. It must stay stable — roughly 25 Producers pin against it.
- **`maintenance.yaml`**: the `/admin/*` administrative surface, including the permission each
  endpoint requires. Separate from `api.yaml` because it has a different audience and a different
  stability promise. Both are enforced by build-time checks in `src/lib.rs`
  (`router_matches_api_yaml_contract`, `router_matches_maintenance_yaml_contract`): every route must
  appear in one of them or in the explicit allowlist of routes belonging to neither.

## Working Agreements
- Do not "fix everything" in one pass — one focused change, tested, matching the current DDD layout.
- Don't guess: if a change's relation to the request is unclear, ask rather than assume.
- Keep handlers thin — if a handler is growing logic, move it to `src/application/services.rs`.

## Key Files
| File                                | Purpose                                                     |
|-------------------------------------|-------------------------------------------------------------|
| `Cargo.toml`                        | Rust package manifest                                       |
| `api.yaml`                          | OpenAPI contract for the Producer/Consumer API               |
| `maintenance.yaml`                  | OpenAPI contract for the `/admin/*` administrative surface   |
| `justfile` / `package.json`         | Task runner entry points (`just check`, `npm run check`, …) |
| `CHANGELOG.md` / `OLDER_CHANGES.md` | User-facing change history                                  |
| `docs/`                             | User and developer documentation (see `docs/README.md`)     |
| `ai/improvements.md`                | Prioritized improvement/risk backlog                        |
| `docs/ai/plan.md`                   | Longer-term feature roadmap                                 |

## External Integrations
- Playwright MCP server available for browser-based UI testing (`npx @playwright/mcp`).

## Agent skills

### Issue tracker

Issues live in GitHub Issues (`paxel/sanshain-service`); skills use the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout (`CONTEXT.md` + `docs/adr/` at the repo root, created lazily). See `docs/agents/domain.md`.

## Note for maintainers
This file is the shared, tool-agnostic instruction set (also usable by Junie, Codex, etc.).
`GEMINI.md` and `.junie/guidelines.md` predate it and overlap significantly — worth consolidating
so there's one source of truth instead of three drifting copies. `GEMINI.md`'s skills table points
at a project-local `.agents/skills/` that never existed — the real skills (changelog-updater,
secure-csrf, verify-release, version-management, skill-creator, markdown-table-formatter, plus
git-operations/rust-engineer/js-engineer) live at the user level in `~/.agents/skills/`, with
`~/.junie/skills` kept as a compat symlink to the same place, and each one symlinked individually
into `~/.claude/skills/` so Claude Code picks them up too. `GEMINI.md`'s table paths were corrected
to match.
