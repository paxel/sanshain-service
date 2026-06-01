# Sanshain Service — Implementation Plan

## Completed

The following major milestones have been delivered and are fully functional:

- **Version 1.2.0 (Current)**: Added a favicon to the web interface using the service logo. Bumped minor version across all project files (`Cargo.toml`, `api.yaml`, `README.md`, etc.) and moved previous version 1.1.0 to `OLDER_CHANGES.md`. Created `skill_creator` to enforce AI skill conformity, added `verify_release` for full-stack quality checks, and moved skills to `.junie/skills/` for CLI visibility. Fixed demo scripts for data filling. Fixed circular dependency line style in graph. Resolved 6-hour UI test hang by adding `playwright.config.js` with global timeouts and fixing a dangerous `dialog` listener. Hardened CI robustness by purging `needrestart` to prevent interactive hangs during Playwright installation and adding verbose diagnostics.
  - **Reset History Feature**: Added a new admin feature to reset version history for a branch. This prunes all old versions of endpoints, renumbers the latest version to 1, and resets the branch-level version counter, while preserving existing endpoints and client dependencies.
  - **Admin API**: New endpoint `POST /admin/services/{name}/branches/{branch}/reset-history` to trigger the history reset.
  - **OpenAPI Splitting Determinism**: Ensured that OpenAPI splitting is bit-for-bit deterministic by using ordered collections and explicit sorting of components, preventing false-positive change detection.
  - **Security Hardening**: Removed the shipped CSRF test backdoor; hashed session tokens at rest (SHA-256); required a `Bearer ` prefix for the CSRF API exemption; added loud startup `SECURITY WARNING`s for `dev_mode` and `0.0.0.0` binds; and removed the `auth_login` all-users over-fetch by returning the user from `login`. Added the `secure-csrf` skill.
  - Fix authentication in demo scripts. ✓
  - Fix `require-bundle` in `demo.sh`. ✓
  - Standardize `BASE_URL` and parameters. ✓
  - Fix circular dependency lines (remove unintentional dashing). ✓
  - Fix UI test hang (global timeouts + dialog dismissal). ✓
  - Repository cleanup (remove temp files, update .gitignore). ✓
  - Harden CI stability (purged `needrestart` + verbose install diagnostics + service cleanup). ✓

- **Version 1.1.0 (Released)**: Promoted service to `1.1.0` to reflect accumulated improvements since `1.0.1`. Added graph fallback for feature branches (ghost nodes), public protected branches endpoint, and auto-skip for new services. Introduced `force` mode for `provide` endpoints and a shared contract diff viewer.

- **Version 1.0.1 (Released)**: Fixed a critical production bug where independent services with identical endpoint paths incorrectly shared a "shared contract." Scoped contract tracking to individual service branches (`branch_id`) and verified independence with new automated integration tests.

- **Version 1.0.0 (Released)**: Promoted the service to its first major stable release. This version consolidates all previous improvements and introduces a major breaking change: squashed database migrations for consistent PostgreSQL/SQLite deployments. Direct upgrades from `0.13.x` require a fresh database or manual schema migration.

- **Landing Page Redesign (v0.13.1)**: Replaced old link cards with a beautiful three-step visual graphic (Provide -> Manage -> Require) and added a "Unified Contract Management" section highlighting support for OpenAPI, AsyncAPI, and Protobuf. Updated top banner to link the username to the account page. Fixed a bug where navigating directly to Reports or Graph views via URL hash would result in empty branch selectors; all discovery views are now fully autonomous. Registered missing admin API routes causing 404s in service discovery.

- **Service Discovery Page Split (v0.13.1)**: Split the monolithic `service.html` (1693 lines) into four standalone pages — `services.html`, `clients.html`, `graph.html`, `reports.html`. Shared utilities extracted into `js/discovery.js`. Cross-page navigation uses URL redirects with query params for deep-linking. Old `/service.html` URLs redirect to the correct new page for backward compatibility. Banner nav links updated across all pages.

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
- **Service Isolation Report**: redesigned from bullet-point lists to well-formatted markdown tables with Target, Protocol, and Port columns. AsyncAPI connections route through the central message service (KAFKA). All protocols display as `https`.
- **Lenient Path Matching**: implemented path normalization and lenient variable matching to handle variations in OpenAPI specs and client requests; added database indexing for normalized paths to maintain performance.
- **Simplified `sanshain.yaml` Format**: relocated `serviceName` to root and replaced protocol-specific file fields with a generic `file` parameter.
- **Multiple Provides Support**: updated `docs/sanshain-yaml.md` to support a `provides` list in `sanshain.yaml`, allowing a single service to publish multiple API specifications simultaneously.
- **Extended Configuration Example**: updated the main `sanshain.yaml` example in `docs/sanshain-yaml.md` to demonstrate a multi-protocol configuration supporting OpenAPI, AsyncAPI, and gRPC/Proto simultaneously.
- **Unified Client Configuration**: updated `docs/sanshain-yaml.md` with detailed support and examples for AsyncAPI and gRPC/Proto in the `sanshain.yaml` format.
- **AsyncAPI v2→v3 Upgrade Guide**: added version compatibility documentation to `docs/sanshain-yaml.md` covering operation mapping differences, channel address vs key restriction, and a migration checklist.
- **Spec-to-YAML Matching Guide**: added a "How Matching Works" section to `docs/sanshain-yaml.md` with detailed examples showing how OpenAPI, AsyncAPI, and Proto spec entries map to `sanshain.yaml` `path`/`method` fields, plus a quick-reference table.
- **Service Tags**: added service tagging system with auto-detection from API type (`asyncapi` → `messaging`, `proto` → `grpc`) and optional manual tags via provide requests. Tags are stored in a dedicated `service_tags` table and included in the dependency report.
- **Simplified Dependency Graph Visualization**: simplified the graph by replacing all specialized node shapes (hexagon, diamond, cylinder, octagon) with standard rectangles for a cleaner look. Specialized service types and states are now indicated by clear symbolic icons and emojis: the `🛢️` (oil drum) emoji in the top-right for Kafka (AsyncAPI/messaging), `⛓️` (chains) in the top-right for gRPC/Proto, and `!` in the top-left for missing services. Removed "database" and "infrastructure" hypothetical markers from the graph and legend to focus on actual communication protocols. Renamed the virtual "MESSAGING" node to "KAFKA" and enhanced "register" lines with blue color for better visibility.
 - **Version 0.11.1**: bumped version to 0.11.1.
- **Version 0.10.0**: bumped version to 0.10.0 and initialized the new development cycle following the 0.9.0 release.
- **UI Stability & Cat Loader**: implemented a full-screen "Snoozing Cat" loading overlay with full light/dark mode support and improved UI stability by hiding main content until authentication state is resolved, eliminating visual flickering during navigation.
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

- **Quality Hardening**: Added `.github/workflows/quality.yml` CI quality gate (Clippy `-D warnings`, `cargo fmt --check`, `cargo test`, `cargo audit`, `cargo tarpaulin`, ESLint, Prettier). Set up JS linting infrastructure (`package.json`, `eslint.config.js`, `.prettierrc`). Replaced all production `.unwrap()` calls with proper error handling in `pages.rs`, `fragments.rs`, and `spec_service.rs`. Extracted `parse_spec_endpoints` from `provide_spec_inner` to reduce complexity. Added 24 new unit tests across `auth_service.rs`, `admin_service.rs`, and `spec_service.rs` (total: 59 lib tests).

## Open

- [x] Remove the unused `DashboardTemplate.is_admin` field warning after the unified banner cleanup.
- [x] Restore direct banner navigation to the discovery subviews (services, clients, graph, reports) after the banner unification accidentally hid those entry points.
- [x] Update the changelog to describe the user-visible banner/navigation/license fixes and the major DDD refactoring for v0.13.1.
- [x] Restore the broken observability raw metrics link by exposing the Prometheus endpoint at `/metrics` again and cover it with an integration regression test.
- [x] Fix the landing/info banner and shared nav highlighting, restore admin password changes, and expose the auto-acknowledge new users admin setting in the UI with regression coverage.
- [x] Fix the root password-change flow so changing the admin password rotates the session token cleanly, keeps the UI logged in, and still allows re-login with the new password.
- [x] Bump the service version to 0.13.2 for the SQLite migration fix release.
- [x] Fix the SQLite `20240316000000_api_type` migration ordering so existing databases no longer fail startup migrations with a foreign-key constraint error.
- [x] Harden the PostgreSQL `20240316000000_api_type` migration for production by dropping legacy unique constraints by constrained columns instead of generated names.
- [x] Add migration regression coverage for NULL-endpoint dependency deduplication and PostgreSQL api_type constraint handling.
- [x] Refactor brittle Proto service parsing into boundary-safe helpers and add edge-case coverage for non-ASCII headers and malformed service blocks.
- [x] Consolidate duplicated integration-test application setup into a shared helper and extend `ai/ai-rules.md` with KISS cleanup, dead-code/unused-file removal, coverage targets, and required security checks.
- [x] Fix landing page UI breakage by updating CSP to allow required CDNs (Tailwind, jsDelivr) and adding image dimension fallbacks.
- [x] Apply UI fixes (image dimension fallbacks) to all discovery pages: Services, Clients, Graph, Reports, Observability, Admin, and Account.
- [x] Harden the service against `cargo geiger` findings by migrating from deprecated `serde_yaml` and untrusted `serde_yml` to `serde_yaml_ng`, a maintained fork.
- [x] Verified zero `unsafe` usage in the project's own source code.
- [x] Implemented standard HTTP security headers (CSP, HSTS, XFO, etc.) across all routes to improve the security posture.
- [x] Fixed admin settings persistence (dev-mode, local-users, auto-approve) by returning JSON objects instead of raw booleans.
- [x] Expanded `api.yaml` and implemented `tests/interface_test.rs` to enforce backend/frontend contract consistency.
- [x] Introduced a Playwright-based UI smoke test suite in `tests/ui/` for automated rendering and flow validation.
- [x] Hardened `.gitignore` and provided advice on git maintenance.
- [x] Implemented `scripts/itest.sh`, a comprehensive integration test suite with automated assertions, including shared contract rollback verification.
- [x] Implemented automation and demo scripts for `SanshainMaven` supporting OpenAPI, AsyncAPI, and Proto.
- [x] Verified caching mechanism (both service-side and Maven client-side) and enhanced cache statistics reporting and ETag support.
- [x] Refactored `nuke_database` into a "factory reset" and fixed `FOREIGN KEY` issues in branch/service deletion.
- [x] Evaluated and proposed an OpenAPI-driven code generation strategy.
- [x] Verified security headers with integration tests and ensured `cargo audit` is clean.
- [x] Added modern automation (Cargo Aliases, Justfile, NPM) and integrated `fmt --check` into `itest.sh`.
- [x] Integrated `itest.sh` and Playwright UI tests into GitHub Actions and SourceHut CI.
- [x] Hardened `.gitignore` with `test-results/`.
- [x] Optimized CI workflows for disk space and build speed (rust-cache, taiki-e/install-action).
- [x] Hardened CI service startup logic to prevent Playwright connection timeouts.
- [x] Fixed invalid element IDs (#admin-dashboard) and added reload banner dismissal in Playwright smoke tests.
- [x] Fixed broken task runners and cargo aliases in CI.
- [x] **PostgreSQL Stabilization & Migration Squashing (v1.0.0)**: Consolidated migrations into a single initial schema, fixed `name[] = text[]` operator errors, implemented `ON DELETE CASCADE` for data integrity, and added `testcontainers` for real PostgreSQL verification.

### Deployment
- [ ] Kubernetes manifests (Deployment, Service, Ingress, ConfigMap, Secret).
- [ ] Helm chart.
- [ ] Reverse proxy TLS config + documentation.

### Observability (Advanced)
- [ ] OpenTelemetry tracing.

### Custom Dependency Graph Visualization (Polish)
- [x] Graph Fallback for Feature Branches (merged report API, ghost nodes, conflict detection, target branch dropdown).
- [ ] Edge bundling / merge at endpoint entry points.
- [ ] Export as PNG.

### Web Frontend (Advanced)
- [ ] SSE for `/require` long-polling and live updates.
- [ ] WebSocket support (if bidirectional real-time needed).

### AsyncAPI Semantics & Sync Concurrency
Design exploration document: [`ai/sync.plan`](sync.plan.md)
- [x] Problem 1: `/provide/asyncapi` should only store PUB operations (SUB belongs in `requires`).
- [x] Problem 2: Multiple publishers for the same topic — detect and warn about conflicts.
  - [x] Display the "current" with diff to "source"
- [x] Problem 3: Concurrent developers on same service/branch overwrite each other (optimistic concurrency with `base_version`).
- [x] Problem 4: Provide returns JSON body with version and content hash.
- [x] Problem 5: Skip specification processing if the content hash matches the current version.
- [x] Require-Side Caching: implemented ETag and If-None-Match support for all require-endpoints to reduce network traffic and build times.
- [x] Onboarding Improvements: auto-skip shared contract checks for new services (no protected-branch endpoints) and `force` parameter to reset shared contract source on feature branches.
- [x] Problem 6: Bundle Hash Stability (ensure stable ETag regardless of request order).
- [ ] Problem 7: Semantic Versioning for Specs.

### Going big
- [ ] Have a top-layer system switch. so that the service can be used completely separated from different systems of the customer.
- [ ] User roles and user groups for allowing users/groups to contribute to dedicated systems only
- [ ] Different maintenance roles to allow some users to do project administration / user administration etc.
- [ ] manual editing: 
  - [ ] mark endpoints to be used by external services
  - [ ] add external services
  - [ ] define yamls of external services
- [ ] eye candy: 
  - [ ] select icons for services
  - [ ] group services to clusters
