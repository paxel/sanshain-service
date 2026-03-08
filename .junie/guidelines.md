# SanShain Service Development Guidelines

## Project Structure
- `src/main.rs`: Axum server setup and API handlers.
- `src/openapi.rs`: Logic for parsing and splitting OpenAPI specifications.
- `migrations/`: SQL migration files for `sqlx`.

## Build & Run
The service is built with Rust (2024 edition).
```bash
cargo build
cargo run
```

## Testing
Unit and integration tests are located in `src/` and the `tests/` directory.
```bash
cargo test
```

## Database
The service uses SQLite by default. To change the database, set the `DATABASE_URL` environment variable.
Example for PostgreSQL (requires changing the `sqlx` feature in `Cargo.toml`):
```bash
DATABASE_URL=postgres://user:password@localhost/dbname cargo run
```

## OpenAPI Splitting
The splitting logic is currently basic and includes all schemas/components in each snippet. For improved performance, this could be refactored to include only the schemas used by the specific operation.
