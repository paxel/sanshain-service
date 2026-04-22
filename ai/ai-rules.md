# AI Rules — Rust & Service Development Best Practices

## Rust Best Practices

### Error Handling
- STRICTLY FORBIDDEN: `unwrap()`. All fallible operations must be handled with `?`, `match`, or `if let`.
- Avoid `expect()` in production logic. Use `.expect("clear message")` ONLY in:
    - Startup code (`main.rs`) where failure means the service cannot run (though even here, `match` with `std::process::exit(1)` is preferred).
    - Tests and `mod tests`.
    - Static Regex compilation.
- Define domain-specific error enums; implement `std::fmt::Display` and `std::error::Error`.
- Use `thiserror` for library-style errors, `anyhow` only in binaries or top-level orchestration.
- Propagate errors with `?`; convert at layer boundaries (e.g., infra errors → domain errors).
- Never ignore errors with `let _ = ...` unless there is a strong justification and a comment explaining why.

### Ownership & Lifetimes
- Prefer borrowing (`&T`, `&mut T`) over cloning. Clone only when ownership transfer is genuinely needed.
- Use `Arc<T>` for shared ownership across async tasks; avoid `Rc` in async code.
- Keep lifetime annotations minimal — let the compiler infer where possible.

### Async & Concurrency
- Use `tokio` as the async runtime (already the project standard).
- Avoid blocking calls inside `async fn`; use `tokio::task::spawn_blocking` when necessary.
- Prefer `tokio::sync::Mutex` over `std::sync::Mutex` when the lock is held across `.await` points.

### Code Style
- Follow `rustfmt` defaults. Run `cargo fmt` before committing.
- Run `cargo clippy` and address all warnings.
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
- For every task that involves code changes, you MUST run `cargo clippy -- -D warnings` and ensure it passes before submitting.
- Always verify that all existing and new tests pass using `cargo test`.
- Use the `update_status` tool to keep the user informed about progress.
- The current project version is defined by `Cargo.toml` for all documentation purposes.

### Documentation
- Keep `README.md` current with every new feature or endpoint.
- Update `CHANGELOG.md` (Keep a Changelog format) with every user-facing change.
- Update `ai/plan.md` when tasks are completed or new tasks are identified.
- Code comments: explain *why*, not *what*. Match the existing comment density.

### CI/CD
- All PRs must pass `cargo build`, `cargo test`, `cargo clippy`, and `cargo fmt --check`.
- Docker images are built from release templates with pre-compiled binaries.
- Database migrations run automatically on startup — no manual migration steps in deployment.
