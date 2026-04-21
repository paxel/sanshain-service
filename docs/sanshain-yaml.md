# `sanshain.yaml` — Client Configuration Format

This document defines the standard `sanshain.yaml` configuration file format used by Sanshain client plugins (Maven, Gradle, Cargo, Go, npm, etc.) to configure their interaction with a Sanshain Service instance.

## Overview

A `sanshain.yaml` file lives in the root of a project and declares:

1. **Connection settings** — Sanshain server URL, timeouts, compression.
2. **Provide** — Which API spec this project publishes (OpenAPI, AsyncAPI, or Proto).
3. **Requires** — Which endpoints/channels from other services this project depends on.

Client plugins read this file and translate it into the appropriate `/provide`, `/require`, and `/require-bundle` API calls.

## Full Example

```yaml
sanshainUrl: https://sanshain.example.com
timeout: 120
compression: true
clientName: order-service

provide:
  serviceName: order-service
  openApiFile: src/main/resources/openapi.yaml

requires:
  - serviceName: user-service
    branch: main
    outputDirectory: target/generated-sources/sanshain/user-service
    timeout: 60
    endpoints:
      - method: GET
        path: /api/v1/users
      - method: GET
        path: /api/v1/users/{id}
      - method: POST
        path: /api/v1/users
  - serviceName: inventory-service
    branch: main
    outputDirectory: target/generated-sources/sanshain/inventory-service
    endpoints:
      - method: POST
        path: /api/v1/orders
```

## Field Reference

### Top-Level Fields

| Field         | Type    | Required | Default | Description                                                                 |
|---------------|---------|----------|---------|-----------------------------------------------------------------------------|
| `sanshainUrl` | string  | **yes**  | —       | Base URL of the Sanshain Service instance.                                  |
| `clientName`  | string  | **yes**  | —       | Name identifying this project as a client in dependency tracking.           |
| `timeout`     | integer | no       | `30`    | Default timeout in seconds for require/require-bundle calls (long-polling). |
| `compression` | boolean | no       | `false` | Whether to request gzip-compressed responses.                               |

### `provide` Section

Declares the spec this project publishes. Omit entirely if this project only consumes APIs.

| Field           | Type   | Required | Default                  | Description                                                                |
|-----------------|--------|----------|--------------------------|----------------------------------------------------------------------------|
| `serviceName`   | string | **yes**  | —                        | Name of the service being provided.                                        |
| `apiType`       | string | no       | `openapi`                | Type of API: `openapi`, `asyncapi`, or `proto`.                            |
| `openApiFile`   | string | no       | —                        | Path to the OpenAPI YAML file (use if `apiType` is `openapi`).             |
| `asyncApiFile`  | string | no       | —                        | Path to the AsyncAPI YAML file (use if `apiType` is `asyncapi`).            |
| `protoFile`     | string | no       | —                        | Path to the `.proto` file (use if `apiType` is `proto`).                   |
| `branch`        | string | no       | *auto-detected from VCS* | Branch name. Plugins should auto-detect from Git; override here if needed. |

### `requires` Section

A list of service dependencies. Each entry results in a single `POST /require-bundle` call (or `GET /require` if only one endpoint is listed).

| Field             | Type    | Required | Default                  | Description                                                     |
|-------------------|---------|----------|--------------------------|-----------------------------------------------------------------|
| `serviceName`     | string  | **yes**  | —                        | Name of the service to require endpoints from.                  |
| `apiType`         | string  | no       | `openapi`                | Type of API: `openapi`, `asyncapi`, or `proto`.                 |
| `branch`          | string  | no       | *same as provide branch* | Branch to require from. Defaults to the current project branch. |
| `outputDirectory` | string  | **yes**  | —                        | Directory where the merged specification file will be written.  |
| `timeout`         | integer | no       | *top-level timeout*      | Per-service timeout override for long-polling.                  |
| `endpoints`       | list    | **yes**  | —                        | List of endpoints/channels to require (see below).              |

### `endpoints` Entry

| Field    | Type   | Required | Description                                                            |
|----------|--------|----------|------------------------------------------------------------------------|
| `method` | string | **yes**  | HTTP method (GET, POST, etc.) or Operation (PUB, SUB) or gRPC Method. |
| `path`   | string | **yes**  | API path or AsyncAPI Channel or gRPC Service.                          |

## How Plugins Use This

### Provide Phase

1. Read `provide.openApiFile`.
2. Detect branch from VCS (or use some override).
3. Call `POST /provide` with `{ servicename, branch, openapi_yaml }`.

### Require Phase

For each entry in `requires`:

1. If the entry has **multiple endpoints**: call `POST /require-bundle` with:
   ```json
   {
     "clientname": "<clientName>",
     "servicename": "<serviceName>",
     "branch": "<branch>",
     "endpoints": [
       { "path": "/api/v1/users", "method": "GET" },
       { "path": "/api/v1/users/{id}", "method": "GET" }
     ],
     "timeout": 60
   }
   ```
   The response is a **single merged OpenAPI YAML** with all requested paths and deduplicated schemas.

2. If the entry has **one endpoint**: call `GET /require` with query parameters (backward compatible).

3. Write the response YAML to `outputDirectory`.

4. Optionally, run code generation (e.g., OpenAPI Generator) on the output.

### Why Bundle Matters

When a client requires multiple endpoints from the same service, those endpoints often share DTOs (e.g., `UserDTO`, `AddressDTO`). Without bundling:

- Each endpoint returns its own YAML snippet with its own copy of shared schemas.
- In typed languages (Java, C#, Go), this produces **duplicate classes** that are not interchangeable.
- Maven/Gradle codegen places them in separate packages or directories, causing compilation errors or type mismatches.

The `/require-bundle` endpoint solves this by returning a single spec with **one copy of each schema**, ensuring generated code compiles cleanly and DTOs are shared across all operations.

## Authentication

If the Sanshain instance requires authentication, plugins should support:

- **API token**: Pass `Authorization: Bearer san_...` header on all requests.
- Token can be configured via environment variable (`SANSHAIN_TOKEN`) or a separate credentials file (e.g., Maven `settings.xml`).

The `sanshain.yaml` file should **not** contain secrets. Tokens belong in environment variables or credential stores.
