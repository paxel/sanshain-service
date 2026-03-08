# Implementation Plan: SanShain Service

SanShain is a service designed to manage and distribute OpenAPI specifications for microservices, tracking client-service dependencies and generating usage reports.

## 1. Project Initialization & Infrastructure
- [ ] Initialize Rust project with `axum` or `actix-web` for the REST API.
- [ ] Set up a database (e.g., PostgreSQL or MongoDB) to store:
    - Service definitions.
    - Branch-specific OpenAPI specs.
    - Per-endpoint splits (YAML + DTOs).
    - Client registration and endpoint usage tracking.
- [ ] Implement OpenAPI parsing and splitting logic (likely using `openapiv3` crate).

## 2. Core API Implementation

### 2.1. `POST /provide`
- **Arguments**: `servicename`, `branch`, `openapi.yaml` (file or string).
- **Logic**:
    1. Parse the full `openapi.yaml`.
    2. Split the specification into individual endpoints.
    3. Extract necessary DTOs (components/schemas) for each endpoint.
    4. Store the mapping (Service -> Branch -> Endpoint -> YAML) in the DB.

### 2.2. `GET /require`
- **Arguments**: `clientname`, `servicename`, `branch`, `endpoint_url`.
- **Logic**:
    1. Retrieve the specific YAML snippet for the requested `endpoint_url` from the DB.
    2. Record the client's dependency: "Client X uses Endpoint Y on Service Z (Branch B)".
    3. Return the YAML to the client.

### 2.3. `GET /report`
- **Arguments**: `branch`.
- **Logic**:
    1. Query the DB for all client-service-endpoint relationships on the given branch.
    2. Generate a JSON report representing the dependency graph.
    3. Include validation:
        - Identify endpoints provided but never required (unused).
        - Identify endpoints required by clients but not provided in the current branch (missing).

## 3. Advanced Features (Optional/Phase 2)
- [ ] Add a "Release" flag to client requirements to distinguish between development/feature branch usage and production-ready dependencies.
- [ ] Visualization tool for the dependency graph.

## 4. Documentation & Guidelines
- [ ] Create `README.md` in the service root explaining use cases, API usage, and example reports.
- [ ] Finalize `.junie/guidelines.md` with build, test, and development instructions specific to SanShain.

## 5. Verification
- [ ] Create integration tests for `provide`, `require`, and `report` flows.
- [ ] Verify that the OpenAPI splitting logic correctly handles shared schemas/DTOs.
