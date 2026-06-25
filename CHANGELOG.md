# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [1.4.0] - 2026-06-25

### Added
- **Audit Log Filtering**: Introduced powerful search capabilities for the audit log, including date range, action type (Read/Write/Admin), and wildcard support for service and branch names.
- **Unified Audit Timeline**: Consolidated specification changes and system activities into a single chronological timeline for better visibility.
- **Compact Audit Display**: Redesigned the Audit page with a space-efficient 1-2 line per entry layout, significantly improving readability for high-activity systems.
- **Read-Access Tracking**: Added auditing for specification discovery requests, providing full visibility into who is consuming which services.
- **Semantic Versioning (SemVer) for Specs**: Support for MAJOR.MINOR.PATCH versions for overall specification versions.
- **Automatic Version Incrementing**: Automatic calculation of version bumps based on change impact (MAJOR for breaking changes, MINOR for additions, PATCH for non-functional changes).
- **OpenTelemetry Tracing**: Integrated OpenTelemetry (OTEL) for distributed tracing using OTLP/gRPC.
- **Instrumentation**: Added tracing instrumentation to application services and database repositories.
- **Interactive Graph Popups**: Enhanced the dependency graph with interactive tooltips for nodes and edges.
- **Node Details**: Node popups show service metadata (branch, tags, source) and provide direct links to service details.
- **Edge Details**: Edge popups list all involved endpoints with direct links to their specific YAML specifications.
- **High-Resolution PNG Export**: Enabled high-quality PNG export with dynamic scaling (up to 10,000px resolution) to ensure readability for complex system graphs.
- **Live Updates (SSE)**: Integrated Server-Sent Events to provide real-time updates across all dashboard pages when specifications are changed or deleted.
- **Audit Timeline**: Created a dedicated Audit page featuring a global chronological timeline of all specification updates.
- **Side-by-Side Diff Viewer**: Integrated `diff2html` for interactive visualization of specification changes in the Audit view.

### Fixed
- **Audit Log Integrity**: Successfully "healed" the audit log history by un-masking redacted usernames (e.g., `r**t` -> `root`, `d******r` -> `dev_user`) via database migrations.
- **Username Anonymization Persistence**: Resolved a regression where usernames could still appear redacted in audit logs due to stale server state and aggressive UI caching.
- **Audit Log Filtering**: Resolved an issue where the filter button would reset the search parameters and reload the page instead of applying them.
- **Audit Log Categorization**: Backfilled missing action types for older audit log entries, ensuring they appear correctly in filtered views.
- **Race Conditions in Dashboard**: Fixed a race condition where some dashboard pages (Audit, Graph, Reports) might fail to load if the DOM was already interactive before scripts were fully parsed.
- **Dependency Graph Links**: Resolved an issue where clicking service nodes in the graph used an incorrect data attribute, resulting in broken "undefined" links.
- **Loading Screen Stalling**: Fixed a bug where the YAML viewer and other pages would remain stuck on a static loading screen due to missing `hideLoader()` calls.
- **Username Anonymization**: Removed username masking from audit logs and change history, ensuring full accountability for specification changes.

### Changed
- **Full Accountability**: Completely removed all username masking logic from the backend, ensuring full usernames are always used for all audit logs and specification metadata.
- **UI Cache Control**: Implemented strict `Cache-Control: no-store` headers across all UI and API responses to prevent stale data visibility.
- **Comprehensive Auditing**: Extended audit logging to cover bundle requests and report generation, ensuring full visibility into specification usage.
- **Documentation Restructuring**: Moved detailed API, Configuration, and Benchmark information from the root README.md to dedicated files in the `docs/` directory for better readability and maintainability.
- **Enhanced Breaking Change Detection**: Improved compatibility checks to detect removed paths, removed operations, and new required fields in request bodies as breaking changes on protected branches.
- **Refactor Error Handling**: Introduced a dedicated `BreakingChange` error variant to provide clearer feedback for contract violations.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
