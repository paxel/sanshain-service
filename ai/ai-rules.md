# AI Rules — Rust & Service Development Best Practices

## Rust Best Practices

### Error Handling
- STRICTLY FORBIDDEN: `unwrap()`. All fallible operations must be handled with `?`, `match`, or `if let`.
- Avoid `expect()` in production logic. Use `.expect("clear message")` ONLY in:
    - Startup code (`main.rs`) where failure means the service cannot run.
    - Tests and `mod tests`.
    - Static Regex compilation.
- Use `thiserror` for all domain and application errors. Avoid manual `fmt::Display` implementations for errors.
- Propagate errors with `?`; convert at layer boundaries (e.g., repository errors → application errors).
- Implement `IntoResponse` for `AppError` in the presentation layer to centralize status code mapping and logging.

### Library Usage
- Library-first: Before writing custom logic, check if a library call can achieve the same result (e.g., `argon2` for passwords, `chrono` for time, `uuid` for IDs).
- Avoid duplication: If similar logic exists in multiple places, refactor it into a shared function or service.
- Prefer small, boundary-safe helpers over manual byte indexing or ad-hoc parsers; use iterators, `strip_prefix`, `split_once`, crate parsers, or validated regex captures before writing custom loops.
- Keep dependencies updated: Regularly check for major version updates in `Cargo.toml`.

### Ownership & Lifetimes
- Prefer borrowing (`&T`, `&mut T`) over cloning. Clone only when ownership transfer is genuinely needed.
- Use `Arc<T>` for shared ownership across async tasks; avoid `Rc` in async code.
- Keep lifetime annotations minimal — let the compiler infer where possible.

### Anti-Patterns to Avoid
- **Primitive Obsession**: Use proper enums and structs for domain concepts instead of raw strings or integers (e.g., `ApiType` instead of `String`).
- **Framework Leakage**: Keep the Domain and Application layers free of Axum, SQLx, or other framework dependencies. Use ports (traits) for abstraction.
- **Manual Handlers**: Avoid large, monolithic handlers. Delegate business logic to application services.
- **Mocking Overlap**: Use a consistent Mock Repository for testing that implements all port traits, rather than creating ad-hoc mocks in every test file.
- **Mixed Routing**: Be consistent with nesting and prefixing. Explicit routes are often better than deep nesting for clarity.
- **Dead Code and Dead Files**: Remove unused code, obsolete helpers, generated scratch files, and stale assets when they are clearly no longer referenced. Do not keep "maybe useful later" code in the repository.
- **Copy-Paste Setup**: Shared test/application setup must live in one helper or fixture; duplicated setup logic is a maintenance bug.

### Code Style
- Follow `rustfmt` defaults. Run `cargo fmt` before committing.
- Run `cargo clippy` and address all warnings. STRICTLY FORBIDDEN: disabling Clippy warnings or compiler lints using `#[allow(...)]` annotations. Always refactor the code to satisfy the linter.
- Prefer `impl Trait` in function signatures over explicit generics when the type is used once.
- Use `#[must_use]` on functions whose return values should not be silently ignored.

### Dependencies
- Pin major versions in `Cargo.toml`; use `cargo update` deliberately.
- Prefer well-maintained crates with minimal transitive dependencies.
- Audit new dependencies with `cargo audit`.

### Testing
- Every public function in domain and application layers must have unit tests.
- Use `#[cfg(test)]` modules co-located with the code they test.
- Integration tests go in `tests/` and exercise the full HTTP stack.
- Use `mockall` or hand-written mocks for port traits in unit tests.
- Test both success and error paths; include edge cases (empty input, boundary values).
- Target healthy unit-test coverage of 80–90% for domain and application logic. If coverage is below that range, add meaningful tests for core behavior before adding broad integration-only coverage.
- Coverage must be measured with `cargo tarpaulin --out Xml --skip-clean` (or the current CI coverage command) for quality reviews; do not claim coverage improvements without running the tool.

#### Test Quality Rules
- Tests must be reasonable and useful and assert actual values — avoid tautologies and vacuous checks.
  - Prefer concrete assertions over permissive ones: use `assert_eq!`, `assert_ne!`, and exact field/value checks instead of only `is_ok()`/`is_err()`.
  - Avoid assertions that only check non-emptiness, length, or presence of any substring when a precise structure or value is known.
  - Do not write tests that merely reassert implementation details without validating observable behavior at the public API boundary.
  - Keep randomness controlled (fixed seeds or deterministic inputs) so assertions target exact, stable results.
  - No coverage gaming: do not add no-op tests solely to bump coverage; each test must validate meaningful behavior or error handling.

## Service Development Best Practices

### Architecture (DDD Hexagonal/Onion)
- **Domain layer** is the innermost ring: models, value objects, port traits. Zero framework dependencies.
- **Application layer** contains use-case functions that orchestrate domain logic. Depends only on domain.
- **Infrastructure layer** implements ports (database adapters, external APIs). Depends on domain; never imported by application directly (only via trait objects).
- **Presentation layer** (`main.rs`) wires everything together and defines thin HTTP handlers.
- New features: define the port trait first, implement the use-case, then add the adapter and handler.

### API Design
- Return appropriate HTTP status codes: `200` (success), `201`/`202` (created/accepted), `400` (bad request), `404` (not found), `409` (conflict), `500` (internal error).
- Validate all input at the handler level before passing to application services.
- Use consistent JSON error responses with a human-readable message.
- Document every endpoint in `api.yaml` and keep it in sync with the implementation.

### Database
- All schema changes go through migration files — never modify the database manually.
- SQL migrations are located in the `src/infrastructure/migrations/<db>/` directory.
- Use parameterized queries exclusively; never interpolate user input into SQL.
- Keep transactions short; avoid holding locks across async boundaries.

### Security
- Never log secrets, tokens, or passwords.
- Store passwords with Argon2; store API tokens as SHA-256 hashes.
- Validate and sanitize all user input; use Askama's auto-escaping for HTML output.
- Apply security headers (CSP, X-Content-Type-Options, X-Frame-Options, Referrer-Policy).
- CSRF tokens on all state-changing endpoints.

### Observability
- Log at appropriate levels: `error` for internal failures, `warn` for client errors, `info` for lifecycle events, `debug`/`trace` for development.
- Include request context (method, path, status) in log entries.
- Use structured logging fields, not string interpolation.

### AI Workflow Rules
- For every task that involves code changes, you MUST run `cargo clippy -- -D warnings` and ensure it passes before submitting. You MUST NOT use `#[allow(...)]` to hide warnings or bypass this check.
- Always verify that all existing and new tests pass using `cargo test`.
- Run `cargo audit` and `cargo geiger` for security-sensitive or quality-hardening tasks, and document any accepted finding with a clear reason.
- For cleanup/refactor tasks, explicitly check for dead code, unused files, duplicate setup, and hand-rolled logic that should be a library or standard-library call.
- Use the `update_status` tool to keep the user informed about progress.
- The current project version is defined by `Cargo.toml` for all documentation purposes.

#### Git guardrails
- STRICTLY FORBIDDEN: Commit, edit, or delete anything under the `.git/` directory. Never touch `.git/*`, including `.git/config`, `.git/hooks/*`, or any internal Git metadata, unless the user explicitly instructs to do so in this session.
- Do not rewrite history (no `git reset --hard`, `rebase --onto`, force-pushes, or similar) unless explicitly requested by the user.
- Do not create or modify Git hooks automatically.

### Documentation
- Keep `README.md` current with every new feature or endpoint.
- Update `CHANGELOG.md` (Keep a Changelog format) with every user-facing change.
- Update `ai/plan.md` when tasks are completed or new tasks are identified.
- Code comments: explain *why*, not *what*. Match the existing comment density.

### CI/CD
- All PRs must pass `cargo build`, `cargo test`, `cargo clippy`, and `cargo fmt --check`.
- Docker images are built from release templates with pre-compiled binaries.
- Database migrations run automatically on startup — no manual migration steps in deployment.

### Quality Tooling
The `quality.yml` workflow is the merge gate for all PRs and pushes:

| Tool            | Purpose                  | Command                                  |
|-----------------|--------------------------|------------------------------------------|
| Clippy          | Rust linting             | `cargo clippy -- -D warnings`            |
| rustfmt         | Formatting               | `cargo fmt --check`                      |
| cargo-audit     | Security audit           | `cargo audit`                            |
| cargo-tarpaulin | Coverage (informational) | `cargo tarpaulin --out Xml --skip-clean` |
| ESLint          | JS linting               | `npx eslint static/js/`                  |
| Prettier        | JS formatting            | `npx prettier --check static/js/`        |
