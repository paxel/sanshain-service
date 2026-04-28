# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [0.13.2] - unreleased

### Added
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

---

Historical changes can be found in [OLDER_CHANGES.md](OLDER_CHANGES.md).
