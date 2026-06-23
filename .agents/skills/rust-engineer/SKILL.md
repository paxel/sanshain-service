---
name: rust-engineer
description: Enforce Rust project policy, formatting, and quality standards during development.
---

# Rust Engineer — Policy & Pitfalls

This skill encodes the project policy for Rust development in Sanshain. It ensures code quality, consistency, and adherence to DDD architecture and security standards.

## Purpose
To provide proactive guidance during the coding phase to avoid formatting, linting, and architectural issues.

## Trigger
Use this skill whenever:
- Creating or modifying Rust code (`.rs` files).
- Refactoring the backend logic.
- Adding new dependencies or database operations.

## Setup Check

1. **Rust Edition** — Verify `edition = "2024"` in `Cargo.toml`.
2. **Architecture** — Maintain Hexagonal/DDD structure:
   - `src/domain/`: Logic-free models and port traits.
   - `src/application/`: Use-case services (business logic).
   - `src/infrastructure/`: Port implementations (DB, external APIs).
   - `src/main.rs`: Thin Axum handlers.

## MUST DO

- **Format Code** — Always run `cargo fmt` after modifying `.rs` files.
- **Lint Code** — Always run `cargo clippy -- -D warnings` and fix all issues.
- **Simplify Types** — Use `type` aliases for complex types (e.g., `Vec<(i64, String, ...)>`) to avoid clippy `type_complexity` warnings.
- **Error Handling** — Use `Result<T, AppError>` for business logic. Map infrastructure errors (e.g., `sqlx::Error`) to `RepositoryError` and then to `AppError` at the boundary.
- **Tracing** — Decorate non-trivial functions with `#[tracing::instrument(skip_all)]` or appropriate fields.
- **Documentation** — Keep `CHANGELOG.md` updated with every user-facing change.

## MUST NOT DO

- **No `unwrap()` / `expect()`** — Use `?` operator or handle `None`/`Err` cases explicitly.
- **No `unsafe`** — Minimize use of `unsafe`. If required, it must be documented and justified.
- **No Complex Inline Types** — Avoid inline types with more than 3-4 nested components.
- **No Leaking Infrastructure** — Never leak `sqlx` or other infrastructure types into the Domain or Application layers.

## Procedures

### 1. Pre-Implementation Review
- Check `src/domain/models.rs` and `src/domain/ports.rs` for existing abstractions.
- Identify the correct layer for the new logic.

### 2. Implementation
- Apply the code changes following the MUST DO/MUST NOT DO rules.
- Use `#[allow(clippy::type_complexity)]` only as a last resort if refactoring to a `type` alias or struct is not feasible.

### 3. Post-Implementation Check
- Run `cargo fmt`.
- Run `cargo clippy -- -D warnings`.
- Run `cargo test`.

## Quality Checklist
- [ ] Code is formatted with `cargo fmt`.
- [ ] No clippy warnings (`cargo clippy -- -D warnings`).
- [ ] Domain logic is separated from infrastructure.
- [ ] All new functions are instrumented with `tracing`.
- [ ] Errors are properly mapped to `AppError`.
