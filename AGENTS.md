# Sanshain Service — Agent Instructions

Sanshain ("Sunshine" in Japanese) is a Rust service that manages, splits, and distributes API
specifications (OpenAPI, AsyncAPI, gRPC/Proto). **Producers** "provide" their full spec; **Consumers**
"require" only the snippets they need (see `CONTEXT.md`).

## Know the service before you speak

`docs/agents/service-facts.md` is the map of how the service behaves, each fact with the file that
is its authority (the import below loads it with this file; an agent that does not follow imports
reads it now). Before proposing
a change, answering a question about behaviour, or asking the user anything, read the matching
topic **and open its source**. A question the map or its sources answer is not a question for the
user; a claim that is in neither is a guess and must be called one.

@docs/agents/service-facts.md

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
- **Lint**: both clippy runs of the gate must be clean (Quality gates, below). `#[allow(...)]` is forbidden — fix the lint instead.
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
- `./scripts/itest.sh` — bash integration test suite (requires the service running).
- `npx playwright test tests/ui/smoke.test.js` — UI smoke tests (requires the service running).
  UI tests import `test` from `tests/ui/fixtures.js`, not from `@playwright/test`, so their
  coverage is recorded.
- `just check` / `npm run check` — a quick pass (format check, clippy on the shipped code without
  features, tests, site tests). What decides is the gate, below.

## Quality gates

The standard the service is held to is [paxel/gates](https://github.com/paxel/gates);
`docs/adr/0012-move-to-gates.md` says where the service adds to it and why. The workflows
call gates' shared workflows, pinned by commit; Dependabot moves the pin.

### Before a commit

Rust: `cargo fmt`, clippy and the tests, on the toolchain `rust-toolchain.toml` pins. Clippy is
two runs, as in the shared Rust gate: the shipped code under the whole lint set, and the
tests with the lints allowed that are how a test fails:

```sh
cargo clippy --locked --all-features -- -D warnings
cargo clippy --locked --tests --benches --examples --all-features -- -D warnings \
  -A clippy::unwrap_used -A clippy::expect_used -A clippy::panic -A clippy::indexing_slicing \
  -A clippy::string_slice -A clippy::arithmetic_side_effects -A clippy::unreachable
scripts/ci-test.sh cargo test --locked --all-features
```

`--all-features` turns on `test-support`, so the in-memory repository
(`src/application/mock_repo.rs`) is linted as shipped code. The PostgreSQL tests start their
database through testcontainers and need Docker; `scripts/ci-test.sh` pulls the image once
and then runs what it is given, as CI does.

JavaScript (the page scripts in `static/js/`, the site build in `scripts/`, the tests in
`tests/ui/` and `tests/site/`): plain JavaScript, type-checked by `tsc` (`checkJs`). These are
the commands of the shared TypeScript gate, on the Node line `.nvmrc` pins:

```sh
npm ci --ignore-scripts
npx --no-install prettier --check .
npx --no-install eslint --max-warnings 0 .
npx --no-install tsc --noEmit
npm run test:coverage
```

A change to `static/css/app.input.css` or to the classes a page uses is followed by
`npm run build:css`; CI fails when the committed `static/css/app.css` is not the built one.

### Coverage ratchet

Line coverage may only go up. `coverage-floors.toml` holds three floors: `rust` (the Rust
gate in `ci.yml`), `site` (the site build in `scripts/`, the TypeScript gate) and `ui` (the page
scripts in `static/js/`, the `ui` job). Every feature ships with tests that hold or raise
coverage; a feature that lowers it is not done. When coverage rises, raise the floor and the
badge in the same pull request, with gates checked out beside the repository:

```sh
scripts/ci-test.sh cargo tarpaulin --engine llvm --locked --all-features --workspace \
  --skip-clean --out Lcov --output-dir coverage && mv coverage/lcov.info coverage/rust.lcov
python3 ../gates/scripts/coverage_ratchet.py --floors coverage-floors.toml --part rust

npm run test:coverage && cp coverage/lcov.info coverage/site.lcov
python3 ../gates/scripts/coverage_ratchet.py --floors coverage-floors.toml --part site
```

Every Rust test target takes part, the PostgreSQL tests included; a new file in `tests/`
needs no entry anywhere. The `ui` part is measured by the UI tests against a started service
(`cargo run`, then `UI_COVERAGE=1 npx playwright test` with the files the `ui` job of `ci.yml`
lists; `tests/ui/coverage.js` writes `coverage/ui.lcov`), then the ratchet with `--part ui`.
Never lower a floor.

### Fuzzing

`fuzz/` holds the targets `openapi`, `asyncapi`, `proto`, `backstage` and `helm`; `fuzz.yml`
runs them weekly, unoptimised (`--dev`), and replays `fuzz/regressions/<target>` first. See
`fuzz/README.md`. `fuzz/Cargo.lock` is committed and has its own `fuzz/osv-scanner.toml`,
because osv-scanner reads the exceptions beside the lockfile.

### Before a release

Run the `verify-release` skill. For this repository that is: fmt, both clippy runs, the full
test suite under `scripts/ci-test.sh`, the commands of the TypeScript gate, osv-scanner with
gates' `scripts/osv_gate.py`, the coverage ratchet for the three parts, `scripts/itest.sh`
and the UI tests against a started service, and the stylesheet check. The version stands in
`Cargo.toml` and `Cargo.lock`. The release workflow checks the tag against `Cargo.toml` and
runs the shared release gate before it builds; the image holds the attested binaries.

### Security rules

The decisions behind these are in `docs/adr/0012-move-to-gates.md`.

- Shipped code may not panic: `Cargo.toml` denies unwrap, expect, panic, todo,
  unimplemented, unreachable, indexing, string slicing, overflowing arithmetic and `dbg!`
  under `[lints]`. Tests are exempt through the second clippy run.
- `unsafe` is forbidden in the crate (`#![forbid(unsafe_code)]` at both crate roots). A test
  file that must call `std::env::set_var` says so with a file-level
  `#![expect(unsafe_code, reason = "...")]`. `#[allow(...)]` is denied: a lint that must be
  silenced gets `#[expect(<lint>, reason = "...")]`.
- Every action in `.github/workflows/` is pinned to a full commit SHA with its version in
  a trailing comment. Never reference an action by tag or branch alone.
- Rust is pinned in `rust-toolchain.toml`; the shared workflows and the service's own CI jobs
  read it from there. The release workflow's build job mirrors it
  (`dtolnay/rust-toolchain@<sha> # <version> branch`), and so does the builder image of the
  `Dockerfile`, by tag and digest. A toolchain bump is a commit of its own that changes the
  file and those references together. Node is pinned in `.nvmrc`.
- Every workflow starts with `permissions: contents: read`; a job that writes widens it
  itself.
- A dependency advisory is fixed by an update. When no fixed release exists, it is accepted
  in `osv-scanner.toml` (for the fuzz crate in `fuzz/osv-scanner.toml`) with the reason the
  service is not exposed and an `ignoreUntil` date. A licence outside the allowlist is
  accepted per package with `effectiveUntil`. Never ignore without reason and date.
- A code-scanning alert or a SonarQube Cloud finding that stays open is accepted in
  `code-scanning-ignores.toml` with its tool, rule, file, reason and an `ignoreUntil` date.
  Nothing is dismissed in GitHub or marked in SonarQube Cloud, and
  `sonar-project.properties` silences no finding.
- The release fails on a known vulnerability, on every open code-scanning alert, secret
  alert and SonarQube Cloud finding that is not accepted, and on a missing `CHANGELOG.md`
  section for the version. The weekly Security run fails on the same, and on unmaintained
  dependencies and passed dates; its image scan fails on a fixable high or critical CVE in
  the latest released image.
- `summer` takes pull requests only.

## Documentation Upkeep
Update these as part of the same change, not as a follow-up:
- **`docs/agents/service-facts.md`**: every fact the change alters, and every path it moves or
  deletes. `tests/site/agent-facts.test.js` fails on a pointer to a file that no longer exists;
  keep the map under ~400 lines (it is loaded into every session).
- **`README.md`**: new features, endpoints, or configuration options.
- **`CHANGELOG.md`**: every user-facing change, [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format, under the current unreleased/version heading.
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

## External Integrations
- Playwright MCP server available for browser-based UI testing (`npx @playwright/mcp`).

## Agent skills

### Issue tracker

Issues live in GitHub Issues (`paxel/sanshain-service`); skills use the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout (`CONTEXT.md` + `docs/adr/` at the repo root, created lazily). See `docs/agents/domain.md`.

### Ticket IDs

Prefix `san`: an implementation ticket is `[san-1007]`, in issue titles, commit messages, the
changelog and the docs. See `docs/agents/ticket-ids.md`.
