# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.11.0] - Unreleased

### Added
- **Enhanced Graph Filtering**: added quick filter toggle buttons (OpenAPI, AsyncAPI, Proto) to the dependency graph toolbar. Users can now selectively hide or show service dependencies based on their protocol.
- **Improved Observability & Logging**: added detailed logging for specification processing.
    - Providing invalid YAML now logs the full erroneous input content at the `WARN` level to facilitate debugging of client-side issues.
    - Detailed `DEBUG` logs now report why specific endpoints are considered unchanged during a `provide` operation.
    - Successful `provide` operations now log a concise `INFO` summary of the changes applied (inserts, updates, deletes).
    - Database adapter operations (`apply_spec_changes`) now log the number of changes and individual operations at the `DEBUG` level for both SQLite and PostgreSQL backends.
- **Technical Performance Tuning**: exposed several internal parameters via environment variables for production tuning, including database pool sizes (`MAX_POSTGRES_CONNECTIONS`, `MAX_SQLITE_CONNECTIONS`), SQLite busy timeouts, background cleanup intervals, and in-memory buffer sizes.
- **Performance Optimization**: optimized OpenAPI specification processing with an 87% performance gain. `split_openapi` is now ~8x faster by using direct object model traversal instead of redundant YAML serializations.
- **Bulk Repository Operations**: introduced `find_endpoints_bulk` and `record_dependencies_bulk` to minimize database roundtrips during client requirement bundling.
- **N+1 Frontend Optimization**: refactored the admin dashboard to fetch services and their branches in a single bulk call, eliminating the N+1 request bottleneck for large service counts.
- **High-Performance Database Indexing**: added critical indices for branch, endpoint, and dependency lookups to ensure sub-millisecond query times.
- **Criterion.rs Benchmarking**: integrated `Criterion.rs` for reliable micro-benchmarking of hot paths and documented results in `README.md`.
- **Configurable Instance ID**: added support for the `INSTANCE_ID` environment variable to allow overriding the randomly generated unique ID for a service instance.
- **Custom Prometheus Endpoint**: introduced the `PROMETHEUS_ENDPOINT` environment variable, allowing administrators to customize the metrics scraping path (defaults to `/metrics`).
- **Configurable Static Assets Directory**: added the `STATIC_DIR` environment variable to allow serving web assets from a custom directory.
- **Enhanced Initial Admin Setup**: the initial root user can now be pre-configured via `INITIAL_ADMIN_USERNAME`, `INITIAL_ADMIN_PASSWORD`, and `INITIAL_ADMIN_TOKEN` environment variables. The startup message now also dynamically includes the correct `BIND_ADDRESS`.
- **Configurable Session Duration**: added `LOGIN_SESSION_DURATION_HOURS` to allow customizing how long user sessions remain valid (defaults to 24 hours).
- **Flexible Log Capture Filtering**: introduced `CAPTURE_LOG_FILTER` to allow tuning the level and scope of logs captured in the in-memory observability buffer independently from the main stdout logs.
- **Simplified `sanshain.yaml` Format**: relocated `serviceName` to the root of the configuration file and replaced protocol-specific file fields (`openApiFile`, `protoFile`, etc.) with a single, generic `file` parameter. `apiType` now defaults to `openapi` if omitted.
- **Multiple Provides Support**: updated `docs/sanshain-yaml.md` to support a `provides` list in `sanshain.yaml`, allowing a single service to publish multiple API specifications simultaneously.
- **Extended Configuration Example**: updated the main `sanshain.yaml` example in `docs/sanshain-yaml.md` to demonstrate a multi-protocol configuration supporting OpenAPI, AsyncAPI, and gRPC/Proto simultaneously.
- **Unified Client Configuration**: updated `docs/sanshain-yaml.md` with detailed support and examples for AsyncAPI and gRPC/Proto in the `sanshain.yaml` format.
- **Protocol-Specific Examples**: added comprehensive examples for providing and requiring AsyncAPI channels and gRPC methods, including instructions on how to call the endpoints.

### Fixed
- **Resolved CVE-2023-0071 (rsa/Marvin Attack)**: completely eliminated the `rsa` vulnerability by removing the `sqlx-mysql` dependency. Switched to manual `FromRow` implementation and runtime migrations to allow dropping the `sqlx/macros` feature.
- **Resolved CVE-2026-0104 (rustls-webpki)**: updated `rustls-webpki` to `0.103.13` to fix a reachable panic in certificate revocation list parsing.
- **Improved Code Quality**: ensured 100% `cargo clippy` compliance and zero unsafe code (outside of standard CSP headers).
- **Reduced Binary Size & Dependencies**: dropped the heavy `sqlx-macros` and `sqlx-mysql` dependencies, leading to faster compile times and a smaller security surface area.

## [0.10.0] - 2026-04-22

### Added
- **AI Clippy & Error Handling Rules**: added stricter rules to `ai/ai-rules.md` forbidding `unwrap()` in production code and requiring `cargo clippy` verification for all changes.
- **Improved Error Type**: implemented `Display` and `Error` traits for `AppError` for better error reporting and idiomatic integration with the standard library.
- **gRPC/Proto Support**: added support for `.proto` files. Services can provide their Proto definitions via `POST /provide/grpc`, and clients can require individual methods via `GET /require/grpc`.
- **Protocol Demo Script**: added `demo_protocols.sh` to demonstrate AsyncAPI and gRPC/Proto integration and unified dependency tracking.
- **Protocol-Aware Dependency Tracking**: the service now tracks and displays the protocol (OpenAPI, AsyncAPI, Proto) for all endpoints and dependencies in the UI and reports.
- **Enhanced Require-Bundle**: the `/require-bundle` endpoint now supports an optional `api_type` parameter to bundle AsyncAPI or Proto snippets.
- **Lenient Path Matching**: introduced normalized path matching to handle common differences in OpenAPI path definitions and client requirements. The service now collapses redundant slashes, trims whitespace, and ignores path variable names (e.g., `/api/{id}` matches `/api/{userId}`) by using a universal placeholder during lookup.
- **Normalized Path Database Indexing**: added a `normalized_path` column to the `endpoints` and `dependencies` tables with a database index to ensure O(log N) lookup performance for lenient matching.
- **Auto-Approve Users**: added an "Auto-Approve Users" toggle in the admin dashboard (User Management tab). When enabled, self-registered users are automatically approved and can log in immediately without manual admin intervention.
- **Authorized User Discovery Access**: non-admin authorized users (any logged-in user) can now access service discovery data and view the dependency graph. Read-only discovery endpoints were moved to a less restrictive authentication layer while preserving admin-only access for destructive operations.
- **Improved Graph Node Readability**: significantly increased the base font size (13px → 16px) and node dimensions in the custom dependency graph to improve readability. Text wrapping now uses a larger line height and wider estimated character widths for better centering within the adapted node boxes.
- **Graph SVG Download**: enabled the download and copy-to-clipboard buttons for the custom dependency graph. Users can now export the interactive graph as a high-quality SVG file or copy the SVG markup directly.
- **Autobahn Flow Layout**: reorganized nodes within each graph rank to follow a logical "client-to-server" flow. Nodes are aligned into global lanes based on their architectural roles (Client Only, Both, Service Only), ensuring that the "leftest" nodes of the same role are consistently aligned across all ranks, eliminating zig-zag patterns. Increased the gap between role groups to further improve visual separation.
- **System Reports Section**: introduced a dedicated "Reports" section in the service discovery UI. This centralizes access to system-wide reports like the Service Isolation Report and Markdown Report, organized by branch. Redundant links were removed from service and graph views to improve navigation clarity.
- **Service Isolation Report**: added a new markdown report that lists all outbound service-to-service communication in a table-per-service format. The report identifies which target services each service "talks" to, including placeholder columns for port and protocol as required by compliance auditors. Available via `GET /report/isolation` and through the new Reports section.
- **Snoozing Cat Loader**: added a playful, cat-themed loading animation that appears during long-running tasks like graph generation or service list fetching.
- **Improved UI Stability**: eliminated "login flicker" by hiding main content by default and using a full-screen loading overlay until authentication state is confirmed.
- **Advanced Dependency Graph UI**: improved the dependency graph interface by moving the legend to a sticky side panel and introducing a new toolbar row for advanced filtering.
- **Graph Redraw & Filtering**: added "Show All" and "Circular Dependencies" redraw options.
- **Graph Focus Mode**: added a service-specific focus mode with autocomplete support that filters the graph to show only a selected service and its direct neighbors.
- **Optimized Graph Toolbar**: consolidated graph controls (orientation toggle, copy, download) into a unified toolbar for better ergonomics.
- **Socke Dark-Mode Gimmick**: introduced a "Socke" theme gimmick that automatically rebrands the service from "Sanshain" to "Socke" and swaps the sun logo for a "Socke" (sock) image when dark mode is enabled.

### Fixed
- **Startup Robustness**: replaced several `expect()` calls in `src/main.rs` with graceful termination, proper logging, and improved signal handling (Ctrl+C and SIGTERM).
- **Graceful LDAP Error**: replaced a risky `expect()` in the LDAP provider with proper error propagation.
- **Clippy Compliance**: resolved several clippy warnings including `needless_borrows_for_generic_args`, `trim_split_whitespace`, and `derivable_impls`.
- **Refactored Too-Many-Arguments**: replaced the 8-argument `record_dependency` method in the `SpecRepository` trait with a parameter object (`RecordDependencyParams`) to comply with clippy's complexity limits.
- **Startup Panic on Port In Use**: replaced `unwrap()` on `TcpListener::bind` with graceful error handling and a helpful hint when the port is already in use.
- **Graph Layout Crash**: fixed a "nodeW is not defined" reference error in the dependency graph renderer caused by stale fixed-size layout remnants.

### Removed
- **Dependency Graph Clustering**: removed the structural clustering and brick-staggering layout. While intended to reduce noise, the automatic grouping was found to be confusing and unpredictable in many architectural scenarios. The graph now uses a cleaner, simplified layout while preserving the Autobahn role-based grouping.

## [0.9.0] - 2026-04-20

### Added
- **Demo 3 Scenario — Google Cloud Online Boutique**: added `demo3.sh`, a CLI-registration script that provisions the 11-service "Online Boutique" (a.k.a. Hipster Shop) e-commerce microservices architecture published by Google at [GoogleCloudPlatform/microservices-demo](https://github.com/GoogleCloudPlatform/microservices-demo). All services are registered on a dedicated `google` branch (overridable via `DEMO3_BRANCH`), so the scenario can be viewed side-by-side with `demo.sh`/`demo2.sh` (`main` branch) by switching the branch selector in the service UI — the dependency graph reloads automatically with the selected branch's topology. The public source is cited in the script header.
- **Brand Logo (Sonne)**: integrated the `sonne.png` brand image across the UI and documentation. The image was moved from `templates/` (where it was not being served) to `static/images/sonne.png` so it is exposed via the `ServeDir` at `/images/sonne.png`. The logo now appears in the shared layout footer, the landing page nav + hero, the admin / account / service page navs, the admin and account login screens, and at the top of `README.md`.
- **Dependency Graph Clustering**: implemented structural clustering for the custom dependency graph. Services sharing the same set of clients are now automatically grouped into dashed "Cluster" boxes to reduce visual noise in large-scale scenarios like `demo2.sh`.
- **Intelligent Cluster Naming**: added an algorithm that identifies global "noise" words (common prefixes/suffixes) across all services and filters them out to generate concise, meaningful labels for clusters (e.g., "ETL" or "ML"). Labels are omitted if no distinctive common words are found.
- **Improved Graph Spacing & Compaction**: updated the Dagre layout with significantly increased rank spacing. Implemented a "bricks in a wall" vertical and horizontal staggering effect to "compact" large clusters into overlapping multiple rows, drastically reducing horizontal space while keeping standalone services on a clean, single line per rank.
- **Structured Per-Rank Repacking**: added a math-based post-layout step that treats each rank as a 1D packing problem. Standalone nodes and clusters (as single blocks with their compacted width) are re-packed left-to-right with a uniform gap around each rank's original midpoint, reclaiming the horizontal space freed by cluster compaction and eliminating visual misalignment between clustered and non-clustered rows.
- **Elegant Curved Edge Routing**: replaced the stale Dagre polyline waypoints (which were routing around pre-compaction node positions and creating chaotic detours) with freshly-computed cubic Bezier curves. Top-to-bottom edges now use vertical tangents proportional to rank distance for a smooth "river" flow, while same-rank/reverse edges use a lateral S-curve bow so arrows never cut through nodes.
- **Graph Direction Pivot**: added a toggle button next to the graph mode selector to flip the dependency graph between vertical (top→bottom) and horizontal (left→right) orientation. The whole compaction, brick-staggering, cluster-bounds and Bezier-tangent pipeline is now rank-axis parametric, so both orientations render with the same quality.
- **Dagre Compound Graph Support**: migrated the custom graph renderer to use Dagre's compound graph layout for stable cluster positioning.

### Fixed
- **Horizontal Graph Node Overlap**: in horizontal (LR) mode, nodes on the same rank were overlapping along the Y axis because the Dagre `nodesep` and the brick-layout flow-axis gaps were sized for the TB orientation (where the flow axis is X, not Y). Made `nodesep`, `GAP`, `BRICK_GAP` and `BRICK_DR` direction-aware so LR layouts get proper vertical separation between stacked nodes and bricks.
- **Vertical Cluster Y-axis Overlap**: in vertical (TB) mode, clustered nodes staggered into the "brick" second row had only 5px Y clearance because the rank-axis stagger (`BRICK_DR`) was 45px while node height is 40px. Increased `BRICK_DR` in TB to 70px so staggered bricks now have a comfortable 30px vertical gap.
- **Dependency Graph Z-index**: fixed an issue where cluster boxes were overlapping arrows by reordering SVG rendering. Clusters are now drawn in the background behind edges and nodes.
- **Admin UX - Developer Mode Visibility**: moved the "Developer Mode" (auth bypass) toggle from the "System Config" tab to the "User Management" tab in the admin dashboard for better discoverability and logical grouping. Added a clearer "Auth Bypass" warning badge.
- **Graph Toolbar Visibility**: the Vertical/Horizontal direction toggle is now only shown when the custom "Graph" mode is active (it has no effect on Mermaid/Detailed views), and the Copy/Download buttons (which export the Mermaid code) are now only shown in Mermaid/Detailed modes — eliminating the misleading UI where all three buttons were visible regardless of the active mode.
- **Horizontal Cluster X-axis Overlap**: in horizontal (LR) mode, clusters were still heavily overlapping because the brick-row stagger (`BRICK_DR=90`) was smaller than the 160px node width along the rank axis, so the second brick row sat on top of the first, and the 180px `ranksep` left no room for the stagger to push into the gap between ranks. Raised LR `BRICK_DR` to 200 (nodeW + 40px clearance) and LR `ranksep` to 260 so staggered cluster members no longer overlap each other or the neighbouring rank.
- **Horizontal Cluster Layout — No Brick Weaving**: in horizontal (LR) mode the brick-weaving pattern was causing cluster members to spread along the X axis into the next rank; switched LR clusters to a straight single-column stack along the flow (Y) axis with no rank-axis stagger. Cluster members now sit neatly under each other, making large clusters like the ETL pipeline much easier to read in horizontal view. TB (vertical) mode still uses the 2-row brick pattern to keep horizontal footprint compact. LR `ranksep` relaxed from 260 → 200 accordingly.
- **Horizontal Layer Gap Tightened**: reduced LR `ranksep` further from 200 → 110 now that brick weaving is gone on the rank (X) axis, bringing rank columns closer together for a denser, more readable horizontal graph while still leaving room for the Bezier edge curvature between layers.

## [0.8.1] - 2026-04-19

### Added
- **Complex Scenario Demo**: added `demo2.sh`, a realistic scenario with 30 services representing a complex data pipeline and ML system with central orchestration and infrastructure wrappers.

### Fixed
- **Release workflow and binary naming**: fixed the GitHub Actions release failure by updating all references from the legacy `sanshain_service_bin` name to the new idiomatic `sanshain_service` name.
- **Project architecture alignment**: ensured Dockerfile and documentation are fully aligned with the binary-library separation refactor.

## [0.8.0] - 2026-04-19

### Changed
- **Admin Dashboard Refactoring**: reorganized the admin page into a tabbed interface (Observability, User Management, System Config, Services & Clients).
- **Dependency Updates**: updated core dependencies to their latest major/minor versions (askama v0.15, similar v3, sysinfo v0.38, sha2 v0.11) and refreshed the lockfile.
- **Security & Safety Verification**: confirmed zero unsafe code usage via `cargo geiger` and performed a security sweep with `cargo audit`. Acknowledged a known vulnerability in `rsa` (via `sqlx-mysql`) with no current upstream fix, noting it does not impact the service's runtime as MySQL is not used.

### Added
- **AI Clippy Rule**: added a mandatory rule for AI agents to run `cargo clippy` and verify tests for every task involving code changes.
- **Security Audit**: performed a comprehensive security audit covering SQL injection, path traversal, access control, and web security headers. Findings documented in `ai/security-audit.md`.
- **Process-aware Uptime**: added process uptime tracking to the observability dashboard to distinguish between service restarts and system-wide uptime.
- **System Observability Dashboard**: added a new "System Observability" section to the admin dashboard with real-time monitoring and debugging tools.
- **In-memory Log Buffer**: implemented a 100-message ring buffer that captures application logs in real-time with level filtering.
- **Live System Statistics**: added real-time tracking of CPU, memory, uptime, and request/failure counters using `sysinfo`.
- **Improved Request Tracking**: focused request and failure counters on business logic endpoints, excluding admin and static traffic.
- **Dynamic Debug Tracing**: introduced toggleable debug flags for "Business Logic" and "Admin/User Activity" to enable detailed tracing at runtime.
- **Custom Tracing Layer**: implemented a custom `tracing-subscriber` layer to intercept logs and populate the in-memory buffer.
- **Request & Failure Counters**: added a global middleware to track total requests and 5xx failures for high-level health monitoring.
- **Transactional specification updates**: `provide` operations now apply all database changes within a single transaction to ensure data and history sync.
- **SQLite reliability tuning**: optimized SQLite with WAL mode, normal synchronous mode, and a 5-second busy timeout for better concurrent performance.
- **Service-specific Fallback Branch**: introduced per-service custom fallback branches prioritized during endpoint resolution.
- **Service Detail Enhancement**: updated the Admin UI to allow configuring fallback branches for each registered service via an inline form.
- **Enhanced Log Viewer**: expanded log viewer height and added a "Copy Logs" button for easier troubleshooting.

### Fixed
- **Security Dependency Updates**: resolved multiple high and medium severity vulnerabilities identified by `cargo audit`.
  - Upgraded `ldap3` to `v0.12.1` to fix vulnerable/unmaintained `ring` `v0.16.20`.
  - Upgraded `rand` to `v0.10.1` to resolve soundness issues in `v0.8`/`v0.9`.
  - Downgraded `argon2` to stable `v0.5.3` to avoid yanked dependencies (`digest`, `password-hash`) in `v0.6.0-rc.8`.
  - Resolved vulnerabilities in `rustls-webpki` and `time` via `cargo update`.
- **Code Quality & Maintenance**: resolved all remaining `clippy::type_complexity` and `clippy::too_many_arguments` issues by introducing parameter objects and database row structs, eliminating the need for `#[allow]` annotations.
- **Improved AuthMode parsing**: implemented `std::str::FromStr` for `AuthMode` for better idiomatic string parsing.
- **Optimized OpenAPI splitting**: pre-calculates a component dependency graph, reducing complexity from O(N*M) to O(N+M) for faster updates.
- **Improved Backward Compatibility checking**: validates compatibility once for the entire specification, eliminating O(N*M) redundancy.
- **LDAP Server URL validation**: added basic validation for LDAP server URLs to ensure valid protocols and host formats, mitigating SSRF risks.
- **Date/Time library migration**: replaced manual date formatting with the `chrono` library throughout the application service layer.
- **CSRF Token Memory Leak & Security**: replaced `HashSet` with a `HashMap` that tracks creation time and prunes tokens older than 24 hours.
- **CSRF Bypass for API Tokens**: requests using `Authorization: Bearer` headers now bypass CSRF protection for easier automation.
- **Event-driven Long-polling**: replaced the busy-wait loop in `/require` with a `tokio::sync` watch/broadcast mechanism for immediate wake-ups.
- **Improved error handling and robustness**: replaced unsafe `.unwrap()` calls in critical paths with proper error handling and descriptive `.expect()`.
- **Prometheus Metrics**: integrated `axum-prometheus` to expose standard HTTP metrics at the `/metrics` endpoint.
- **Structured JSON Logging**: added support for JSON log format via the `LOG_FORMAT=json` environment variable.
- **Kubernetes Deployment**: added a full set of Kubernetes manifests and Kustomization support in `deploy/kubernetes/`.
- **LDAP Injection Vulnerability**: added filter escaping for usernames in the LDAP authentication provider to prevent injection attacks.
- **Admin page config switches missing after login**: fixed a bug where dashboard fragments wouldn't render immediately after login without a refresh.
- **Integration Test Stability**: resolved a race condition and panic in integration tests caused by multiple Prometheus recorder registrations.

## [0.7.0]

### Added
- **Backward compatibility checking for protected branches**: instead of rejecting all YAML changes on protected branches, the service now parses and compares the old and new OpenAPI specs structurally. Backward-compatible changes (adding optional fields, new schemas, new endpoints) are accepted; breaking changes (removing fields, changing types, removing response codes or schemas) are rejected with a descriptive error.
- **Endpoint version history**: every backward-compatible update on a protected branch records a new version with the full YAML content and a unified diff from the previous version. New `GET /endpoint-versions` API endpoint returns the version history for a given endpoint (query params: `servicename`, `branch`, `path`, `method`).
- **`endpoint_versions` database table**: new migration adds version tracking storage for both SQLite and PostgreSQL backends.
- **Version history & diff UI**: when viewing an endpoint with multiple versions, the modal now shows a tabbed interface with "Current YAML" and "Version History" tabs. The history tab lists all versions with expandable YAML view and diff-from-previous buttons, plus a compare tool to diff any two arbitrary versions using client-side LCS diff with color-coded added/removed lines.
- **Dark/light mode toggle**: a 🌙/☀️ button in the navigation bar lets users switch between light and dark themes. Preference is stored in a cookie (`sanshain_theme`) and persists across sessions and pages.
- **Dedicated read-only admin endpoints for the UI**: three new read-only admin endpoints replace the previous pattern of abusing `/require` for YAML previews: `GET /admin/endpoint-yaml` (fetch YAML content), `GET /admin/endpoint-versions` (fetch version history), and `GET /admin/services/{name}/branches/{branch}/endpoints` (list endpoints for a service branch). This eliminates the `_viewer` ghost client issue and enforces proper separation between business APIs and UI data access.
- **Stale UI detection**: each page now checks the server's version and instance ID on load. When the server has been restarted or updated, a yellow banner appears offering a one-click reload, eliminating the need to manually clear browser cache.
- **Custom interactive dependency graph (MVP)**: added a new dagre-based SVG graph view as the default dependency visualization, replacing Mermaid as the primary view. Features topological top-down layout (clients on top, services below), color-coded nodes (client-only, service-only, both), red dashed edges for circular dependencies, hover tooltips showing HTTP method and path on edges, click-to-highlight connected subgraph (dims unrelated nodes), and mouse wheel zoom + drag pan. Mermaid and Detailed views remain available via a view-mode toggle. Copy and Download buttons adapt to the active view (SVG export for custom graph, `.mmd` for Mermaid).

- **Cross-navigation between services and clients**: the "N clients" badge on service endpoints now shows a dropdown listing each client with a click-to-navigate link. The "resolved" badge on client endpoints now links back to the providing service endpoint. This makes it easy to explore the dependency graph directly from the endpoint views.

### Fixed
- **Dependency graph filtering**: fixed over-aggressive JOIN that hid valid endpoints from the dependency report/UI. Now uses LEFT JOIN with a WHERE filter so dependencies are shown unless the target endpoint is explicitly soft-deleted.
- **Diff view readability**: stored diffs now render with colored add/remove spans instead of unstyled black text on dark background.
- **Service endpoints not visible in UI**: the service detail page was fetching endpoints only from the `/report` endpoint (dependency graph + unused), which meant services with no client dependencies (e.g., dry-run-service) showed "No endpoints found". Now fetches the authoritative list from `/admin/services/{name}/branches/{branch}/endpoints` and merges report data for client/usage info.
- **Dry-run mode no longer creates database records**: `provide` and `require` with `dry_run=true` previously called `ensure_service`/`ensure_branch`/`ensure_client` which created service, branch, and client records as a side effect. Now uses read-only `find_service`/`find_branch` lookups so dry-run is truly side-effect-free.
- **Graph zoom/pan not centering on mouse cursor**: zoom via mouse wheel now correctly centers on the cursor position, and panning moves at the expected speed. Previously, the coordinate conversion between screen pixels and SVG viewBox space was missing, causing the graph to drift down-right when zooming.

## [0.6.0]

### Added
- **Dry-run mode for provide and require**: all three endpoints (`/provide`, `/require`, `/require-bundle`) now accept a `dry_run` parameter (boolean, in JSON body or query string). When `true`, the request validates everything (YAML parsing, conflict detection, endpoint lookup) but does not persist any data — no specs are stored, no client dependencies are recorded. This enables CI pipelines to test whether a feature branch would be valid against the main branch before allowing a PR to be merged.
- **YAML viewer copy & download buttons**: the paginated YAML display on the service overview page now includes "Copy" (clipboard) and "Download" (`.yaml` file) buttons for easy content export.
- **Graph view copy & download buttons**: the dependency graph view now includes "Copy" (clipboard) and "Download" (`.mmd` file) buttons to export the generated Mermaid code.

### Changed
- **Descriptive errors for provide conflicts**: `/provide` now returns a descriptive error message in the response body on 409 Conflict, including the method, path, branch, and service name (e.g., "DTO changed for GET /users on protected branch 'main' of service 'my-svc'").
- **Descriptive errors for missing endpoints**: `/require` and `/require-bundle` now return the missing endpoint details (method, path, service name, branch) in the response body instead of a bare 404/400 status. `/require-bundle` with partially missing endpoints now returns 404 (was 400) with a list of all missing endpoints, helping clients identify which endpoints were removed on the service side.

### Fixed
- **Phantom services in service list**: services that were only referenced by client dependencies (via `/require`) but never had specs uploaded no longer appear in the services list with 0 branches.
- **Phantom clients in client list**: clients whose dependencies were removed (e.g., after deleting a service) no longer appear in the clients list with 0 branches.

## [0.5.1]

### Fixed
- **Broken navigation links on landing page**: the index page linked to `/admin`, `/account`, and `/services` which had no matching routes. Added redirect routes for `/account` → `/account.html` and `/services` → `/service.html`, and changed the admin link to point to `/admin.html` (since `/admin` is reserved for the API namespace).
- **Unhelpful error on server unreachable**: login and registration forms showed a raw browser "NetworkError" when the server was not running. Now displays a clear "Server is not reachable" message. Added show/hide password toggle buttons to all password fields.
- **Docker container ignores Ctrl+C**: the container kept running (as "unhealthy") after pressing Ctrl+C because PID 1 did not handle signals. Added `tini` as init process in both Dockerfiles, `init: true` in docker-compose template, and graceful shutdown signal handling (SIGINT/SIGTERM) in the server.
- **Aggressive browser caching of static files**: updated JS/HTML was not picked up by browsers without a hard reload. Added `Cache-Control: no-cache, must-revalidate` header to all static file responses so browsers always revalidate with the server.
- **CSP blocked htmx from unpkg.com**: the Content-Security-Policy `script-src` directive did not include `https://unpkg.com`, causing the browser to block the htmx script on the admin dashboard.

## [0.5.0]

### Added
- **Web frontend modularisation (Phase 1)**: extracted shared Askama layout template (`templates/layout.html`) with nav, footer, and common CSS/JS. Landing page (`index.html`) and dashboard now extend the layout. Created `static/js/common.js` with shared fetch helpers, CSRF token management, HTML escaping, and confirm modal logic. Refactored `admin.html`, `account.html`, and `service.html` to use `common.js` instead of duplicating ~60 lines of JS each.
- **Endpoint pruning on provide**: when a new spec is uploaded via `POST /provide`, endpoints present in the previous spec but missing from the new one are now removed. On protected branches, removed endpoints are soft-deleted (marked `deleted`) so that re-introducing them later is rejected as a contract violation. On feature branches, removed endpoints are hard-deleted and can be freely re-added.
- **Stale dependency pruning**: dependency rows are now timestamped with `last_seen_at` (updated on every `/require` or `/require-bundle` call). A background task (hourly, alongside branch cleanup) deletes dependency rows older than a configurable max-age (default 30 days). Admins can configure via `GET/POST /admin/settings/dependency-max-age` and trigger immediate cleanup via `POST /admin/settings/dependency-cleanup`.
- **htmx-powered admin dashboard (Phase 2)**: replaced ~500 lines of vanilla JavaScript DOM manipulation in `admin.html` with htmx attributes and server-rendered Askama HTML fragments. All dynamic sections (users, services, clients, settings toggles, protected branches, auth config, cleanup controls) are now loaded and updated via `hx-get`/`hx-post`/`hx-delete` attributes that swap HTML fragments from `/fragments/admin/*` routes. Only ~80 lines of JS remain for login/logout flow, LDAP test, auth config save, and password change modal.

### Fixed
- **Duplicate dependencies**: added UNIQUE constraint on the dependencies table and `INSERT OR IGNORE` / `ON CONFLICT DO NOTHING` to prevent duplicate endpoint entries when clients re-register the same dependency.
- **Graph branch selector**: replaced hardcoded `branch=main` in the dependency graph with a dynamic branch dropdown populated from all known service branches, fixing "No dependencies found" when the branch name differs.
- **Node.js 20 deprecation warnings**: set `FORCE_JAVASCRIPT_ACTIONS_TO_NODE24=true` in the release workflow to silence GitHub Actions Node.js 20 deprecation annotations.

## [0.4.4]

### Fixed
- **CI build cache fix**: removed `target/` directory from the cargo cache to prevent stale proc-macro artifacts (`zerofrom_derive`) from causing cross-compilation failures.

## [0.4.3]

### Fixed
- **aarch64 cross-compilation CI fix**: replaced manual musl cross-compiler download (musl.cc is unreliable) with `cross-rs/cross`, the standard Rust cross-compilation tool that handles all toolchains via Docker containers.

## [0.4.2]

### Changed
- **Release workflow tag trigger**: the GitHub Actions release workflow now triggers automatically on `vX.X.X` version tags in addition to manual `workflow_dispatch`.

## [0.4.1] - 2026-04-01

### Fixed
- **Cross-compilation fix**: aarch64-unknown-linux-musl builds now use the correct musl cross-compiler toolchain instead of glibc, fixing linker errors with undefined symbols (`open64`, `stat64`, etc.).

## [0.4.0] - 2026-04-01

### Added
- **Branch max-age auto-cleanup**: non-protected branches are automatically deleted after a configurable period of inactivity (default: 30 days). A background task runs hourly. Admins can configure the max-age via `GET/POST /admin/settings/branch-max-age` and trigger immediate cleanup via `POST /admin/settings/branch-cleanup`.

### Fixed
- **Gzip request decompression**: the server now decompresses gzip-encoded request bodies (`Content-Encoding: gzip`), fixing failures when clients (e.g., Maven plugin) upload specs with compression enabled.

### Changed
- **Multi-architecture release builds**: GitHub Release now publishes Linux binaries for both `x86_64` and `aarch64`. Docker images are built as multi-platform manifests (`linux/amd64`, `linux/arm64`) and pushed to GHCR. Windows and macOS are not supported.
- **Release binary naming**: binaries use standard architecture names (`sanshain-linux-x86_64`, `sanshain-linux-aarch64`) instead of Docker-style `amd64`/`arm64`.

## [0.3.0]

### Added
- **`POST /require-bundle` endpoint**: request multiple endpoints from a single service in one call and receive a single merged OpenAPI YAML with deduplicated schemas/components. Solves the problem of duplicate DTOs when clients (e.g., Java/Maven) require multiple endpoints that share the same models.
- **`sanshain.yaml` client configuration format**: documented standard config file format for client plugins (Maven, Gradle, Cargo, etc.) covering provide, require, and require-bundle settings. See `docs/sanshain-yaml.md`.
- **Server-side gzip compression**: responses are automatically gzip-compressed when clients send `Accept-Encoding: gzip`. Enabled via `tower-http` `CompressionLayer`. The `compression: true` option in `sanshain.yaml` is now functional.


## [0.2.0] - 2026-03-26

### Changed
- **OpenAPI splitting now includes only referenced schemas**: each endpoint snippet contains only the `components/schemas` (and other component types) that are actually referenced by that operation, resolved transitively. Previously all schemas were included in every snippet.

### Added
- **LDAP authentication support**: new Auth Mode selector (Dev / Local / LDAP) on the admin dashboard.
- `AuthMode` enum and `LdapConfig` model in the domain layer; `AuthProvider` port trait for pluggable authentication.
- `LdapAuthProvider` adapter (ldap3 crate) with bind-and-search authentication and group-based admin detection.
- `LocalAuthProvider` adapter wrapping existing Argon2 password verification.
- Auth-mode and LDAP config CRUD services in the application layer (`get_auth_mode`, `set_auth_mode`, `get_ldap_config`, `set_ldap_config`, `test_ldap_connection`, `login_with_provider`).
- `GET /api/admin/auth-config`, `PUT /api/admin/auth-config`, and `POST /api/admin/auth-config/test` admin API endpoints.
- Login handler dispatches to the active auth provider; LDAP users are auto-provisioned as shadow accounts.
- Admin UI: Authentication section with radio buttons, LDAP configuration form, and "Test Connection" button.
- Bind password is redacted (`****`) in API responses; re-submitting `****` preserves the stored password.
- Unit tests for auth mode, LDAP config validation, provider dispatch (mock), and connection testing.
- Integration test for auth-config API endpoints (CRUD, validation, auth requirement).
- PostgreSQL database backend as an alternative to SQLite, selectable via `DATABASE_URL` environment variable.
- `PostgresSpecRepository` adapter with full feature parity to the SQLite adapter.
- `DatabaseRepo` enum-based dispatch layer for runtime database backend selection.
- PostgreSQL migration script (`src/infrastructure/migrations/postgres/`).
- Admin dashboard "Database Configuration" section showing current backend and (masked) connection URL.
- `GET /admin/settings/database` endpoint returning current database backend info.
- Documentation: PostgreSQL setup guide in `docs/getting-started.md` (Docker, Docker Compose, bare metal) and Database Configuration section in `docs/administration.md`.

## [0.1.1] - 2026-03-14

### Added
- `ai/ai-rules.md` with Rust and service development best practices for AI-assisted development.
- `docs/` Detailed User doc started.
- `docs/user-guide.md` — Day-to-day usage guide covering concepts, web UI navigation, dependency reports, and build tool plugin overview (links to individual tools will be added as they become available).

### Changed
- `demo.sh` now registers multiple services and clients (with feature branches) to populate a realistic dependency graph for UI testing; verbose YAML output replaced with a concise line-count summary.
- `ai/plan.md` consolidated from ~200 lines to ~50 lines; completed items grouped into a summary, only open tasks listed individually.

### Fixed
- Release workflow now builds with `x86_64-unknown-linux-musl` target so the binary runs correctly on Alpine-based Docker images (fixes "no such file or directory" error).

## [0.1.0] - 2025-03-13

### Added
- Initial release of Sanshain Service.
- `POST /provide` endpoint for uploading OpenAPI specifications.
- `GET /require` endpoint for retrieving per-endpoint YAML snippets with dependency tracking.
- `GET /report` and `GET /report/markdown` endpoints for dependency reports.
- OpenAPI splitting logic using the `openapiv3` crate.
- Immutable endpoint path policy with idempotency support.
- Feature branch fallback to `master` on `/require`.
- Long-polling support with configurable timeout on `/require`.
- Web Dashboard for browsing services, branches, and dependencies.
- SQLite database with automatic migrations.
- Integration tests for provide, require, and report flows.
- DDD Hexagonal/Onion Architecture with Domain, Application, and Infrastructure layers.
- `SpecRepository` trait (port) for persistence abstraction.
- `SqliteSpecRepository` adapter implementing the repository trait.
- Application service layer with use-case functions (`provide_spec`, `require_endpoint`, `generate_report`, `render_report_markdown`).
- Thin Axum handlers delegating to application services.
- Unit tests for OpenAPI splitting logic, application services (with mock repository), and markdown report rendering.
- Protected branch configuration: `main` and `master` are protected by default (immutable endpoints).
- Feature branch exception: on non-protected branches, endpoint DTOs can be updated freely.
- Admin API for managing protected branches (`GET/POST /admin/protected-branches`, `DELETE /admin/protected-branches/:pattern`).
- Admin API for data management: list and delete services (`GET/DELETE /admin/services/:name`), branches (`GET/DELETE /admin/services/:name/branches/:branch`), and clients (`GET/DELETE /admin/clients/:name`).
- AGPL-3.0 License.
- `GET /health` endpoint for liveness/readiness probes.
- Multi-stage `Dockerfile` (build in Rust Alpine, run in Alpine).
- `Dockerfile.release.template` for building Docker images from pre-compiled binaries during release.
- `docker-compose.release.template.yaml` for running the published Docker image with a database volume.
- `.dockerignore` to optimize Docker build context.
- SourceHut CI pipeline (`.build.yml`): build and test on Alpine Linux.
- Per-adapter migration structure (`src/infrastructure/migrations/sqlite/`) with `run_migrations()` on `SqliteSpecRepository`.
- Session-based authentication with Argon2 password hashing and 256-bit random session tokens.
- Automatic root admin user creation on first start (password printed to stderr).
- Auth endpoints: `POST /auth/login`, `POST /auth/logout`, `GET /auth/me`, `POST /auth/change-password`.
- Dev mode toggle (`POST /admin/settings/dev-mode`): when enabled, non-admin API endpoints are open without authentication; when disabled (default), all API endpoints require a valid session.
- Admin endpoints always require a valid admin session token.
- Configurable bind address via `BIND_ADDRESS` environment variable (default `0.0.0.0:3000`).
- Initial admin setup message now includes a URL (`http://localhost:3000/admin.html`) for changing the password.
- Upgraded all dependencies to latest releases: axum 0.7→0.8, sqlx 0.7→0.8, tower 0.4→0.5, tower-http 0.5→0.6, rand 0.8→0.9, openapiv3 2.0→2.2, argon2 0.5→0.6.0-rc.7.
- Admin dashboard now fetches and sends CSRF tokens for all state-changing requests (POST/DELETE).
- `api.yaml` — Hand-written OpenAPI 3.0.3 specification for the client-facing `/provide` and `/require` endpoints, intended as the contract for all client plugins (Maven, Gradle, Cargo, Go, npm, etc.).

#### API Token Management
- API token management for programmatic/CI access: `POST /auth/tokens` (create), `GET /auth/tokens` (list), `DELETE /auth/tokens/{id}` (revoke).
- Tokens use `san_` prefix (e.g., `san_a1b2c3...`) for easy identification; only SHA-256 hash stored in DB.
- Raw token shown only once at creation time; tokens have configurable expiry (1–3650 days, default 365).
- Auth middleware accepts both session cookies and `Authorization: Bearer san_...` API tokens.
- User dashboard page (`/dashboard`) with Askama server-side templates for token management UI.
- User account page (`/account.html`) — web frontend for self-registration, login, password change, and API token creation/management.
- Landing page (`/index.html`) with service info, version display, and links to Admin Dashboard, Account, and Service Overview pages.
- `GET /version` endpoint returning the service version from `Cargo.toml`.
- One-time token display modal with copy-to-clipboard and Maven `settings.xml` usage hint.
- Introduced Askama 0.13 for server-side HTML templating (compile-time checked, auto-escaped).

#### User Management
- Local user self-registration (`POST /auth/register`) — disabled by default, admin enables via `POST /admin/settings/local-users`.
- Registered users require admin approval before login (prevents unauthorized access).
- Admin endpoints for user management: list (`GET /admin/users`), approve (`POST /admin/users/{id}/approve`), delete (`DELETE /admin/users/{id}`).
- `GET /admin/settings/local-users` to check local users setting status.
- Admin dashboard UI: Local User Registration toggle, Users list with approve/delete actions and status badges (admin, approved, pending).

#### Observability
- Added structured logging for all failed REST endpoint calls (warn-level for client errors, error-level for internal errors) to aid debugging.
- Added HTTP request/response tracing via `tower-http` `TraceLayer` for full request lifecycle visibility.
- Default log level changed from `debug` to `info`; configurable via `RUST_LOG` environment variable (e.g., `RUST_LOG=sanshain_service=debug,tower_http=debug`).
- Service overview page now dynamically discovers all provided services and branches instead of only querying the `main` branch.
- Client drill-down view in service page: browse clients → branches → endpoints → YAML content, with search/filter support.
- Service drill-down view: browse services → branches → endpoints (with used/unused status and client counts) → paginated YAML preview, with search/filter support.
- Paginated YAML viewer for large OpenAPI files (80 lines per page) used in both service and client YAML views.
- New admin API endpoints: `GET /admin/clients/{name}/branches` and `GET /admin/clients/{name}/branches/{branch}/endpoints` for client dependency drill-down.

#### Graph View
- Redesigned dependency graph: unified node model where services that are also clients appear as a single node (no artificial client/service split).
- Top-down tree layout (`graph TD`) for clearer hierarchy visualization.
- Cycle detection with red-colored edges and a warning banner when circular dependencies exist.
- Detailed view toggle: labels each edge with endpoint path and HTTP method.
- Color-coded legend distinguishing client-only, service-only, and dual-role nodes.
- Improved mermaid theming with custom colors, fonts, and stroke styles.

#### DevOps
- GitHub Actions CI workflow: build and test on every push and pull request.
- GitHub Actions Release workflow: manually triggered via GitHub UI to create a release with Linux binary, Dockerfile, docker-compose.yaml, and CHANGELOG.
- Docker image built from release template (pre-compiled binary) and pushed to GitHub Container Registry (ghcr.io) on release.
- Release includes a ready-to-use `Dockerfile` and `docker-compose.yaml` (with correct image version) generated from templates.
- README updated with initial admin password setup procedure and installation options (source, binary, Docker).
