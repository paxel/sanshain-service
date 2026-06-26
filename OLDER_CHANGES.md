# Older Changes

This file contains historical changelog entries for the Sanshain Service.
For recent changes, see [CHANGELOG.md](CHANGELOG.md).

## [1.4.0] - 2026-06-26

### Added
- **Audit Log with Filtering & Timeline**: Created a dedicated Audit page with a unified chronological timeline, powerful search capabilities (date range, action type, wildcard support), compact 1-2 line per entry layout, and side-by-side diff viewing via `diff2html`.
- **Read-Access Tracking**: Added auditing for specification discovery requests, providing full visibility into who is consuming which services.
- **Semantic Versioning (SemVer) for Specs**: Support for MAJOR.MINOR.PATCH versions with automatic version bump calculation based on change impact (MAJOR for breaking changes, MINOR for additions, PATCH for non-functional changes).
- **OpenTelemetry Tracing**: Integrated OpenTelemetry (OTEL) for distributed tracing using OTLP/gRPC, with instrumentation across application services and database repositories.
- **Interactive Graph Popups & PNG Export**: Enhanced the dependency graph with interactive tooltips showing service metadata and endpoint details with direct links, plus high-resolution PNG export (up to 10,000px).
- **Live Updates (SSE)**: Integrated Server-Sent Events to provide real-time updates across all dashboard pages when specifications are changed or deleted.
- **Log Viewer Copy & Download**: Added copy-to-clipboard and download-as-text-file buttons to the Observability log viewer, with a toggleable auto-scroll control and a fallback for non-HTTPS contexts.

### Fixed
- **PNG Graph Export**: Fixed PNG download failure by updating the Content Security Policy (CSP) to allow `blob:` URLs for images. Added robust error handling and sanitization to the export process to handle browser security restrictions, particularly for complex Mermaid graphs.
- **Graph Fit-to-View**: Fixed scaling and centering of the dependency graph by correctly calculating actual bounding box after layout compaction.
- **Favorites in Dev Mode**: Fixed FOREIGN KEY constraint error when marking services/clients as favorites in dev mode by ensuring the dev user exists in the database.
- **Log Spam**: Reduced `tower_http` trace logging from INFO to DEBUG/WARN to eliminate repetitive "finished processing request" messages from stdout.

### Changed
- **Full Accountability**: Completely removed all username masking logic from the backend and healed historical audit logs, ensuring full usernames are always used for all audit logs and specification metadata.
- **Comprehensive Auditing**: Extended audit logging to cover bundle requests, report generation, and backfilled missing action types for older entries, ensuring full visibility into specification usage.
- **UI Cache Control**: Implemented strict `Cache-Control: no-store` headers across all UI and API responses to prevent stale data visibility.
- **Documentation Restructuring**: Moved detailed API, Configuration, and Benchmark information from the root README.md to dedicated files in the `docs/` directory for better readability and maintainability.
- **Enhanced Breaking Change Detection**: Improved compatibility checks to detect removed paths, removed operations, and new required fields in request bodies as breaking changes on protected branches. Introduced a dedicated `BreakingChange` error variant for clearer feedback.

## [1.3.1] - 2026-06-09

### Added
- **Migration Integrity Protection**: Re-implemented automated SHA-256 integrity checksum tests (`migration_checksum_test.rs`) for all SQLite and PostgreSQL schema migration scripts to prevent accidental inline changes that break upgrade compatibility.
- **Upgrades Flow Fix**: Separated the default setting logic of `auth_mode` into a safe new database migration step `20240506000000_add_auth_mode_default.sql`, preventing checksum mismatches for upgrading users of `v1.2.0`.

### Changed
- **Version Bump**: Bumped the version to `1.3.1`.

## [1.3.0] - 2026-06-04

### Added
- **Branch Dropdown Selector Sorting**: Enhanced the user experience on the dependency graph and report views by sorting all branch selection dropdowns. Protected branches are displayed first, followed by non-protected (feature) branches sorted by their last modified date (descending, newest updated first), falling back to alphabetical sorting when modification dates match. Added a database query, repository port `list_branches_with_metadata`, and secure `/branches/metadata` REST API endpoint to retrieve branch last-modified timestamps.
- **Interactive User Favorites (Services & Clients)**: Implemented personalized favorites for services and clients. Users can mark/toggle any service or client as a favorite using an interactive star toggle icon. Marked items are dynamically pinned to the top of list views in alphabetical order, while preserving the standard alphabetical ordering for all other non-favorite items.
- **Favorites Management REST API**: Added secure API endpoints to retrieve user favorites (`GET /auth/favorites`) and manage favorites (`POST` and `DELETE /auth/favorites/{item_type}/{item_name}`), fully protected by JWT-based session authentication.
- **Database Support and Schema Migrations**: Created and integrated SQLite and PostgreSQL migration scripts for the new `user_favorites` table with complete cascade-deletion behaviors.
- **Linkable Top-Layer YAML Viewer**: Introduced a dedicated `/yaml.html` page featuring a completely scrollable YAML viewport, sidebar-driven version list and compare panel, and full deep-linking support for direct URL sharing of specific versions or unified patch diffs.
- **Interactive Blame Attribution**: Built a lightweight client-side blame algorithm displaying line-by-line history metadata (version number, author, branch, and timestamp) in real-time.
- **Unified Patch-Set Diff and Export**: Created a Git-style unified diff formatter merging differences into hunks. Enabled downloading the active YAML version or downloading the active diff as a standard `.patch` file.
- **Separate Metadata Schema**: Added a new database-level schema `endpoint_version_metadata` in SQLite and PostgreSQL to securely track uploading actor's name and source branch without altering the core schemas or impacting database performance.
- **Frontend Deep-Linking and History Support**: Implemented comprehensive client-side deep-linking and browser history support for `services.html`, `clients.html`, `graph.html`, and `reports.html`. Users can now copy URLs directly from their browser's address bar to share specific views (services/clients lists, branches, selected endpoints, diagram configurations, focus tags, protocol filters, and reports). Clicking cards or options dynamically updates the address bar via the HTML5 History API without page reloads, and the browser's Back and Forward controls work seamlessly across all pages.
- **Persistent Database Audit Log**: Implemented a database-backed audit logging system for both SQLite and PostgreSQL. All successful database mutating operations (spec uploads, client token creation/revocation, user registration/approval/deletion, settings updates, and admin database resets) are now securely recorded.
- **Audit Masking and Security**: Built safe, automated username masking (e.g., `root` -> `r**t`) and absolute data sanitization, ensuring raw passwords, secrets, or API keys are never persisted. Exposed secure JSON and CSV retrieval endpoints requiring proper authentication.
- **Observability Audit Log Table**: Integrated a live-updating, auto-refreshing Database Audit Log table in the System Observability dashboard along with single-click CSV export functionality.
- **Reset History UI Action**: Added a "Reset History" button next to each branch under the "Services" list on the Admin Dashboard (`/admin.html`) with interactive confirmation, and generalized the shared confirmation modal in `static/js/common.js` to support dynamic titles and action-specific confirmation button labels.
- **OFF / Maintenance Mode**: Added `AuthMode::Disabled` as a secure-by-default initial installation state. Anonymous and token-based interaction with provide/require API endpoints are blocked in this state, returning `503 Service Unavailable`.
- **OFF / Maintenance Option**: Added "OFF / Maintenance" option to the unified 4-switch Authentication configuration selector on the admin dashboard.

### Changed
- **Discovery Endpoint Click Experience**: Replaced the modal-based preview container in `services.html` and `clients.html` to directly route users to `/yaml.html`.
- **Admin Dashboard Simplification**: Removed the redundant Developer Mode toggle section card and its obsolete JS functions (`loadDevMode`, `updateDevModeUI`, `toggleDevMode`, and `devModeEnabled`), consolidating all configuration into the single Authentication 4-switch selector.
- **Secure Default State**: Fresh installations now default to the secure "OFF / Maintenance" mode rather than "Dev Mode".

## [1.2.0] - 2026-06-01

### Added
- **Reset History Feature**: Added a new admin feature to reset version history for a branch. This prunes all old versions of endpoints, renumbers the latest version to 1, and resets the branch-level version counter, while preserving existing endpoints and client dependencies.
- **Admin API**: New endpoint `POST /admin/services/{name}/branches/{branch}/reset-history` to trigger the history reset.
- **Verify Release Skill**: Created `.junie/skills/verify_release` to automate full service verification, including Rust tests, JS lints, security audits, and integration tests.
- **Skill Creator**: Created `.junie/skills/skill_creator` to automate the creation and validation of AI skills according to the official Agent Skills specification.
- **Favicon**: Added a favicon link to the layout template using the service logo.
- **Graph Visualization**: Fixed a bug where circular dependency lines were incorrectly rendered as dashed lines; they are now solid purple as intended.
- **Modified Endpoint Highlighting**: Service and Client views now display a "modified" label next to endpoints that have diverged from their protected branch baseline, providing immediate visual feedback on changes.

### Fixed
- **OpenAPI Splitting Determinism**: Ensured that OpenAPI splitting is bit-for-bit deterministic by using ordered collections and explicit sorting of components, preventing false-positive change detection.
- **CI Stability & Caching**: Fixed a persistent hang and random cancellations during Playwright installation in GitHub Actions by purging `needrestart`, adding GHA caching for Playwright browsers, and removing the slow, I/O-intensive `Free up space` step across all GHA workflows to prevent disk I/O saturation. Added verbose diagnostics to the installation process. Added `trap` to ensure background services are properly terminated on failure.
- **Release Trigger Fix**: Fixed a critical bug where the release workflow (`release.yml`) failed to trigger on new tag pushes because the pattern was incorrectly specified as a Regular Expression (`'v[0-9]+.[0-9]+.[0-9]+'`) instead of a valid GHA Glob pattern (`'v[0-9]*.[0-9]*.[0-9]*'`).
- **Repository Cleanup**: Removed untracked temporary files (`demo_test.db`, `playwright-report/`) and removed `verify_service.log` from version control.
- **UI Tests Timeout**: Resolved a critical issue where UI tests could hang for up to 6 hours in CI due to an incomplete `dialog` event listener in Playwright. Added a robust `playwright.config.js` with a 10-minute global timeout and automatic dialog dismissal to prevent future hangs.
- **Demo Scripts**: Restored functionality of `demo.sh`, `demo2.sh`, `demo3.sh`, and `demo_protocols.sh`.
  - Added automatic authentication support via `SANSHAIN_PASSWORD`.
  - Fixed `require-bundle` payload structure and usage in `demo.sh`.
  - Standardized `BASE_URL` handling across all scripts.
  - Added missing `branch` and `service` parameters to various API calls.
  - Corrected AsyncAPI operations in `demo_protocols.sh` to ensure compatibility with service filtering.
- **Skill Visibility**: Moved skills from `.agents/` to `.junie/skills/` so they are correctly discovered and displayed by the Junie CLI.
- **Skill Creator**: Updated validation script and instructions to use `.junie/skills/` as the primary skill location.

### Removed
- **Playwright CI Tests**: Removed Playwright UI smoke tests from CI to avoid browser-install timeouts; local UI test scripts remain available.

### Changed
- **Skill Conformity**: Updated `.junie/skills/version-management/SKILL.md` to conform to the official AI SKILL definitions (YAML frontmatter and standard sections).
- **Version Bump**: Bumped minor version to `1.2.0`.
- **Login Efficiency**: `auth_login` no longer loads all users to determine admin status; the `login` service now returns the authenticated user directly.
- **Configurable Static Directory**: Allowed overriding the static assets directory via the `STATIC_DIR` environment variable, defaulting to `"static"`.

### Security
- **Removed CSRF Test Backdoor**: Removed a hardcoded `X-CSRF-Token: test-csrf-token` bypass that was shipped in production CSRF middleware and allowed any caller to skip CSRF validation. Tests now register a real, non-expired token through the normal validation path.
- **Secure CSRF Skill**: Added `.junie/skills/secure-csrf` to prevent test-only bypasses or hardcoded secrets from leaking into production security checks.
- **Session Tokens Hashed at Rest**: Session tokens are now stored as SHA-256 hashes (matching API-token handling), so a database read no longer yields usable live sessions.
- **CSRF Bearer Fallback Hardened**: The CSRF exemption for API clients now requires a proper `Authorization: Bearer ` prefix instead of accepting any `Authorization` header value.
- **Unsafe Default Warnings**: The server now logs loud `SECURITY WARNING` messages at startup when `dev_mode` is enabled (unauthenticated API access) or when binding to all interfaces (`0.0.0.0`).
- **Hiding Password Hashes**: Prevented password hashes from being serialized and exposed in `/auth/me` and `/admin/users` responses by adding `#[serde(skip_serializing, default)]` on `User::password_hash`.
- **LDAP Bind Password Protection**: Prevented admin config updates from overwriting the stored LDAP bind password with `"****"` masking string.
- **Telemetry Mutex Poisoning**: Replaced silent swallowing of mutex locks with robust poison recovery (`.unwrap_or_else(|e| e.into_inner())`) on log buffers to prevent telemetry from silently stopping on panics.
- **Destructive Endpoint Audit Logs**: Added explicit audit logging with authenticated user context for all `/admin/nuke/*` endpoints.

## [1.1.0] - 2026-05-11

### Changed
- **Version Bump**: Promoted service to `1.1.0` to reflect accumulated improvements since `1.0.1`.
- **Version Synchronization**: Updated `api.yaml` and `README.md` to reflect the current `1.1.0` version.

### Fixed
- **Bundle Hash Stability**: `/require-bundle` responses now produce identical ETags and response bodies regardless of the order endpoints are requested, improving client-side caching.

### Added
- **Graph Fallback for Feature Branches**: New `GET /report/merged?branch=X&target=Y` endpoint merges dependency reports from two branches, tagging each node/edge as `Branch`, `Target`, or `Both`. The graph UI shows a "Target" dropdown (defaulting to the first protected branch) and renders target-only nodes as faded ghosts (30% opacity, dashed borders, 👻 indicator). Conflict detection highlights services with incompatible endpoint changes (⚠ badge).
- **Public Protected Branches Endpoint**: New `GET /branches/protected` endpoint accessible to any authenticated user, enabling the graph page to populate the target dropdown without admin privileges.
- **Version Management Skill**: Created `.agents/version-management/SKILL.md` to automate version bumping across all project files for AI-assisted development.
- **Auto-Skip for New Services**: Services with no endpoints on any protected branch automatically skip shared contract backward-compatibility checks on feature branches, allowing free iteration during onboarding.
- **Force Mode**: New `force` parameter on all provide endpoints (`/provide`, `/provide/asyncapi`, `/provide/grpc`) resets the shared contract source to the current upload, bypassing compatibility checks. Blocked on protected branches (returns 400).
- **Shared Contract Diff Viewer**: New `GET /admin/shared-contract` endpoint and "Shared Contract" tab in the endpoint detail modal on the Services page. Shows a diff between source (protected branch) and current (feature branch) YAML, owner service badge, and handles no-divergence state.

## [1.0.1] - 2026-05-07

### Fixed
- **Shared Contract Collision**: Resolved a critical bug where independent services with identical endpoint paths (e.g., `/notification`) incorrectly shared a single contract. Contracts are now explicitly scoped by `(branch_name, service_id)`, ensuring each service maintains compatibility only with its own endpoints.
- **Database Schema**: Refactored `shared_contracts` table to use `(branch_name, service_id)` as the uniqueness constraint for both SQLite and PostgreSQL, replacing the previous global `branch_name`-only scope.

### Added
- **Cross-Service Testing**: Expanded the automated integration test suite (`itest.sh`) with dedicated scenarios verifying endpoint independence across different services.

## [1.0.0] - 2026-05-02

### Breaking Changes
- **Database Migration Squash**: All legacy database migrations have been squashed into a single `20240430000000_initial_schema.sql`.
  - **IMPORTANT**: This makes direct upgrades from `v0.13.x` impossible without manual intervention. Users must either nuke their existing database or manually reconcile their schema.
  - This change was necessary to fix critical PostgreSQL production issues and establish a stable baseline for `1.0.0`.

### Major Changes
- **Migration Squashing**: Consolidated all database migrations into a single initial schema for both SQLite and PostgreSQL. This resolves issues with "previously applied but modified" migrations and simplifies fresh installations.
- **PostgreSQL Stability**: Fixed a critical `name[] = text[]` operator error in PostgreSQL migrations.
- **Robust Data Deletion**: Implemented `ON DELETE CASCADE` across all major foreign key relationships, ensuring reliable cleanup of dependent data (branches, endpoints, versions, etc.) during deletion operations.
- **Shared Contract Schema**: Established the `shared_contracts` table with `branch_name`-based scoping in the initial schema.

### Added
- **PostgreSQL Testing**: Integrated `testcontainers` for automated PostgreSQL integration testing. The test suite now verifies full API flows against a real PostgreSQL instance.

## [0.13.2] - 2026-04-30

### Fixed
- **Security Update**: Resolved a `cargo audit` vulnerability (`RUSTSEC-2025-0111` in `tokio-tar`) and warning (`RUSTSEC-2025-0134` in `rustls-pemfile`) by bumping the `testcontainers` and `testcontainers-modules` dev-dependencies to their latest versions.
- **LDAP Debugging**: Added detailed error feedback to the admin dashboard for LDAP connection failures. The UI now displays the specific error message from the backend and provides a troubleshooting guide for common LDAP issues.
- **UI Test Reliability**: Fixed a critical mismatch in element IDs (`#admin-dashboard` instead of `#admin-panel`) in Playwright smoke tests. Hardened tests by clearing `localStorage` and `sessionStorage` before runs and using unambiguous selectors, and added automatic dismissal of the "Reload" banner.
- **Security & CSRF**: Implemented a functional CSRF protection mechanism with appropriate bypasses for authenticated API clients and login routes, hardening the frontend/backend interface.
- **UI Testing**: Corrected a regression in Playwright smoke tests where an invalid element ID (`#user-display-name`) and improper wait logic caused false negatives in login verification.
- **CI Hardening**: Resolved "Connection Refused" errors in Playwright tests by explicitly decoupling compilation from service startup and increasing health check timeouts in GitHub Actions and SourceHut.
- **CI Resilience**: Hardened GitHub Actions workflows against disk space issues by adding automated cleanup of pre-installed runner software and optimizing Rust caching with `Swatinem/rust-cache`.
- **Robust CI Tooling**: Switched to `taiki-e/install-action` for installing `cargo-audit` and `cargo-tarpaulin`. This is more reliable than manual binary installation and significantly reduces CI build times and disk usage.
- **Git Maintenance**: Added `test-results/` to `.gitignore` to prevent Playwright artifacts from being tracked.
- **Automated CI Integration**: Enhanced CI pipelines for both GitHub and SourceHut:
    - **Integration Testing**: Integrated `scripts/itest.sh` into GitHub Actions and SourceHut `.build.yml` to verify the full API lifecycle on every push.
    - **UI Automation**: Added Playwright-based UI smoke tests to GitHub Actions, including automated browser dependency management.
    - **Fixed Task Runners**: Corrected the modern task runners (`justfile`, `package.json`) to use direct commands, resolving issues with broken `cargo` aliases in CI environments.
- **21st-Century Automation**: Replaced the legacy `Makefile` with modern alternatives:
    - **Cargo Aliases**: Integrated standardized task automation directly into `cargo` via `.cargo/config.toml` (`cargo fmt-check`, `cargo lint`).
    - **Justfile**: Added `justfile` for modern, clean task execution.
    - **NPM Orchestration**: Expanded the root `package.json` to coordinate tasks across the entire polyglot monorepo.
- **Standardized Test Scripts**: Added npm scripts to `package.json` for consistent local verification.
- **Enhanced `itest.sh`**: Integrated `cargo fmt --check` into the integration test suite to ensure all contributions follow the project's formatting standards.
- **ETag Support for Require**: Implemented server-side ETag generation and `If-None-Match` validation for all `/require` and `/require-bundle` endpoints. This enables client-side caching (e.g., in the Maven plugin) to avoid redundant downloads when API specifications are unchanged.
- **Cache Health Monitoring**: Enhanced the `CachedSpecRepository::cache_stats` API to provide accurate, real-time statistics including entry counts and memory usage by ensuring internal cache maintenance tasks are flushed before reporting.
- **Maven Plugin Automation Scripts**: Introduced a new `scripts/` directory in `SanshainMaven` containing demonstration and automation scripts (`demo.sh`, `setup-demo.sh`, `provide.sh`, `require.sh`).
- **Standardized Configuration Examples**: Added `settings-example.xml` in `SanshainMaven/scripts` to demonstrate how to securely manage Sanshain credentials using Maven settings and environment variables.
- **Multi-Type API Support Demo**: The Maven demo project now showcases simultaneous support for OpenAPI, AsyncAPI, and Protocol Buffers (gRPC) specifications managed via a single `sanshain.yaml`.
- **Comprehensive Integration Test Suite**: Introduced `scripts/itest.sh`, a bash-based integration test suite that verifies the full lifecycle of service operations (Auth, Provider, Consumer, Admin, Observability, and Shared Contract Rollback) with assertions and summary reporting.
- **Interface Consistency Tests**: Introduced a new suite of integration tests in `tests/interface_test.rs` that strictly validate JSON response structures for administrative endpoints, preventing regressions where raw values (booleans/integers) are returned instead of expected JSON objects.
- **UI Smoke Testing Suite**: Added a Playwright-based smoke testing framework in `tests/ui/smoke.test.js` to automate UI validation, including landing page rendering and admin settings persistence.
- **Comprehensive Admin OpenAPI Spec**: Expanded `api.yaml` to include all administrative endpoints, serving as a single source of truth for the entire API.
- **Improved Git Maintenance**: Hardened `.gitignore` to cover SQLite temporary files, JetBrains workspace state, local environment files, and tool-specific temporary folders, ensuring a cleaner repository state.

### Changed
- **Nuke Database Behavior**: Refactored `nuke_database` into a "factory reset" operation. It now preserves configuration (protected branches, settings) while wiping all transient data (services, endpoints, dependencies, non-admin users). Added transaction support for atomicity.
- **Backend API Standardization**: Hardened all administrative endpoints (including nuke operations and settings toggles) to consistently return JSON objects. For example, `/admin/nuke/branch/{branch}` now returns `{"deleted": count}` instead of a raw integer.
- **Content Security Policy**: Updated CSP to allow Tailwind CSS and jsDelivr CDNs, restoring UI functionality across all service pages including Services, Clients, Graph, Reports, and Observability.
- **Image Robustness**: Added explicit `width` and `height` attributes to critical UI images (logo, landing page graphics, account icons) across all pages to prevent layout shifts and oversized images when CSS/JS is slow or blocked.
- **Code Quality Rules**: Strengthened the AI development rules around KISS refactoring, dead-code and unused-file cleanup, duplicate setup removal, coverage targets, and required security checks.

### Fixed
- **Database Deletion Integrity**: Fixed `FOREIGN KEY` constraint failures in `delete_branch` and `delete_service` by ensuring all dependent records (including `service_spec_versions`) are cleaned up before primary record deletion. Wrapped these operations in transactions for atomicity.
- **Admin Settings Persistence**: Fixed an issue where "developer mode", "local user registration", and "auto-approve" settings were not persisting in the UI. The backend API now returns these boolean settings as JSON objects (e.g., `{"dev_mode": true}`) instead of raw boolean values, matching the frontend expectations.
- **Proto Splitting Robustness**: Replaced brittle byte-index service parsing with boundary-safe service-block extraction and added edge-case tests for non-ASCII headers and malformed service blocks.
- **Integration Test Maintainability**: Consolidated duplicated integration-test application-state setup into one shared helper.
- **PostgreSQL Migration Safety**: Hardened the `20240316000000_api_type` PostgreSQL migration so it drops legacy unique constraints by their constrained columns instead of relying on fragile auto-generated constraint names, ensuring production PostgreSQL databases can safely support multiple API types per endpoint/dependency key.
- **Dependency Deduplication Migration Coverage**: Added regression coverage for the NULL-endpoint dependency deduplication migration and clarified that it keeps the newest inserted duplicate row before enforcing the partial unique index.

### Security
- **Hardened YAML Parsing**: Migrated from the deprecated `serde_yaml` to `serde_yaml_ng`, a maintained and community-trusted fork. This addresses security audit findings and ensures continued support for OpenAPI/AsyncAPI parsing.
- **HTTP Security Headers**: Implemented standard security headers across all API and page routes, including `Content-Security-Policy`, `X-Content-Type-Options`, `X-Frame-Options`, and `Referrer-Policy`.
- **Zero Unsafe Verification**: Verified that the service source code contains zero `unsafe` blocks, achieving maximum memory safety for all internal logic.

## [0.13.1] - 2026-04-27

### Added
- **DDD Hexagonal Architecture**: refactored the entire service from a monolithic state into a clean Domain-Driven Design structure. Logic is now separated into Domain (models/ports), Application (services), Infrastructure (adapters), and Presentation (Axum handlers) layers.
- **Centralized Error Handling**: implemented a robust error management system using `thiserror` and Axum's `IntoResponse`, ensuring consistent and helpful error messages across the API.
- **Modern Rust 2024 Features**: migrated to Rust 2024 edition, utilizing `let_chains` and other modern language features for cleaner code.

### Changed
- **Isolation Report Redesign**: Replaced bullet-point lists in the Service Isolation Report with well-formatted markdown tables showing network connections with Target, Protocol, and Port columns. AsyncAPI connections now route through the central message service (KAFKA) instead of showing direct client-to-service links.
- **Report Viewer Flexible Layout**: Widened the report viewer container from `max-w-5xl` to `max-w-7xl` and added horizontal scrolling for tables with long service names. Table cells no longer wrap, ensuring readability with long paths.
- **Service Discovery Page Split**: Split the monolithic `service.html` (1693 lines) into four standalone pages — `services.html`, `clients.html`, `graph.html`, `reports.html` — each served as a first-level route. Shared utilities extracted into `js/discovery.js`. Old `/service.html` URLs redirect to the correct new page for backward compatibility.
- **Landing Page Redesign**: Replaced central link cards on the home page with a visual "How it Works" graphic explaining the Provide -> Manage -> Require lifecycle.
- **Protocol Support Visibility**: Added a new section highlighting support for OpenAPI, AsyncAPI, and Protobuf with icons.
- **Improved Navigation**: Updated the top banner to link the logged-in username directly to the account management page.
- **Code Quality & Maintenance**: eliminated AI-generated anti-patterns, duplicate logic, and primitive obsession. Manual library-call re-implementations were replaced with standard library or crate calls.
- **Unified Logic Consolidation**: consolidated shared logic between JSON and HTMX handlers in the Application layer, improving maintainability and reducing code duplication.
- **Security Hardening**: improved authentication middleware and ensured sensitive fields (like LDAP passwords) are properly redacted in responses.
- **Dependency Refresh**: updated core dependencies to their latest versions, including `axum` 0.8, `rand` 0.10, and `argon2` 0.5.

### Fixed
- **Admin API Routes**: Fixed a bug where several admin endpoints (client branches, endpoint YAML/versions) were missing from the router, causing 404 errors in the service discovery UI.
- **Service Discovery Page Robustness**: Fixed a bug where navigating directly to Reports or Graph views via URL hash would result in empty branch selectors. All discovery views (Services, Clients, Reports, Graph) are now fully autonomous and ensure their required data is loaded independently.
- **Integration Test Alignment**: resolved all 35/35 integration test failures by aligning the new architecture with legacy API contracts and behavior.
- **Graph Protocol Icons for Clients**: Fixed missing protocol icons (AsyncAPI/Protobuf) on client nodes in the dependency graph. Client tags are now derived from their dependency edge `api_type`, so clients using messaging or gRPC protocols display the correct icons.
- **Observability Metrics Link**: Restored the Prometheus endpoint at `/metrics`, fixing the broken raw metrics link from the observability page and adding regression coverage so the route cannot silently disappear again.
- **Admin Navigation and User Settings**: Restored the missing Observability link on the landing/info banner, fixed consistent active-page highlighting across the top navigation, re-enabled admin password changes by restoring the `/auth/change-password` route, and exposed the auto-acknowledge/auto-approve new users toggle in the admin UI.
- **Root Password Change Session Flow**: Fixed password changes to rotate the authenticated session and return a fresh token so `root` stays signed in after updating the password and can immediately continue using the admin/account pages without getting stuck on an invalidated session.
- **Password Change Logout Bug**: Fixed the password change forms on both the account and admin pages so that entering an incorrect current password shows a clear "Current password is incorrect" error instead of silently logging the user out and preventing re-login.
- **Logout Redirect**: Fixed logout redirecting to the non-existent `/index.html` instead of `/`, which caused a 404 after signing out.
- **Deep-Link Infinite Loading**: Fixed the services page showing an infinite loading screen when navigating via deep-links (e.g., clicking a client's "resolved → service" link). The `showServiceBranchEndpoints` and `showServiceBranches` functions now properly manage the loader overlay.
- **Admin Link Visibility**: The Admin navigation link in the top banner is now hidden for non-admin users and only shown when the logged-in user has admin privileges.
- **Cargo Audit Vulnerability**: Added `.cargo/audit.toml` to ignore RUSTSEC-2023-0071 (`rsa` crate Marvin Attack), a transitive dependency via `sqlx-mysql` that is never used at runtime since the project only uses SQLite and PostgreSQL backends. No upstream fix is available.
- **SQLite Migration Ordering**: Fixed the `20240316000000_api_type` SQLite migration so it drops the dependent `dependencies` table before rebuilding `endpoints`, avoiding foreign-key failures during startup migrations on existing databases.

## [0.13.0] - 2026-04-25

### Changed
- **Unified Navigation Banner & Footer**: harmonized the shared header across the info, admin, dashboard, account, and discovery views. The Sanshain logo now consistently links to `/`, the banner includes quick links to Discovery and Admin, and the right side consistently shows sign-in or username/logout state. Footers now include direct links to the GitHub repository and the project license.
- **Discovery Navigation Restoration**: expanded the shared banner links to include direct entry points for Services, Clients, Graph, and Reports, and taught `service.html` to honor `#clients`, `#graph`, and `#reports` deep links so those views are reachable again from every page.
- **Project License**: switched the project licensing metadata and `LICENSE` file content to Apache License 2.0.
- **AsyncAPI PUB-Only Provide**: the `/provide/asyncapi` endpoint now only stores PUB (publish/send) operations. SUB (subscribe/receive) operations are silently filtered out with a warning log. Subscribers should declare their dependencies in `sanshain.yaml` `requires` instead.
- **Circular Dependency Color**: changed circular dependency edge color in the dependency graph from red to purple (`#a855f7`) to clearly distinguish it from the orange missing dependency color.
- **Bidirectional PUB/SUB Edges**: connections between two services that have both PUB and SUB operations are now rendered as dotted lines with no arrows on either side, visually distinguishing bidirectional messaging from directional dependencies.
- **Virtual KAFKA Node**: when any AsyncAPI dependency exists in the graph, a virtual "KAFKA" node (with `messaging` tag, rendered as a rectangle with an oil drum `🛢️` emoji) is automatically added. All services involved in AsyncAPI connections are linked to it with blue dashed "register" lines (no arrows), visually distinct from PUB/SUB bidirectional edges, providing a clear visual hub for Kafka-based message-driven communication.

### Added
- **Multi-Publisher Conflict Detection (Problem 2)**: implemented a shared contract mechanism for non-protected branches. When multiple services provide the same endpoint (e.g., shared AsyncAPI channels), the first provide establishes a "source" version. Subsequent modifications by any service must be backward-compatible with the source. If a service (owner) maintains a modification, other services' changes must be backward-compatible with the owner's version. This ensures that concurrent development on the same branch doesn't lead to breaking contract conflicts.
- **Optimistic Concurrency Control (Problem 3)**: implemented version-based concurrency control for the `provide` endpoints. Each service/branch now tracks a monotonic `spec_version`. Provide requests can include an optional `base_version`; if the server's current version has advanced beyond the base, the request is rejected with a `409 Conflict` to prevent overwriting concurrent changes.
- **Provide Response Body (Problem 4)**: the `provide`, `/provide/asyncapi`, and `/provide/proto` endpoints now return a `202 Accepted` response with a JSON body containing the new `version`, a SHA-256 `content_hash` of the specification, and a summary of changes (inserts, updates, deletes).
- **Client-Side Caching Support (Problem 5)**: the server now skips specification processing and version increments if the provided content's hash matches the stored version's hash. This enables client-side plugins to implement efficient skip-if-unchanged logic.
- **Require-Side Caching**: implemented standard HTTP caching via `ETag` and `If-None-Match` headers for all require endpoints (`/require`, `/require-bundle`). This allows client plugins to skip re-downloading and re-processing specifications if they haven't changed since the last build.
- **Simplified Dependency Graph Nodes**: replaced complex node shapes (hexagon, diamond, cylinder, octagon) with standard rectangles for all services. Added clear symbols (emojis and SVG icons) to distinguish specialized services: `🛢️` (oil drum) for Kafka (AsyncAPI/messaging), `⛓️` (chains) for gRPC/Proto, and `!` in the top-left for missing services.
- **Improved Kafka Visualization**: renamed the virtual "MESSAGING" node to "KAFKA", renamed "Messaging" tag to "Kafka", and replaced the scaled Kafka logo with a more recognizable `🛢️` emoji.
- **Enhanced Register Lines**: changed the "messaging-register" dashed lines from grey to blue for better visibility and distinction.
- **Removed Hypothetical Tags**: removed the visual distinction (unique colors/shapes) and legend entries for "database" and "infrastructure" tags to simplify the graph and focus on actual communication protocols.
- **Updated Graph Legend**: the legend now reflects the new symbol-based notation, including the oil drum emoji for Kafka and the chains emoji for gRPC.

### Documentation
- **API Specification Update**: updated `api.yaml` with the new `base_version` request field and the `ProvideResponse` structure for all provide endpoints.
- **CI Integration Guide**: added a detailed section on **Optimistic Concurrency & Caching** to `docs/ci-integration.md`, explaining how to use `base_version` and `content_hash` to optimize pipelines and prevent overwrites.
- **README Update**: updated the core `README.md` with new payload examples, response structures, and explanations for content-based skipping and multi-publisher conflict detection.
- **Client Configuration Guide**: updated `docs/sanshain-yaml.md` with the new `baseVersion` field for optimistic concurrency, documented the provide response JSON format, and added a section on how to implement require-side caching in client plugins.

### Fixed
- **Consistent Banner Navigation**: fixed the shared banner rollout so all main views expose the same useful navigation targets, including direct access to Services, Clients, Graph, and Reports instead of forcing users back through the info page.
- **License Link Availability**: fixed the previously dead `/LICENSE` link in the shared footer so the project license opens correctly from the UI.

## [0.12.0] - 2026-04-23

### Added
- **AsyncAPI 3.x Support**: the service now supports both AsyncAPI 2.x and 3.x specifications. AsyncAPI 3.x documents (which use top-level `operations` with `action: send/receive` instead of `publish`/`subscribe` on channels) are now correctly parsed and split into per-operation snippets.
- **Markdown Report Viewer**: added a dedicated `report-viewer.html` page that renders markdown reports as beautifully formatted HTML using `marked.js`. Reports now open in a styled viewer with a header, copy-to-clipboard, and download buttons instead of displaying raw markdown in the browser. Both the Service Isolation Report and Full Dependency Report use the new viewer. Report button colors are now consistent (indigo-600).
- **Admin Nuke Buttons**: added bulk-delete ("nuke") actions for services, clients, non-admin users, and the entire database in the admin dashboard. Each action requires typing an exact confirmation phrase (e.g., `DELETE ALL SERVICES`) in a modal dialog to prevent accidental data loss. Skull/danger emoji emphasize the destructive nature of these operations.
- **Admin Search/Filter for Services & Clients**: added real-time search/filter input fields to the services and clients lists in the admin dashboard, with item counts and scrollable containers for large lists.
- **Danger Zone Section**: added a dedicated "Danger Zone" section at the bottom of the admin dashboard for the full database nuke operation, styled with red borders and warning text.
- **Nuke API Endpoints**: added `POST /admin/nuke/services`, `/admin/nuke/clients`, `/admin/nuke/users`, and `/admin/nuke/database` endpoints, all requiring exact confirmation text in the request body.
- **Missing Endpoint Graph Visualization**: dependency edges targeting unresolved/missing endpoints are now rendered as orange dashed lines with orange arrowheads in the custom dependency graph. Services where ALL inbound edges are missing are displayed as orange hexagonal nodes, providing immediate visual feedback on unresolved dependencies.
- **Graph Legend Updates**: added legend entries for missing dependency edges (orange dashed) and missing service nodes (orange hexagon).
- **Integration Tests for Missing Endpoints**: added tests verifying that clients with only missing endpoint dependencies appear in the client list, and that duplicate NULL endpoint_id dependency rows are properly deduplicated.

### Fixed
- **Duplicate NULL Endpoint Dependencies**: fixed a SQLite bug where the `ON CONFLICT` clause on the `dependencies` table never fired for rows with `endpoint_id IS NULL` (SQLite treats NULLs as distinct for UNIQUE constraints). Added a partial unique index and split the upsert logic to handle NULL and non-NULL endpoint_id separately, preventing duplicate dependency rows.
- **Dark Mode Graph Toolbar Visibility**: fixed the graph filter toolbar row and its buttons being invisible/unreadable in dark mode. Added dark-mode CSS overrides for the toolbar background (`bg-slate-100/50`), hover states, focus tag pills, and label text colors.
- **Report Navigation**: reports now open in the same tab instead of a new tab, so the "Back to Dashboard" link in the report viewer works as expected.
- **Missing Nuke Buttons in Admin Dashboard**: the nuke buttons (Nuke Users, Nuke Services, Nuke Clients, Danger Zone / Nuke Database) were present in the static HTML file but missing from the Askama template actually served at `/admin.html`. Added all nuke buttons, the Danger Zone section, the nuke confirmation modal, and the nuke JS functions to `templates/admin.html`.
- **Protected Branch Nuke Warning**: branch delete buttons in the admin dashboard now distinguish between protected and feature branches. Protected branches are highlighted with a red background, shield icon, and a prominent "☠️ Nuke" button with an extra warning confirmation. Feature branches retain the simple "Delete" button.
- **Branches Not Visible in Admin Dashboard**: service branches in the Services & Clients tab were hidden behind a small toggle arrow and not loaded by default. Branches are now easily expandable with a single click on the toggle arrow.
- **Infinite Loading Screen on Graph Page**: fixed an issue where the dependency graph page could get stuck on the loading screen if the graph data fetch failed or returned empty results. The loader is now properly dismissed in all code paths.

### Added
- **Nuke Branch Across All Services**: added a new section in the admin dashboard (Services & Clients tab) that allows deleting a specific branch from every service at once. Enter a branch name and confirm to remove it globally, eliminating the need to delete branches one service at a time. Backed by a new `POST /admin/nuke/branches/{branch}` endpoint.
- **Nuke Branch Autocomplete**: the branch name input for the "Nuke Branch Across All Services" action now features autocomplete, suggesting existing branch names fetched from the server for faster and error-free selection.
- **Client Branches in Admin Dashboard**: clients in the admin dashboard now have expandable branch lists, matching the service branch UI. Click the toggle arrow on any client to view its branches.
- **PUB/SUB Bidirectional Legend Entry**: added a "PUB/SUB bidir." entry to the graph legend with a dashed line style to document the new bidirectional messaging edge rendering.
- **Legend Hover-to-Highlight**: hovering over any legend entry in the dependency graph now highlights matching nodes or edges and dims everything else, making it easy to visually isolate specific node roles (client, service, tags) or edge types (circular, missing, PUB/SUB).

## [0.11.1] - 2026-04-22

### Added
- **Enhanced Graph Filtering**: added quick filter toggle buttons (OpenAPI, AsyncAPI, Proto) to the dependency graph toolbar. Users can now selectively hide or show service dependencies based on their protocol.
- **Improved Observability & Logging**: added detailed logging for specification processing.
    - Providing invalid YAML now logs the full erroneous input content at the `WARN` level to facilitate debugging of client-side issues.
    - Detailed `DEBUG` logs now report why specific endpoints are considered unchanged during a `provide` operation.
    - Successful `provide` operations now log a concise `INFO` summary of the changes applied (inserts, updates, deletes).
    - Database adapter operations (`apply_spec_changes`) now log the number of changes and individual operations at the `DEBUG` level for both SQLite and PostgreSQL backends.
- **Graph Edge Visualization**: implemented asynchronous arrow start and end points (66% outbound, 33% inbound) to reduce overlap and improve readability of complex dependency structures.
- **Unified Log View**: removed separation between important and standard logs in the observability dashboard. All logs are now displayed in a single unified stream, sorted chronologically from oldest (top) to newest (bottom).
- **Japanese Branding**: added Japanese characters for "Sanshain" (サンシャイン) and "Soka" (そうか) to the root banner, with automatic switching between the two versions based on the active theme.
- **Dark Mode Rebranding**: renamed the dark mode "Socke" gimmick to "SOKA" (Japanese for "I see").
- **Technical Performance Tuning**: exposed several internal parameters via environment variables for production tuning, including database pool sizes (`MAX_POSTGRES_CONNECTIONS`, `MAX_SQLITE_CONNECTIONS`), SQLite busy timeouts, background cleanup intervals, and in-memory buffer sizes.
- **Performance Optimization**: optimized OpenAPI specification processing with an 87% performance gain. `split_openapi` is now ~8x faster by using direct object model traversal instead of redundant YAML serializations.
- **Bulk Repository Operations**: introduced `find_endpoints_bulk` and `record_dependencies_bulk` to minimize database roundtrips during client requirement bundling.
- **N+1 Frontend Optimization**: refactored the admin dashboard to fetch services and their branches in a single bulk call, eliminating the N+1 request bottleneck for large service counts.
- **High-Performance Database Indexing**: added critical indices for branch, endpoint, and dependency lookups to ensure sub-millisecond query times.
- **Criterion.rs Benchmarking**: integrated `Criterion.rs` for reliable micro-benchmarking of hot paths and documented results in `README.md`.
- **Expanded Benchmarks**: added benchmarks for `split_asyncapi`, `split_proto`, `normalize_path`, `generate_diff`, and `check_backward_compatibility` to the Criterion suite for comprehensive performance tracking.
- **Static Regex Compilation**: replaced per-call `Regex::new()` in `normalize_path` (2 regexes) and `split_proto` (1 regex) with `LazyLock` statics, yielding a ~50% improvement in `split_openapi` throughput.
- **In-Memory Spec Cache**: added a high-performance, memory-bounded in-memory cache layer (`CachedSpecRepository`) using the `moka` crate. The cache sits between the application services and the database, dramatically reducing DB round-trips for hot read paths (provide, require, require-bulk, reports, graph). Features include lazy fill on cache miss, write-through invalidation on every DB write, weighted byte-size eviction (TinyLFU + LRU), configurable memory limit via `CACHE_MEMORY_MB` env var (default 256 MB), admin UI for runtime configuration and cache stats, and a JSON stats endpoint for observability.
- **Graph Focus Tag Cloud**: replaced the single-service focus input with a multi-service tag cloud. Users can add multiple services as tags (Enter key), each with an × remove button. All tagged services and their direct peers are drawn. Removing the last tag restores the full graph.
- **Viewport-Sized Graph Canvas**: the graph SVG now fills the available browser viewport height with fit-to-view scaling and centering, eliminating page-level scrolling. Navigation is done entirely via built-in zoom/pan. The canvas auto-resizes on window resize.
- **Graph Toolbar Reorganization**: moved Circular and Protocol toggle buttons to the right side of the toolbar, grouped with Direction/Copy/Download. The left side now contains only the Focus tag cloud and input.
- **Configurable Instance ID**: added support for the `INSTANCE_ID` environment variable to allow overriding the randomly generated unique ID for a service instance.
- **Custom Prometheus Endpoint**: introduced the `PROMETHEUS_ENDPOINT` environment variable, allowing administrators to customize the metrics scraping path (defaults to `/metrics`).
- **Configurable Static Assets Directory**: added the `STATIC_DIR` environment variable to allow serving web assets from a custom directory.
- **Enhanced Initial Admin Setup**: the initial root user can now be pre-configured via `INITIAL_ADMIN_USERNAME` and `INITIAL_ADMIN_PASSWORD` environment variables. The startup message now also dynamically includes the correct `BIND_ADDRESS`.
- **Configurable Session Duration**: added `LOGIN_SESSION_DURATION_HOURS` to allow customizing how long user sessions remain valid (defaults to 24 hours).
- **Flexible Log Capture Filtering**: introduced `CAPTURE_LOG_FILTER` to allow tuning the level and scope of logs captured in the in-memory observability buffer independently from the main stdout logs.
- **Simplified `sanshain.yaml` Format**: relocated `serviceName` to the root of the configuration file and replaced protocol-specific file fields (`openApiFile`, `protoFile`, etc.) with a single, generic `file` parameter. `apiType` now defaults to `openapi` if omitted.
- **Multiple Provides Support**: updated `docs/sanshain-yaml.md` to support a `provides` list in `sanshain.yaml`, allowing a single service to publish multiple API specifications simultaneously.
- **Extended Configuration Example**: updated the main `sanshain.yaml` example in `docs/sanshain-yaml.md` to demonstrate a multi-protocol configuration supporting OpenAPI, AsyncAPI, and gRPC/Proto simultaneously.
- **Unified Client Configuration**: updated `docs/sanshain-yaml.md` with detailed support and examples for AsyncAPI and gRPC/Proto in the `sanshain.yaml` format.
- **Protocol-Specific Examples**: added comprehensive examples for providing and requiring AsyncAPI channels and gRPC methods, including instructions on how to call the endpoints.
- **AsyncAPI v2→v3 Upgrade Guide**: added a version compatibility section to `docs/sanshain-yaml.md` documenting the key differences between AsyncAPI 2.x and 3.x (operation mapping, channel identifier resolution), the restriction around channel `address` vs key name in v3, and a step-by-step migration checklist.
- **Spec-to-YAML Matching Guide**: added a comprehensive "How Matching Works" section to `docs/sanshain-yaml.md` with detailed side-by-side examples showing exactly how OpenAPI paths, AsyncAPI channels, and Proto services/methods from spec files map to `sanshain.yaml` `path` and `method` fields, including a quick-reference table.

### Security
- **Removed pre-created admin session token**: the initial admin setup no longer creates a long-lived session token (previously valid until 2099) or prints it to stderr. This eliminates the risk of tokens leaking through log aggregation systems (Loki, etc.). Admins must now log in via `POST /login` with the printed credentials to obtain a session token. The `INITIAL_ADMIN_TOKEN` environment variable is no longer supported.

### Fixed
- **Docker Release Build**: fixed the release Docker build by ensuring database migrations are included in the Docker build context. The release workflow now copies migrations alongside static assets, and `Dockerfile.release.template` references the correct context-relative path.
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
- **SOKA Dark-Mode Gimmick**: introduced a "SOKA" theme gimmick that automatically rebrands the service from "Sanshain" to "SOKA" and swaps the sun logo for a "SOKA" image when dark mode is enabled.

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
