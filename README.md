# SanShain Service

SanShain is a service designed to manage and distribute OpenAPI specifications for microservices, tracking client-service dependencies and generating usage reports.

## Core Features
- **OpenAPI Splitting**: Automatically splits large OpenAPI files into per-endpoint snippets.
- **Dependency Tracking**: Records which clients depend on which service endpoints.
- **Dependency Reports**: Identifies unused endpoints and missing dependencies.

## API Usage

### 1. `POST /provide`
Provide an OpenAPI specification for a service branch.

**Payload:**
```json
{
  "servicename": "UserService",
  "branch": "main",
  "openapi_yaml": "..."
}
```

### 2. `GET /require`
Request the OpenAPI snippet for a specific endpoint and record the dependency.

**Query Parameters:**
- `clientname`: Name of the client service.
- `servicename`: Name of the target service.
- `branch`: Branch name.
- `path`: Endpoint path.
- `method`: HTTP method (GET, POST, etc.).

### 3. `GET /report`
Generate a dependency report for a specific branch.

**Query Parameters:**
- `branch`: Branch name to report on.

## Development

### Prerequisites
- Rust (2024 edition)
- SQLite

### Running the service
```bash
cargo run
```
The service listens on `0.0.0.0:3000` by default.

### Database
The service uses SQLite. The database file `sanshain.db` will be created automatically on the first run, and migrations will be applied.

SanShain (Japanese for "Sunshine") is a specialized REST service designed to manage, split, and distribute OpenAPI specifications. It acts as a central repository that allows microservices to "provide" their full API definitions and clients to "require" only the specific snippets (endpoints and necessary DTOs) they actually use.

## Use Cases

- **Contract-First Development**: Services publish their API contracts, and clients consume exactly what they need.
- **Dependency Tracking**: Automatically track which clients are using which specific endpoints across different branches.
- **Impact Analysis**: Identify unused endpoints or missing requirements before they cause issues in production.
- **Client Generation**: Provide minimal, focused YAML snippets to clients for lightweight code generation in any language.

## API Endpoints

### 1. Provide API
`POST /provide`
Registers or updates the OpenAPI specification for a specific service and branch.

- **Parameters**:
  - `servicename`: Name of the service providing the API.
  - `branch`: The git branch (e.g., `main`, `feature/new-login`).
  - `openapi.yaml`: The full OpenAPI 3.x specification.
- **Behavior**: The service parses the YAML, splits it into individual endpoint definitions (including all referenced schemas/DTOs), and stores them in the database.

### 2. Require API
`GET /require`
Retrieves a specific endpoint definition and registers the client's dependency.

- **Parameters**:
  - `clientname`: Name of the client requesting the API.
  - `servicename`: Name of the target service.
  - `branch`: The specific branch to pull from.
  - `url` (optional): The specific endpoint URL path.
- **Returns**: A YAML snippet containing only the requested endpoint and its required DTOs.
- **Behavior**: SanShain records that `clientname` is now a consumer of the specified endpoint.

### 3. Report API
`GET /report`
Generates a dependency graph and health report for a specific branch.

- **Parameters**:
  - `branch`: The branch to analyze.
- **Returns**: A JSON object mapping clients to services and endpoints.

## Example Report

```json
{
  "branch": "main",
  "graph": [
    {
      "client": "MobileApp-Android",
      "dependencies": [
        {
          "service": "UserService",
          "endpoint": "/users/{id}",
          "status": "active"
        }
      ]
    },
    {
      "client": "WebPortal",
      "dependencies": [
        {
          "service": "UserService",
          "endpoint": "/users/login",
          "status": "active"
        },
        {
          "service": "OrderService",
          "endpoint": "/orders/create",
          "status": "missing_provider"
        }
      ]
    }
  ],
  "warnings": [
    {
      "type": "UNUSED_ENDPOINT",
      "service": "UserService",
      "endpoint": "/internal/debug-info",
      "message": "This endpoint is provided but has no registered clients."
    },
    {
      "type": "MISSING_ENDPOINT",
      "client": "WebPortal",
      "service": "OrderService",
      "endpoint": "/orders/create",
      "message": "Client requires this endpoint but it is not provided in branch 'main'."
    }
  ]
}
```

## Getting Started

### Prerequisites
- Rust (latest stable)
- A running database (PostgreSQL/MongoDB - configured in `.env`)

### Installation
```bash
cargo build --release
```

### Running Tests
```bash
cargo test
```
