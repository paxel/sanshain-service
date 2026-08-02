# User Guide

This guide covers day-to-day workflows for developers using Sanshain Service.

### TL;DR
- **Publish**: Producers upload their API specs (OpenAPI, AsyncAPI, Proto) to Sanshain, versioned and with a declared stability.
- **Consume**: Consumers pin an exact version and download only the specific endpoint snippets they need.
- **Verify**: Use **Dry Run** mode in PRs to validate specs and preview version classification before they merge.
- **Explore**: Use the Web UI to navigate the dependency graph and audit logs.

---

## Core Concepts
- **Provide**: Upload a spec. Sanshain reads the version from the spec itself (`info.version`, or `// sanshain-version:` for proto) and splits it into per-endpoint snippets.
- **Stability**: Declared on every Provide. `snapshot` = overwritable work-in-progress that expires when unused; `ga` = immutable, the number is permanently claimed.
- **Require**: Request a snippet for a single endpoint at a pinned version and record the dependency.
- **Pin**: A Consumer's exact version choice, written in its own configuration. No ranges, no `latest`, no fallback.
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
1. Plugin uploads your spec to `/provide` (or `/provide/asyncapi`, `/provide/grpc`), declaring `snapshot` or `ga` stability (typically derived from your git branch — see [sanshain.yaml](sanshain-yaml.md#how-the-plugin-decides-stability)).
2. Sanshain reads the version from the spec and splits the file into standalone snippets per operation.
3. **Version Rules**: a GA number is immutable — re-providing it with different content returns `409 Conflict` with a `proposed_version` to publish as instead. A GA whose changes against the previous GA are breaking without a major bump is also rejected with the correct proposal.
4. **Idempotency**: Byte-identical specs are a no-op — CI re-runs never fight.
5. **Snapshots**: Overwritable, last writer wins; the previous provider is named in the audit trail.

### Multi-Producer Topics (AsyncAPI)

Kafka topic names are a **global namespace**, so a topic (audit log, DLQ, …) can have many
producers. Sanshain tracks AsyncAPI compatibility per **message**: on GA provides, every *named*
`publish` message registers a contract keyed by `(channel, message name)`, owned by the first
Producer to publish it. The owner may widen its own message; a second producer of the same
channel + message name is accepted only if its payload is identical, otherwise rejected with
`409` naming the owner. Snapshots are never contract-checked. **Name your messages**
(`name`/`title`) — unnamed messages have no cross-service identity and are skipped. See
[API Lifecycle §6](api-lifecycle.md#6-multi-producer-topics-asyncapi-message-contracts) for the
full rules.

### Requiring an Endpoint
1. Plugin calls `/require` for a specific path + method at the pinned version.
2. Sanshain resolves immediately: GA preferred, else the same-numbered snapshot, else `404`. Nothing waits.
3. Sanshain returns a minimal YAML/Proto containing only that operation and its models.
4. Plugin generates client code from the returned snippet.

---

## Web Dashboard Features

#### Producers (`/producers.html`)
- Browse registered producers and their version lines (version, stability, snapshot expiry).
- View endpoint usage and availability.
- Copy or download endpoint specifications.

#### Dependency Graph (`/graph.html`)
- High-performance SVG visualization of all service relationships.
- Highlights **Outdated** pins (below the latest GA) and **Snapshot-pinned** dependencies (building against overwritable content).
- Interactive tooltips with direct links to specs.
- Export as high-resolution PNG.

#### Audit Timeline (`/audit.html`)
- **Administrators only.** The page and the data behind it are restricted, and
  the Audit link is hidden from the navigation for everyone else.
- Global log of all spec updates.
- Side-by-side diff viewer for every change.

---

## Next Steps
- [**CI Integration**](ci-integration.md) — Setting up automation.
- [**Corporate Best Practices**](corporate-best-practices.md) — Recommendations for enterprise usage.
- [**Administration**](administration.md) — Managing users and settings.
- [**Troubleshooting**](troubleshooting.md) — Solving common issues.
