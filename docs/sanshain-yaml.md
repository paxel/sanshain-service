# `sanshain.yaml` — Client Configuration Format

This document defines the standard `sanshain.yaml` configuration file format used by Sanshain client plugins (Maven, Gradle, Cargo, Go, npm, etc.) to configure their interaction with a Sanshain Service instance.

## Overview

A `sanshain.yaml` file lives in the root of a project and declares:

1. **Connection settings** — Sanshain server URL (optional in file), compression.
2. **Provide(s)** — Which API spec(s) this project publishes (OpenAPI, AsyncAPI, and/or Proto). The **version is never configured here** — it lives in the spec file itself (`info.version`, or a `// sanshain-version:` comment for proto). What the file configures is how the plugin decides the **stability** (`snapshot` or `ga`) of each Provide.
3. **Requires** — Which endpoints/channels from other services this project depends on, each **pinned to an exact version**.

Client plugins read this file and translate it into the appropriate `/provide`, `/provide/asyncapi`, `/provide/grpc`, `/require`, and `/require-bundle` API calls.

## Full Example

This example shows a "Gateway Service" that provides an OpenAPI spec and consumes APIs using three different protocols: OpenAPI (REST), AsyncAPI (Messaging), and Proto (gRPC). 

> **Pro-Tip**: `sanshainUrl` is omitted here as it's better provided via environment variables in corporate environments.

```yaml
serviceName: gateway-service
compression: true

# Provides both OpenAPI and gRPC specifications.
# Stability is NOT configured here: every provide is a `snapshot` unless the
# build sets the ga switch (e.g. SANSHAIN_GA=true) — see "How stability is decided".
# The versions are read from the spec files (info.version / // sanshain-version:).
provides:
  - file: src/main/resources/openapi.yaml
  - file: src/main/resources/gateway.proto
    apiType: proto

requires:
  # Requires a REST endpoint from user-service
  - serviceName: user-service
    apiType: openapi
    version: 2.3.0
    outputDirectory: target/generated-sources/sanshain/user-service
    endpoints:
      - method: GET
        path: /api/v1/users/{id}

  # Requires a Messaging channel from notification-service
  - serviceName: notification-service
    apiType: asyncapi
    version: 1.1.0
    outputDirectory: target/generated-sources/sanshain/notification-service
    endpoints:
      - method: PUB
        path: notifications.email

  # Requires a gRPC method from inventory-service
  - serviceName: inventory-service
    apiType: proto
    version: 3.0.1
    outputDirectory: target/generated-sources/sanshain/inventory-service
    endpoints:
      - method: GetProduct
        path: inventory.v1.InventoryService
```

## Field Reference

### Top-Level Fields

| Field             | Type    | Required | Default              | Description                                                                                       |
|-------------------|---------|----------|----------------------|---------------------------------------------------------------------------------------------------|
| `sanshainUrl`     | string  | no       | —                    | Base URL of the Sanshain Service instance. Recommended to provide via ENV.                        |
| `serviceName`     | string  | **yes**  | —                    | Name identifying this project (both for providing and requiring APIs).                            |
| `compression`     | boolean | no       | `false`              | Whether to request gzip-compressed responses.                                                     |

### How stability is decided

Sanshain never sees your git repository, and the plugins do no git detection either. The rule is
deliberately binary — no guessing, no branch magic:

- **Every Provide is a `snapshot` by default.** A developer building locally can never
  accidentally release.
- **`ga` is an explicit act**: the build sets the ga switch — the environment variable
  `SANSHAIN_GA=true` (all clients), the Maven property `-Dsanshain.ga=true`, or the CLI flag
  `--ga`. CI sets it on its protected-branch pipelines; nobody else touches it.

`sanshain.yaml` itself carries no stability configuration at all.

## Environment Overrides & Best Practices

In a corporate environment, you should avoid hardcoding the `sanshainUrl` in `sanshain.yaml` to maintain flexibility across different environments (local, dev, prod) and to avoid rewriting git history if the service moves.

All official plugins support providing the URL via:

1. **Environment Variable**: `SANSHAIN_URL` (e.g., `export SANSHAIN_URL=https://sanshain.corp.com`).
2. **Build Tool Settings**:
   - **Maven**: `<sanshain.url>` property in `pom.xml` or `settings.xml`.
   - **Gradle**: `sanshain.url` in `gradle.properties`.
3. **CLI Flag**: `--url` or `-u` depending on the client.

If `sanshainUrl` is present in `sanshain.yaml`, it will be used as a default but can be overridden by the methods above.

### `provide` / `provides` Section

Declares the specification(s) this project publishes. Omit entirely if this project only consumes APIs.

Plugins should support both a single `provide` object and a `provides` list for services that provide multiple types of endpoints (e.g., REST and gRPC).

| Field       | Type   | Required | Default                          | Description                                                                  |
|-------------|--------|----------|----------------------------------|------------------------------------------------------------------------------|
| `file`      | string | **yes**  | —                                | Path to the specification file.                                              |
| `apiType`   | string | no       | `openapi`                        | Type of API: `openapi`, `asyncapi`, or `proto`.                              |
| `retired`   | bool   | no       | `false`                          | The project no longer provides this family. See **Retiring a protocol** below.|

#### Retiring a protocol

Sanshain cannot tell a dropped protocol from a pipeline that merely stopped running, so removing a
`provide` entry does nothing on its own — the service keeps its old capability tag, graph edges and
contracts. To actually retire a family, keep the entry and set `retired: true` (or drop it and
declare the removal another way the plugin supports). The plugin turns that into
`POST /admin/producers/{name}/retire/{apiType}`, which clears the `messaging`/`grpc` tag, removes
the family from the current dependency graph, and releases its AsyncAPI channel-message contracts —
**without** deleting version history or breaking Consumers still pinned to the old versions.

The **version** of a Provide is read from the spec file itself: `MAJOR[.MINOR[.PATCH]]`, optionally `v`-prefixed — omitted parts are zero and the stored form is always the full three-part version:

- **OpenAPI / AsyncAPI**: the document's `info.version` field.
- **Proto**: a mandatory `// sanshain-version: MAJOR.MINOR.PATCH` comment in the file (conventionally in the header). A missing, malformed or conflicting marker rejects the Provide.

**Example (Multiple Protocols):**
```yaml
serviceName: order-service
provides:
  - file: specs/openapi.yaml        # version read from info.version
  - file: specs/order.proto         # version read from // sanshain-version:
    apiType: proto
```

### `requires` Section

A list of service dependencies. Each entry results in a single `POST /require-bundle` call (or `GET /require` if only one endpoint is listed).

| Field             | Type   | Required | Default   | Description                                                              |
|-------------------|--------|----------|-----------|--------------------------------------------------------------------------|
| `serviceName`     | string | **yes**  | —         | Name of the service to require endpoints from.                           |
| `apiType`         | string | no       | `openapi` | Type of API: `openapi`, `asyncapi`, or `proto`.                          |
| `version`         | string | **yes**  | —         | The exact pinned version (`MAJOR.MINOR.PATCH`). No ranges, no `latest`.  |
| `outputDirectory` | string | **yes**  | —         | Directory where the merged specification file will be written.           |
| `endpoints`       | list   | **yes**  | —         | List of endpoints/channels to require (see below).                       |

The **Pin** is the whole contract: what your build downloads changes only when someone edits `version`. A Pin that does not exist on the server fails the require with `404`.

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
    version: 1.2.0
    outputDirectory: generated/auth-service
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
    version: 2.0.0
    outputDirectory: generated/order-service
    endpoints:
      - method: SUB
        path: orders.created
```

**Calling the endpoint:**
"Calling" an AsyncAPI endpoint usually means subscribing to (`SUB`) or publishing to (`PUB`) a channel. In this example, `payment-service` requires the `orders.created` channel to listen for new orders. The client plugin will download a snippet containing only the relevant channel and its associated message/schema definitions.

### AsyncAPI Version Compatibility (v2 → v3)

Sanshain supports **both AsyncAPI 2.x and 3.x** specifications. The version is auto-detected from the `asyncapi:` field in your YAML. Both versions produce the same internal representation and are fully interoperable — a service providing with v2 can be required by a client expecting v3 semantics and vice versa.

However, there is an important difference in how **channel identifiers** are resolved:

|                      | AsyncAPI 2.x                                 | AsyncAPI 3.x                                                 |
|----------------------|----------------------------------------------|--------------------------------------------------------------|
| **Operations**       | `channels.*.publish` / `channels.*.subscribe` | `operations.*.action: send` / `operations.*.action: receive` |
| **Internal mapping** | `publish` → `PUB`, `subscribe` → `SUB`       | `send` → `PUB`, `receive` → `SUB`                            |
| **Channel identifier** | The channel key (e.g., `user-created`)     | The channel `address` field (e.g., `user/signedup`)          |

> ⚠️ **The 2.x perspective convention.** Sanshain reads AsyncAPI 2.x `publish`/`subscribe` from
> the **application's** perspective: `publish` means *this service publishes to the channel*,
> `subscribe` means *this service consumes it*. The official AsyncAPI 2.x specification defines
> those keywords from the **client's** perspective — exactly inverted. Sanshain deliberately uses
> the application-perspective reading because it matches the unambiguous 3.x `send`/`receive`
> mapping. Write your 2.x documents accordingly: a spec authored with the spec-literal reading
> registers its contracts — and harvests its subscriptions — exactly backwards.

**Key restriction when upgrading from v2 to v3:**

In AsyncAPI 2.x, the channel **key name** (e.g., `orders.created`) is used as the identifier stored in Sanshain. In AsyncAPI 3.x, the channel **`address`** field is used instead, and the key name (e.g., `OrderCreated`) is only a local label.

This means that when writing your `sanshain.yaml` requires for a service that provides an AsyncAPI 3.x spec, you must use the **address** (the actual topic/path name), not the channel key:

```yaml
# AsyncAPI 3.x spec defines:
#   channels:
#     OrderCreated:          ← this is just a local key
#       address: orders.created  ← this is what Sanshain stores

requires:
  - serviceName: order-service
    apiType: asyncapi
    version: 2.0.0
    outputDirectory: generated/order-service
    endpoints:
      - method: SUB
        path: orders.created    # ✅ use the address, not the channel key
      # path: OrderCreated      # ❌ wrong — this is the key, not the address
```

If a v3 channel has no `address` field, Sanshain falls back to the channel key name.

**Migration checklist (v2 → v3):**

1. Update your AsyncAPI spec to v3 format (move operations out of channels, use `send`/`receive` actions).
2. Ensure each channel has an explicit `address` field matching the topic/path name your consumers expect.
3. Update all `sanshain.yaml` `requires` entries to use the channel **address** in the `path` field.
4. Bump `info.version` and re-provide the updated spec via `POST /provide/asyncapi` — Sanshain will parse it as v3 automatically.

### gRPC / Proto

For high-performance RPC communication.

```yaml
serviceName: inventory-service
provide:
  apiType: proto
  file: src/main/proto/inventory.proto   # must carry // sanshain-version: MAJOR.MINOR.PATCH

requires:
  - serviceName: warehouse-service
    apiType: proto
    version: 1.4.2
    outputDirectory: generated/warehouse-service
    endpoints:
      - method: GetStock
        path: warehouse.v1.WarehouseService
```

**Calling the endpoint:**
The client plugin will download a `.proto` file containing the `WarehouseService` definition and only the `GetStock` method (including all transitively referenced messages). You can then use `protoc` or your language's gRPC toolkit to generate a client stub and call `GetStock` as a local-looking function.

## How Matching Works Between `sanshain.yaml` and Spec Files

When a service **provides** a spec file, Sanshain splits it into individual endpoints and stores each one indexed by a **path** (or channel/service) and a **method** (or operation/RPC name). When a client **requires** endpoints, Sanshain looks up the stored entries using the `path` and `method` from the `sanshain.yaml` at the pinned `version`. Understanding this matching is critical — if the pinned version exists but the values don't align, the require call fails `410` (the endpoint is Absent from that version).

The table below summarizes what Sanshain extracts from each spec type and what you must write in `sanshain.yaml` to match:

| Protocol     | Spec provides → Sanshain stores                                  | `sanshain.yaml` `path`                                                  | `sanshain.yaml` `method`                                            |
|--------------|-------------------------------------------------------------------|-------------------------------------------------------------------------|---------------------------------------------------------------------|
| OpenAPI      | Each `paths.*` entry + HTTP verb                                  | The OpenAPI path (e.g., `/api/v1/users/{id}`)                           | The HTTP verb in uppercase (`GET`, `POST`, `PUT`, `DELETE`, `PATCH`) |
| AsyncAPI 2.x | Each `channels.*` key + `publish`/`subscribe`                     | The channel key (e.g., `orders.created`)                                | `PUB` or `SUB`                                                      |
| AsyncAPI 3.x | Each `operations.*` entry → resolved channel `address` + `send`/`receive` | The channel **address** (e.g., `orders.created`)                        | `PUB` or `SUB`                                                      |
| Proto        | Each `service` + `rpc` method                                     | The fully qualified service name (e.g., `inventory.v1.InventoryService`) | The exact RPC method name (e.g., `GetProduct`) — **case-sensitive**  |

### OpenAPI Matching — Detailed Example

Given this OpenAPI spec provided by `user-service` (its `info.version` makes this version `1.0.0` of the line):

```yaml
openapi: 3.0.3
info:
  title: User Service
  version: 1.0.0
paths:
  /api/v1/users:
    get:
      summary: List users
      responses:
        '200':
          description: OK
    post:
      summary: Create user
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/CreateUserRequest'
      responses:
        '201':
          description: Created
  /api/v1/users/{id}:
    get:
      summary: Get user by ID
      parameters:
        - name: id
          in: path
          required: true
          schema:
            type: string
      responses:
        '200':
          description: OK
components:
  schemas:
    CreateUserRequest:
      type: object
      properties:
        name:
          type: string
```

Sanshain splits this into **3 endpoints**:

| Stored `path`        | Stored `method` |
|----------------------|-----------------|
| `/api/v1/users`      | `GET`           |
| `/api/v1/users`      | `POST`          |
| `/api/v1/users/{id}` | `GET`           |

To require the "get user by ID" and "create user" endpoints, write:

```yaml
requires:
  - serviceName: user-service
    apiType: openapi
    version: 1.0.0
    outputDirectory: generated/user-service
    endpoints:
      - method: GET
        path: /api/v1/users/{id}    # ✅ matches the OpenAPI path exactly
      - method: POST
        path: /api/v1/users          # ✅ matches the OpenAPI path exactly
```

> **Lenient path matching:** Sanshain normalizes path variable names, so `/api/v1/users/{id}` and `/api/v1/users/{userId}` will both match. Redundant slashes and trailing whitespace are also ignored.

The client plugin calls `POST /require-bundle` and receives a **single merged OpenAPI YAML** containing only the two requested operations and their shared schemas (e.g., `CreateUserRequest`), ready for code generation.

### AsyncAPI Matching — Detailed Example

Given this AsyncAPI 2.x spec provided by `order-service` (version `1.0.0` via `info.version`):

```yaml
asyncapi: 2.6.0
info:
  title: Order Events
  version: 1.0.0
channels:
  orders.created:
    publish:
      message:
        payload:
          type: object
          properties:
            orderId:
              type: string
            amount:
              type: number
  orders.cancelled:
    subscribe:
      message:
        payload:
          type: object
          properties:
            orderId:
              type: string
            reason:
              type: string
```

Sanshain splits this into **2 endpoints**:

| Stored `path` (channel) | Stored `method` |
|-------------------------|-----------------|
| `orders.created`        | `PUB`           |
| `orders.cancelled`      | `SUB`           |

To subscribe to new order events, write:

```yaml
requires:
  - serviceName: order-service
    apiType: asyncapi
    version: 1.0.0
    outputDirectory: generated/order-service
    endpoints:
      - method: PUB
        path: orders.created        # ✅ matches the channel key
```

> **Important:** `PUB` and `SUB` map to the **provider's** perspective. If the provider declares `publish` on a channel, the client requires it with `method: PUB`. See the [AsyncAPI v2→v3 section](#asyncapi-version-compatibility-v2--v3) for how this changes with v3.

The client receives an AsyncAPI YAML snippet containing only the `orders.created` channel and its message/schema definitions.

### Proto / gRPC Matching — Detailed Example

Given this `.proto` file provided by `inventory-service` (version `1.0.0` via the mandatory marker):

```protobuf
syntax = "proto3";
// sanshain-version: 1.0.0
package inventory.v1;

service InventoryService {
  rpc GetProduct (GetProductRequest) returns (Product);
  rpc ListProducts (ListProductsRequest) returns (ListProductsResponse);
  rpc UpdateStock (UpdateStockRequest) returns (UpdateStockResponse);
}

message GetProductRequest {
  string product_id = 1;
}

message Product {
  string product_id = 1;
  string name = 2;
  int32 stock = 3;
}

message ListProductsRequest {}
message ListProductsResponse {
  repeated Product products = 1;
}

message UpdateStockRequest {
  string product_id = 1;
  int32 delta = 2;
}

message UpdateStockResponse {
  int32 new_stock = 1;
}
```

Sanshain splits this into **3 endpoints**:

| Stored `path` (service)           | Stored `method` (RPC) |
|-----------------------------------|-----------------------|
| `inventory.v1.InventoryService`   | `GetProduct`          |
| `inventory.v1.InventoryService`   | `ListProducts`        |
| `inventory.v1.InventoryService`   | `UpdateStock`         |

To require `GetProduct` and `ListProducts`, write:

```yaml
requires:
  - serviceName: inventory-service
    apiType: proto
    version: 1.0.0
    outputDirectory: generated/inventory-service
    endpoints:
      - method: GetProduct
        path: inventory.v1.InventoryService       # ✅ full package + service
      - method: ListProducts
        path: inventory.v1.InventoryService       # ✅ same service, different method
```

> **Case-sensitive:** Unlike OpenAPI and AsyncAPI, gRPC method names are **case-sensitive**. `GetProduct` ≠ `getProduct`.

The client receives a `.proto` file containing only the `InventoryService` definition with the two requested methods and all transitively referenced messages (`GetProductRequest`, `Product`, `ListProductsRequest`, `ListProductsResponse`). The `// sanshain-version:` marker travels into the split file, so the version is visible in the artifact itself.

### Quick Reference: What Goes Where

| I want to…                      | `apiType`  | `method` value                           | `path` value                                                   |
|---------------------------------|------------|------------------------------------------|----------------------------------------------------------------|
| Call a REST endpoint            | `openapi`  | `GET`, `POST`, `PUT`, `DELETE`, `PATCH`  | The OpenAPI path (e.g., `/api/v1/orders`)                      |
| Subscribe to a message channel | `asyncapi` | `PUB` or `SUB`                           | The channel name/address (e.g., `orders.created`)              |
| Call a gRPC method              | `proto`    | The RPC method name (e.g., `GetProduct`) | The fully qualified service (e.g., `inventory.v1.InventoryService`) |

## How Plugins Use This

### Provide Phase

1. For each entry in `provides` (or for the single `provide` object):
   a. Read the specification file content. The version travels inside it (`info.version` / `// sanshain-version:`).
   b. Determine the stability: `ga` when the ga switch is set (`SANSHAIN_GA=true` / `-Dsanshain.ga=true` / `--ga`), otherwise `snapshot`.
   c. Call the appropriate endpoint based on `apiType`, with `stability` in the payload:
      - `openapi`: `POST /provide`
      - `asyncapi`: `POST /provide/asyncapi`
      - `proto`: `POST /provide/grpc`

2. **Handle the Response**: All provide endpoints return `202 Accepted` with a JSON body describing what was stored:
   ```json
   {
     "version": "1.4.0",
     "stability": "ga",
     "content_hash": "sha256:a1b2c3...",
     "changes": {
       "inserts": 1,
       "updates": 2,
       "deletes": 0
     }
   }
   ```
   `changes` is the endpoint diff relative to what that exact version stored before; a brand-new version reports its whole endpoint set as inserts.

   **Idempotency**: re-providing byte-identical content is a no-op — `changes` reports all zero. CI re-runs of the same commit never fight.

3. **Handle a `409` rejection**: the body carries `proposed_version` — the next free version, bumped by what actually changed (breaking → major, additive → minor, shape-identical → patch). Every rejection is self-service: set the spec file's version to `proposed_version` (or an appropriate higher one) and republish. There is nothing to pull or sync — the fix is always a version bump in your own spec file. A `400` means the document itself is invalid — most commonly a missing or non-semver version.

### Require Phase

For each entry in `requires`:

1. If the entry has **multiple endpoints**: call `POST /require-bundle` with:
   ```json
   {
     "consumername": "<root.serviceName>",
     "producername": "<requires[i].serviceName>",
     "version": "<requires[i].version>",
     "endpoints": [
       { "path": "/api/v1/users", "method": "GET" },
       { "path": "/api/v1/users/{id}", "method": "GET" }
     ]
   }
   ```
   The response is a **single merged OpenAPI YAML** with all requested paths and deduplicated schemas.

2. If the entry has **one endpoint**: call `GET /require` with query parameters (including `version`).

3. Write the response YAML to `outputDirectory`.

4. Optionally, run code generation (e.g., OpenAPI Generator) on the output.

Resolution is immediate — GA preferred, else the same-numbered snapshot, else failure. Plugins should surface the two failure modes distinctly:

- **`404` (Unknown)**: the pinned version does not exist on the server in either stability. A configuration error — fix the Pin.
- **`410` (Absent)**: the pinned version exists but deliberately does not include a requested endpoint. For `/require-bundle`, the whole bundle fails and the body names the missing endpoints.

## Require-Side Caching

To reduce build times and network traffic, Sanshain supports standard HTTP caching via `ETag` and `If-None-Match` headers on all require endpoints (`/require`, `/require-bundle`, etc.).

1. **First call**: Client makes a require request.
2. **Server response**: Returns the specification with an `ETag` header (e.g., `ETag: "sha256:..."`).
3. **Caching**: Client plugin saves the specification file AND the ETag value (e.g., in a `.sanshain-cache` metadata file).
4. **Subsequent calls**: Client sends the stored ETag in the `If-None-Match` header.
5. **Server check**:
    - If the specification has NOT changed, the server returns `304 Not Modified` with an empty body. The plugin uses the cached file.
    - If the specification HAS changed, the server returns `200 OK` with the new content and a new `ETag`. The plugin updates its cache and generates code.

This mechanism is highly recommended for CI/CD pipelines to avoid redundant code generation when upstream dependencies are stable. Note that a Pin on a GA version can never change content; a Pin on a snapshot can, which is exactly what the ETag detects.

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
