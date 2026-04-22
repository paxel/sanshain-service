# `sanshain.yaml` — Client Configuration Format

This document defines the standard `sanshain.yaml` configuration file format used by Sanshain client plugins (Maven, Gradle, Cargo, Go, npm, etc.) to configure their interaction with a Sanshain Service instance.

## Overview

A `sanshain.yaml` file lives in the root of a project and declares:

1. **Connection settings** — Sanshain server URL, timeouts, compression.
2. **Provide(s)** — Which API spec(s) this project publishes (OpenAPI, AsyncAPI, and/or Proto).
3. **Requires** — Which endpoints/channels from other services this project depends on.

Client plugins read this file and translate it into the appropriate `/provide`, `/provide/asyncapi`, `/provide/grpc`, `/require`, and `/require-bundle` API calls.

## Full Example

This example shows a "Gateway Service" that provides an OpenAPI spec and consumes APIs using three different protocols: OpenAPI (REST), AsyncAPI (Messaging), and Proto (gRPC).

```yaml
sanshainUrl: https://sanshain.example.com
serviceName: gateway-service
timeout: 120
compression: true

# Provides both OpenAPI and gRPC specifications
provides:
  - file: src/main/resources/openapi.yaml
  - file: src/main/resources/gateway.proto
    apiType: proto

requires:
  # Requires a REST endpoint from user-service
  - serviceName: user-service
    apiType: openapi
    branch: main
    outputDirectory: target/generated-sources/sanshain/user-service
    endpoints:
      - method: GET
        path: /api/v1/users/{id}

  # Requires a Messaging channel from notification-service
  - serviceName: notification-service
    apiType: asyncapi
    outputDirectory: target/generated-sources/sanshain/notification-service
    endpoints:
      - method: PUB
        path: notifications.email

  # Requires a gRPC method from inventory-service
  - serviceName: inventory-service
    apiType: proto
    outputDirectory: target/generated-sources/sanshain/inventory-service
    endpoints:
      - method: GetProduct
        path: inventory.v1.InventoryService
```

## Field Reference

### Top-Level Fields

| Field         | Type    | Required | Default | Description                                                                 |
|---------------|---------|----------|---------|-----------------------------------------------------------------------------|
| `sanshainUrl` | string  | **yes**  | —       | Base URL of the Sanshain Service instance.                                  |
| `serviceName` | string  | **yes**  | —       | Name identifying this project (both for providing and requiring APIs).      |
| `timeout`     | integer | no       | `30`    | Default timeout in seconds for require/require-bundle calls (long-polling). |
| `compression` | boolean | no       | `false` | Whether to request gzip-compressed responses.                               |

### `provide` / `provides` Section

Declares the specification(s) this project publishes. Omit entirely if this project only consumes APIs.

Plugins should support both a single `provide` object and a `provides` list for services that provide multiple types of endpoints (e.g., REST and gRPC).

| Field           | Type   | Required | Default                  | Description                                                                |
|-----------------|--------|----------|--------------------------|----------------------------------------------------------------------------|
| `file`          | string | **yes**  | —                        | Path to the specification file.                                            |
| `apiType`       | string | no       | `openapi`                | Type of API: `openapi`, `asyncapi`, or `proto`.                            |
| `branch`        | string | no       | *auto-detected from VCS* | Branch name. Plugins should auto-detect from Git; override here if needed. |

**Example (Multiple Protocols):**
```yaml
serviceName: order-service
provides:
  - file: specs/openapi.yaml
  - file: specs/order.proto
    apiType: proto
```

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

| Field    | Type   | Required | Description                                                               |
|----------|--------|----------|---------------------------------------------------------------------------|
| `method` | string | **yes**  | HTTP method (`GET`, `POST`), Operation (`PUB`, `SUB`), or gRPC Method.    |
| `path`   | string | **yes**  | API path, AsyncAPI Channel, or gRPC Service (full package + service name). |

## Protocol Examples

### OpenAPI (REST)

Standard REST API integration.

```yaml
serviceName: user-service
provide:
  file: specs/openapi.yaml

requires:
  - serviceName: auth-service
    apiType: openapi
    endpoints:
      - method: POST
        path: /v1/login
```

**Calling the endpoint:**
Clients typically use generated code (e.g., via `openapi-generator`) to call the REST endpoint using standard HTTP libraries.

### AsyncAPI (Message-Driven)

For services communicating via message brokers (Kafka, RabbitMQ, etc.).

```yaml
serviceName: payment-service
provide:
  apiType: asyncapi
  file: specs/asyncapi.yaml

requires:
  - serviceName: order-service
    apiType: asyncapi
    endpoints:
      - method: SUB
        path: orders.created
```

**Calling the endpoint:**
"Calling" an AsyncAPI endpoint usually means subscribing to (`SUB`) or publishing to (`PUB`) a channel. In this example, `payment-service` requires the `orders.created` channel to listen for new orders. The client plugin will download a snippet containing only the relevant channel and its associated message/schema definitions.

### gRPC / Proto

For high-performance RPC communication.

```yaml
serviceName: inventory-service
provide:
  apiType: proto
  file: src/main/proto/inventory.proto

requires:
  - serviceName: warehouse-service
    apiType: proto
    endpoints:
      - method: GetStock
        path: warehouse.v1.WarehouseService
```

**Calling the endpoint:**
The client plugin will download a `.proto` file containing the `WarehouseService` definition and only the `GetStock` method (including all transitively referenced messages). You can then use `protoc` or your language's gRPC toolkit to generate a client stub and call `GetStock` as a local-looking function.

## How Plugins Use This

### Provide Phase

1. For each entry in `provides` (or for the single `provide` object):
   a. Read the specification file content.
   b. Detect branch from VCS (or use override).
   c. Call the appropriate endpoint based on `apiType`:
      - `openapi`: `POST /provide`
      - `asyncapi`: `POST /provide/asyncapi`
      - `proto`: `POST /provide/grpc`

### Require Phase

For each entry in `requires`:

1. If the entry has **multiple endpoints**: call `POST /require-bundle` with:
   ```json
   {
     "clientname": "<root.serviceName>",
     "servicename": "<requires[i].serviceName>",
     "branch": "<requires[i].branch>",
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
