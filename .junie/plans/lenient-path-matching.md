---
sessionId: session-260419-123520-1fpp
isActive: false
---

# Requirements

### Overview & Goals
Introduce lenient endpoint matching to handle common differences in OpenAPI path definitions and client requirements, such as varying path variable names and redundant slashes.

### Scope
- **In Scope**:
    - Path variable names (e.g., `{id}` matches `{userId}`).
    - Redundant slashes (e.g., `//` matches `/`).
    - Leading/trailing whitespace in paths.
    - Storing and indexing normalized paths for performance.
- **Out Scope**:
    - Case-insensitivity for paths (typically paths are case-sensitive in REST).
    - Handling differences in query parameters (they are not part of the OpenAPI path key).
    - Semantic path matching beyond template normalization.

# Technical Design

### Current Implementation
Matching is currently strict. The `endpoints` table uses the literal path string from the OpenAPI specification as its unique identifier. Any deviation in the required path (e.g., a different parameter name) results in a "404 Not Found" or a "Missing Endpoint" report.

### Proposed Changes
- **Normalization Algorithm**: 
    1. Collapse multiple consecutive slashes into a single slash.
    2. Replace all path variable placeholders (e.g., `{uuid}`, `{id}`) with a universal placeholder `{}`.
    3. Trim whitespace.
- **Data Model**:
    - Add `normalized_path` to `endpoints` table.
    - Index `normalized_path` for fast lookup.
- **Logic**:
    - During `provide`, compute the normalized path for each endpoint and store it.
    - During `require`, normalize the requested path and search against the `normalized_path` column.
    - Update the dependency report logic to use the same normalization if `endpoint_id` is missing.

### File Structure
- `src/openapi.rs`: Contains the `normalize_path` utility.
- `src/domain/models.rs`: `EndpointRecord` and `SpecChange` updated.
- `src/infrastructure/sqlite_repository.rs` & `postgres_repository.rs`: Updated to handle the new column and matching logic.
- `src/infrastructure/migrations/`: New SQL migration files.

### Risks & Mitigations
- **Ambiguity**: If a spec contains two paths that normalize to the same string (e.g., `/users/{id}` and `/users/{name}`), matching becomes ambiguous. 
    - *Mitigation*: These are usually invalid OpenAPI specs or bad REST design. We will pick the first match found in the spec.
- **Performance**: Normalizing paths on the fly during lookup.
    - *Mitigation*: The normalized path is indexed in the database to ensure `O(log N)` lookups.

# Testing

### Validation Approach
Verify leniency by creating a scenario where a service provides an endpoint with one variable name and a client requires it with another.

### Key Scenarios
- **Variable Name Matching**: Provide `/api/{id}`, Require `/api/{userId}` -> Success.
- **Slash Collapsing**: Provide `/api/v1/users`, Require `/api/v1//users` -> Success.
- **Mixed Leniency**: Provide `/api/v1/users/{id}`, Require `/api/v1//users/{uId}/` -> Success.
- **Backward Compatibility**: Ensure exact matches still work correctly.

# Delivery Steps

### ✓ Step 1: Implement path normalization and update models
Implement path normalization logic in `src/openapi.rs` and update domain models to support normalized paths.

- Add `normalize_path(path: &str) -> String` to `src/openapi.rs`.
- Update `EndpointRecord` and `SpecChange` in `src/domain/models.rs` to include a `normalized_path` field.

### ✓ Step 2: Database migrations for normalized paths
Create database migrations to add the `normalized_path` column to the `endpoints` table for both SQLite and Postgres.

- Add `src/infrastructure/migrations/sqlite/20240315000000_endpoint_normalized_path.sql`.
- Add `src/infrastructure/migrations/postgres/20240315000000_endpoint_normalized_path.sql`.
- Migration should add the column and an index on it.

### ✓ Step 3: Update repository implementations and backfill
Update the repository implementations to use normalized paths for endpoint lookups and storage.

- Update `SqliteSpecRepository` in `src/infrastructure/sqlite_repository.rs`.
- Update `PostgresSpecRepository` in `src/infrastructure/postgres_repository.rs`.
- Update `MockRepo` in `src/application/services.rs` for test consistency.
- Implement a one-time backfill of `normalized_path` for existing records in `run_migrations`.

### ✓ Step 4: Refactor application services for leniency
Refactor application services to leverage lenient matching during spec provision and endpoint requirements.

- Update `provide_spec_inner` in `src/application/services.rs` to use normalized paths when checking for existing endpoints.
- Update `require_endpoint_inner` to ensure paths are normalized before querying the repository.