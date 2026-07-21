# User Guide

This guide covers day-to-day workflows for developers using Sanshain Service.

### TL;DR
- **Publish**: Services upload their API specs (OpenAPI, AsyncAPI, Proto) to Sanshain.
- **Consume**: Clients download only the specific endpoint snippets they need.
- **Verify**: Use **Dry Run** mode in PRs to catch breaking changes before they merge.
- **Explore**: Use the Web UI to navigate the dependency graph and audit logs.

---

## Core Concepts
- **Provide**: Upload a spec. Sanshain splits it into per-endpoint snippets.
- **Require**: Request a snippet for a single endpoint and record the dependency.
- **Protected Branch**: A branch (e.g., `main`) where breaking changes are blocked.
- **Feature Branch Fallback**: If an endpoint isn't on your branch, Sanshain looks on `main`.
- **Dry Run**: Validation-only mode for CI pipelines (`dry_run: true`).

## Build Tool Integration
Integrate Sanshain directly into your build process using dedicated plugins. They handle authentication, spec upload, and code generation.

- [**Maven Plugin**](https://github.com/paxel/sanshain-maven-plugin) (Java / Kotlin)
- [**Rust (Cargo)**](https://github.com/paxel/sanshain) (Rust)
- [**Go CLI**](https://github.com/paxel/sanshain-go) (Go)
- [**JavaScript/TypeScript**](https://github.com/paxel/sanshain-js) (Node.js / CI)
- [**Conan Plugin**](https://github.com/paxel/sanshain-conan) (C / C++)

The API is formally specified in [`api.yaml`](../api.yaml).

---

## How it Works

### Providing a Spec
1. Plugin uploads your spec to `/provide` (or `/provide/asyncapi`, `/provide/grpc`).
2. Sanshain splits the file into standalone snippets per operation.
3. **Compatibility Check**: On protected branches, breaking changes return `409 Conflict`.
4. **Idempotency**: Identical specs are skipped to avoid version inflation.
5. **Ownership**: Feature branches track who "owns" an endpoint to prevent cross-service conflicts.

### Multi-Producer Topics (AsyncAPI)

Kafka topic names are a **global namespace**, so a topic (audit log, DLQ, …) can have many
producers. Sanshain tracks AsyncAPI compatibility per **message**: every *named* `publish`
message registers a contract keyed by `(branch, channel, message name)`, owned by the first
service to publish it. The owner may widen its own message; a second producer of the same
channel + message name is accepted only if its payload is identical, otherwise rejected with
`409` naming the owner. **Name your messages** (`name`/`title`) — unnamed messages have no
cross-service identity and are skipped. See
[API Lifecycle §6](api-lifecycle.md#6-multi-producer-topics-asyncapi-message-contracts) for the
full rules.

### Requiring an Endpoint
1. Plugin calls `/require` for a specific path + method.
2. Sanshain returns a minimal YAML/Proto containing only that operation and its models.
3. Plugin generates client code from the returned snippet.
4. **Long-polling**: If the provider hasn't published yet, the request waits (up to a timeout).

---

## Web Dashboard Features

#### Services (`/services.html`)
- Browse registered services and branches.
- View endpoint usage and availability.
- Copy or download endpoint specifications.

#### Dependency Graph (`/graph.html`)
- High-performance SVG visualization of all service relationships.
- Interactive tooltips with direct links to specs.
- Export as high-resolution PNG.

#### Audit Timeline (`/audit.html`)
- Global log of all spec updates.
- Side-by-side diff viewer for every change.

---

## Next Steps
- [**CI Integration**](ci-integration.md) — Setting up automation.
- [**Corporate Best Practices**](corporate-best-practices.md) — Recommendations for enterprise usage.
- [**Administration**](administration.md) — Managing users and settings.
- [**Troubleshooting**](troubleshooting.md) — Solving common issues.
