# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.13.1] - 2026-04-28

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

---

Historical changes can be found in [OLDER_CHANGES.md](OLDER_CHANGES.md).
