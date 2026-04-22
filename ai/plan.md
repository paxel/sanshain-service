# Sanshain Service — Implementation Plan

## Completed

The following major milestones have been delivered and are fully functional:

- **Core API**: `POST /provide` (with idempotency, immutability on protected branches, feature-branch override), `GET /require` (with long-polling, feature-branch fallback, dependency tracking), `GET /report` + `/report/markdown`.
- **Architecture**: DDD Hexagonal/Onion (Domain → Application → Infrastructure → Presentation). SQLite & PostgreSQL adapters with auto-migrations. Refactored into a clear lib-bin split for better testability and library reuse.
- **OpenAPI Splitting & Bundling**: Optimized per-endpoint YAML snippets with transitive schema filtering; `POST /require-bundle` for merged multi-endpoint requirements.
- **Backward Compatibility & Versions**: Structural compatibility checking on protected branches; endpoint version history with diff viewer.
- **Authentication**: Multi-mode auth (Dev / Local / LDAP); API tokens; shadow accounts for LDAP users.
- **Web UI & UX**: Askama-templated landing page; htmx-powered admin dashboard; tabbed interface; dark/light mode; custom dagre-based dependency graph (MVP).
- **Security & Reliability**: CSRF protection (with API bypass); LDAP injection mitigation; transactional spec updates; SQLite WAL mode; cleanup jobs for stale branches/dependencies; resolved all `cargo audit` vulnerabilities (ring, rand, argon2, rustls-webpki, time).
- **Observability**: In-memory log buffer; real-time system stats (sysinfo); request/failure counters; Prometheus metrics; JSON logging; dynamic debug flags.
- **DevOps**: Multi-stage/multi-platform Docker builds (x86_64, aarch64); GitHub Actions CI/Release (v0.8.1); Health checks. Improved release process with idiomatic binary naming (`sanshain_service`).
- **AI Rules & Maintenance**: Added mandatory Clippy and test verification rules for AI-assisted development in `ai/ai-rules.md`; resolved all remaining Clippy `allow` annotations for better code quality. Introduced stricter error handling rules forbidding `unwrap()` in production code.
- **Production-Ready Error Handling**: Audited the codebase and removed non-test `unwrap()` and `expect()` calls, replacing them with graceful error handling and proper logging. Implemented `Display` for `AppError` and refactored complex trait methods to satisfy clippy requirements.
- **Demos & Scenarios**: Added `demo2.sh`, a complex 30-service scenario featuring a data pipeline (ETL), ML system, and various infrastructure wrappers to demonstrate scalability and complex dependency graph visualization. Added `demo3.sh` which reproduces the publicly documented [Google Cloud "Online Boutique" (Hipster Shop)](https://github.com/GoogleCloudPlatform/microservices-demo) 11-service e-commerce microservices architecture on a dedicated `google` branch (overridable via `DEMO3_BRANCH`), so users can switch branches in the service UI to flip between `main` (demo/demo2) and `google` (demo3) and view a real-world-derived graph side-by-side with the synthetic one.
- **Advanced Dependency Graph**: implemented a custom Dagre-based graph renderer with architectural role-based grouping (Autobahn layout). Nodes are automatically categorized as Client Only, Both, or Service Only and aligned into consistent global lanes across all ranks to eliminate zig-zag patterns and ensure "leftest" alignment. Increased font sizes, dynamic node dimensions, and Bezier-curved edge routing improve readability and aesthetic flow. Added direction toggle (TB/LR), click-to-highlight subgraph, and SVG export/clipboard support. Removed experimental clustering to reduce visual confusion.
- **Enhanced Graph UX**: moved legend to a side panel and added a filtering toolbar for "Circular Dependencies" view and service-specific focus mode with autocomplete.
- **User Discovery Access**: moved read-only discovery endpoints to a less restrictive auth layer, allowing non-admin authorized users to view the service graph and dependencies.
- **Auto-Approve Users**: added a system setting to automatically approve self-registered users, enabling immediate login without manual admin approval.
- **Service Isolation Report**: implemented a table-per-service markdown report listing outbound communication links for compliance and architecture overview.
- **Lenient Path Matching**: implemented path normalization and lenient variable matching to handle variations in OpenAPI specs and client requests; added database indexing for normalized paths to maintain performance.
- **Simplified `sanshain.yaml` Format**: relocated `serviceName` to root and replaced protocol-specific file fields with a generic `file` parameter.
- **Multiple Provides Support**: updated `docs/sanshain-yaml.md` to support a `provides` list in `sanshain.yaml`, allowing a single service to publish multiple API specifications simultaneously.
- **Extended Configuration Example**: updated the main `sanshain.yaml` example in `docs/sanshain-yaml.md` to demonstrate a multi-protocol configuration supporting OpenAPI, AsyncAPI, and gRPC/Proto simultaneously.
- **Unified Client Configuration**: updated `docs/sanshain-yaml.md` with detailed support and examples for AsyncAPI and gRPC/Proto in the `sanshain.yaml` format.
- **Version 0.11.0**: bumped version to 0.11.0 and updated the client configuration documentation with multi-protocol support and examples.
- **Version 0.10.0**: bumped version to 0.10.0 and initialized the new development cycle following the 0.9.0 release.
- **UI Stability & Cat Loader**: implemented a full-screen "Snoozing Cat" loading overlay and improved UI stability by hiding main content until authentication state is resolved, eliminating visual flickering during navigation.
- **Socke Dark-Mode Gimmick**: added a playful rebranding gimmick that swaps "Sanshain" for "Socke" and the sun logo for a "Socke" (sock) image when dark mode is enabled.

## Open

### Deployment
- [ ] Kubernetes manifests (Deployment, Service, Ingress, ConfigMap, Secret).
- [ ] Helm chart.
- [ ] Reverse proxy TLS config + documentation.

### Observability (Advanced)
- [ ] OpenTelemetry tracing.

### Custom Dependency Graph Visualization (Polish)
- [ ] Edge bundling / merge at endpoint entry points.
- [ ] Export as PNG.

### Web Frontend (Advanced)
- [ ] SSE for `/require` long-polling and live updates.
- [ ] WebSocket support (if bidirectional real-time needed).
