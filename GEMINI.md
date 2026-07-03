# Sanshain Service - GEMINI Project Context

## Project Overview
Sanshain (Japanese for "Sunshine") is a high-performance Rust service designed to manage, split, and distribute API specifications (OpenAPI, AsyncAPI, and gRPC/Proto). It acts as a central repository where microservices provide their full API definitions, and clients consume only the specific snippets (endpoints, channels, or methods) they need at build time.

### Core Technologies
- **Backend**: Rust (2024 edition), [Axum](https://github.com/tokio-rs/axum) web framework.
- **Runtime**: [Tokio](https://tokio.rs/) (async).
- **Database**: [SQLx](https://github.com/launchbadge/sqlx) supporting **SQLite** (default) and **PostgreSQL**.
- **Template Engine**: [Askama](https://github.com/djc/askama) (type-safe Jinja2-like templates).
- **Caching**: [Moka](https://github.com/moka-rs/moka) (high-performance in-memory cache).
- **Authentication**: Argon2, session-based tokens, optional LDAP/AD.
- **Frontend**: Vanilla JS/CSS, HTMX for admin fragments.
- **Task Runner**: [just](https://github.com/casey/just).

## Architecture (DDD Hexagonal/Onion)
The project strictly follows a layered architecture. Maintain these boundaries:
1.  **Domain Layer** (`src/domain/`): Models (`models.rs`), port traits (`ports.rs`). **NO framework dependencies** (Axum, SQLx).
2.  **Application Layer** (`src/application/`): Use-case orchestration and business logic. Depends only on Domain.
3.  **Infrastructure Layer** (`src/infrastructure/`): Port implementations (SQLx repository, LDAP provider). Depends on Domain.
4.  **Presentation Layer** (`src/presentation/` and `src/main.rs`): Axum handlers, middleware, and app wiring. Keep handlers thin; delegate to Application.

## Development Standards

### Rust Best Practices
- **STRICTLY FORBIDDEN**: `unwrap()`. All fallible operations must be handled with `?`, `match`, or `if let`.
- **Error Handling**: Use `thiserror` for all errors. Propagate with `?`; convert at boundaries.
- **Linter**: Addresses all Clippy warnings (`cargo clippy -- -D warnings`). `#[allow(...)]` is strictly forbidden.
- **Testing**: 80-90% coverage for Domain/Application. Every public function needs a unit test.
- **Immutability**: Prefer borrowing over cloning. Use `Arc<T>` for shared async state.
- **Style**: Follow `rustfmt` defaults. Run `cargo fmt` before committing.

### Security Policies
- **No Backdoors**: Never use hardcoded literals (e.g., `"test-token"`) for authentication bypass in production code.
- **CSRF**: All state-changing endpoints must use CSRF validation. Use real seeded tokens for tests.
- **Sensitive Data**: Never log secrets, tokens, or passwords.
- **Migrations**: All schema changes MUST use SQL migrations in `src/infrastructure/migrations/`.

## Specialized Procedures (AI Skills)
This project uses the **Agent Skills** standard, kept at the user level in `~/.agents/skills/`
(shared across tools; `~/.junie/skills` is a compat symlink to the same location). Before
performing the following tasks, read the corresponding `SKILL.md`:

| Task                      | Skill                      | Path                                                  |
|---------------------------|----------------------------|-------------------------------------------------------|
| **Update Changelog**      | `changelog-updater`        | `~/.agents/skills/changelog-updater/SKILL.md`         |
| **Handle CSRF/Auth**      | `secure-csrf`              | `~/.agents/skills/secure-csrf/SKILL.md`               |
| **Verify Release**        | `verify-release`           | `~/.agents/skills/verify-release/SKILL.md`            |
| **Version Bumping**       | `version-management`       | `~/.agents/skills/version-management/SKILL.md`        |
| **Create Skills**         | `skill-creator`            | `~/.agents/skills/skill-creator/SKILL.md`             |
| **Markdown Table Format** | `markdown-table-formatter` | `~/.agents/skills/markdown-table-formatter/SKILL.md`  |

## Building & Testing

### Key Commands
- **Run**: `cargo run`
- **Quality Check**: `just check` (fmt, lint, test)
- **Security Audit**: `cargo audit`
- **Integration Tests**: `./scripts/itest.sh` (requires running service)
- **UI Tests**: `npx playwright test`

### Test Quality
- Assert actual values, not just `is_ok()`.
- Avoid randomness in tests (use fixed seeds).
- Target coverage: `cargo tarpaulin --out Xml`.

## Key Files
- `Cargo.toml`: Main manifest.
- `api.yaml`: OpenAPI contract (keep in sync with code!).
- `CHANGELOG.md`: User-facing changes (Keep a Changelog format).
- `OLDER_CHANGES.md`: Historical changelog entries.
- `docs/`: User and developer documentation.

## External Integrations
- **MCP**: The project supports the Playwright MCP server for browser-based testing (`npx @playwright/mcp`).
