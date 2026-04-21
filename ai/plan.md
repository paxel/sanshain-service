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
- **AI Rules & Maintenance**: Added mandatory Clippy and test verification rules for AI-assisted development in `ai/ai-rules.md`; resolved all remaining Clippy `allow` annotations for better code quality.
- **Demos & Scenarios**: Added `demo2.sh`, a complex 30-service scenario featuring a data pipeline (ETL), ML system, and various infrastructure wrappers to demonstrate scalability and complex dependency graph visualization. Added `demo3.sh` which reproduces the publicly documented [Google Cloud "Online Boutique" (Hipster Shop)](https://github.com/GoogleCloudPlatform/microservices-demo) 11-service e-commerce microservices architecture on a dedicated `google` branch (overridable via `DEMO3_BRANCH`), so users can switch branches in the service UI to flip between `main` (demo/demo2) and `google` (demo3) and view a real-world-derived graph side-by-side with the synthetic one.
- **Advanced Dependency Graph**: implemented structural clustering to automatically group related services (e.g., ETL enrichers) into visual "Cluster" boxes based on shared clients. Added an intelligent naming algorithm that filters global noise words to generate concise labels. Upgraded layout to Dagre compound graphs with increased spacing and intra-cluster "bricks in a wall" staggering (vertical + horizontal) for compacting large groups into overlapping rows while maintaining clean lines for standalone nodes. Added a pivot button to flip between vertical (TB) and horizontal (LR) orientation; the whole post-layout pipeline (compaction, cluster bounds, edge anchors, Bezier tangents) is rank-axis parametric. Added text wrapping and dynamic node sizing for better readability; enabled SVG export and copy-to-clipboard. Implemented an "Autobahn" flow layout that groups nodes by role (Client Only, Both, Service Only) within each rank with distinct visual gaps.
- **User Discovery Access**: moved read-only discovery endpoints to a less restrictive auth layer, allowing non-admin authorized users to view the service graph and dependencies.
- **Auto-Approve Users**: added a system setting to automatically approve self-registered users, enabling immediate login without manual admin approval.
- **Service Isolation Report**: implemented a table-per-service markdown report listing outbound communication links for compliance and architecture overview.
- **Lenient Path Matching**: implemented path normalization and lenient variable matching to handle variations in OpenAPI specs and client requests; added database indexing for normalized paths to maintain performance.
- **Version 0.10.0**: bumped version to 0.10.0 and initialized the new development cycle following the 0.9.0 release.

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
