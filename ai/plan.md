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
- **Enhanced Graph UX**: moved legend to a side panel and added a filtering toolbar for "Circular Dependencies" view, service-specific focus mode with autocomplete, and **protocol-based filtering** (OpenAPI, AsyncAPI, Proto toggle buttons).
- **Improved Observability & Logging**: implemented detailed logging for specification processing. Providing invalid specs now logs the full erroneous input content at the `WARN` level. Added detailed `DEBUG` logs for unchanged endpoints and `INFO` summaries for all `provide` operations (inserts/updates/deletes).
- **User Discovery Access**: moved read-only discovery endpoints to a less restrictive auth layer, allowing non-admin authorized users to view the service graph and dependencies.
- **Auto-Approve Users**: added a system setting to automatically approve self-registered users, enabling immediate login without manual admin approval.
- **Service Isolation Report**: implemented a table-per-service markdown report listing outbound communication links for compliance and architecture overview.
- **Lenient Path Matching**: implemented path normalization and lenient variable matching to handle variations in OpenAPI specs and client requests; added database indexing for normalized paths to maintain performance.
- **Simplified `sanshain.yaml` Format**: relocated `serviceName` to root and replaced protocol-specific file fields with a generic `file` parameter.
- **Multiple Provides Support**: updated `docs/sanshain-yaml.md` to support a `provides` list in `sanshain.yaml`, allowing a single service to publish multiple API specifications simultaneously.
- **Extended Configuration Example**: updated the main `sanshain.yaml` example in `docs/sanshain-yaml.md` to demonstrate a multi-protocol configuration supporting OpenAPI, AsyncAPI, and gRPC/Proto simultaneously.
- **Unified Client Configuration**: updated `docs/sanshain-yaml.md` with detailed support and examples for AsyncAPI and gRPC/Proto in the `sanshain.yaml` format.
- **AsyncAPI v2→v3 Upgrade Guide**: added version compatibility documentation to `docs/sanshain-yaml.md` covering operation mapping differences, channel address vs key restriction, and a migration checklist.
- **Spec-to-YAML Matching Guide**: added a "How Matching Works" section to `docs/sanshain-yaml.md` with detailed examples showing how OpenAPI, AsyncAPI, and Proto spec entries map to `sanshain.yaml` `path`/`method` fields, plus a quick-reference table.
 - **Version 0.11.1**: bumped version to 0.11.1.
- **Version 0.10.0**: bumped version to 0.10.0 and initialized the new development cycle following the 0.9.0 release.
- **UI Stability & Cat Loader**: implemented a full-screen "Snoozing Cat" loading overlay and improved UI stability by hiding main content until authentication state is resolved, eliminating visual flickering during navigation.
- **SOKA Dark-Mode Gimmick**: added a playful rebranding gimmick that swaps "Sanshain" for "SOKA" (Japanese for "I see") and the sun logo for a "SOKA" image when dark mode is enabled.
- **Japanese Branding**: added Japanese characters for "Sanshain" (サンシャイン) and "Soka" (そうか) to the root banner, with automatic switching between the two versions based on the active theme.
- **Unified Log View**: removed separation between important and standard logs in the observability dashboard; all logs are now sorted chronologically (oldest on top).
- **Graph Edge Visualization**: implemented asynchronous arrow start and end points (66% outbound, 33% inbound) to reduce overlap and improve readability.
- **Docker Release Build Fix**: fixed the release Docker build by ensuring database migrations (kept in `src/infrastructure/migrations/` per DDD architecture) are copied into the Docker build context during CI.
- **Performance Benchmarks & Optimization**: expanded Criterion benchmarks to cover `split_asyncapi`, `split_proto`, `normalize_path`, `generate_diff`, and `check_backward_compatibility`. Replaced per-call regex compilation with `LazyLock` statics in `normalize_path` and `split_proto`, yielding ~50% improvement in `split_openapi` throughput.
- **In-Memory Spec Cache**: added `CachedSpecRepository` using `moka` crate with memory-bounded TinyLFU/LRU eviction. Caches all hot read paths (service/branch IDs, endpoints, reports, protected branches, fallback branches, service/client lists) with write-through invalidation. Configurable via `CACHE_MEMORY_MB` env var (default 256 MB) and admin UI. Includes 10 unit tests and JSON stats endpoint.
- **Graph Focus Tag Cloud**: replaced single-service focus with multi-service tag cloud. Viewport-sized canvas with fit-to-view scaling. Toolbar reorganized with all toggles on the right.

- **Security: Removed pre-created admin token**: removed the long-lived session token from `ensure_initial_admin`. Admins must now log in via `/login` to obtain a session token. `INITIAL_ADMIN_TOKEN` env var removed.
- **Markdown Report Viewer**: added `report-viewer.html` that renders markdown reports as styled HTML using `marked.js`, with copy-to-clipboard and download buttons. All report links now route through the viewer. Unified report button colors to indigo-600.

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
