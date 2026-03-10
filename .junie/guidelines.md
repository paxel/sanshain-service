# SanShain Service Development Guidelines

## Architecture
This project follows **DDD Hexagonal/Onion Architecture**. All changes must maintain this structure:
- **Domain** (`src/domain/`): Models (`models.rs`) and port traits (`ports.rs`). No framework dependencies.
- **Application** (`src/application/`): Use-case services (`services.rs`). Depends only on domain.
- **Infrastructure** (`src/infrastructure/`): Adapter implementations (e.g., `sqlite_repository.rs`). Implements domain ports.
- **Presentation** (`src/main.rs`): Thin Axum handlers that delegate to application services.

When adding new features, place business logic in the application layer, define abstractions in domain ports, and implement adapters in infrastructure. Keep handlers thin.

## Project Structure
- `src/main.rs`: Axum server setup and thin API handlers.
- `src/domain/`: Domain models and repository port traits.
- `src/application/`: Application service layer with use-case functions.
- `src/infrastructure/`: Database adapters implementing domain ports.
- `src/openapi.rs`: Logic for parsing and splitting OpenAPI specifications.
- `migrations/`: SQL migration files for `sqlx`.

## Build & Run
The service is built with Rust (2024 edition).
```bash
cargo build
cargo run
```


## Database
The service uses SQLite by default. To change the database, set the `DATABASE_URL` environment variable.
Example for PostgreSQL (requires changing the `sqlx` feature in `Cargo.toml`):
```bash
DATABASE_URL=postgres://user:password@localhost/dbname cargo run
```

## OpenAPI Splitting
The splitting logic is currently basic and includes all schemas/components in each snippet. For improved performance, this could be refactored to include only the schemas used by the specific operation.

## Testing
All units must be tested. When adding or modifying functionality:
- Add or update unit tests for domain and application logic.
- Add or update integration tests in `tests/` for API-level behavior.
- Run `cargo test` and ensure all tests pass before considering work complete.

## Documentation
- **README.md**: Keep up to date when adding new features, endpoints, or configuration options.
- **CHANGELOG.md**: Update with every user-facing change following [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format. All entries go under `[0.1.0]` until first release.
- **plan.md** (`ai/plan.md`): Update task status as items are completed.
