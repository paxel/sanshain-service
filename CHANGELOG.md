# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


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
- **Favorites in Dev Mode**: Fixed FOREIGN KEY constraint error when marking services/clients as favorites in dev mode by ensuring the dev user exists in the database.
- **Log Spam**: Reduced `tower_http` trace logging from INFO to DEBUG/WARN to eliminate repetitive "finished processing request" messages from stdout.

### Changed
- **Full Accountability**: Completely removed all username masking logic from the backend and healed historical audit logs, ensuring full usernames are always used for all audit logs and specification metadata.
- **Comprehensive Auditing**: Extended audit logging to cover bundle requests, report generation, and backfilled missing action types for older entries, ensuring full visibility into specification usage.
- **UI Cache Control**: Implemented strict `Cache-Control: no-store` headers across all UI and API responses to prevent stale data visibility.
- **Documentation Restructuring**: Moved detailed API, Configuration, and Benchmark information from the root README.md to dedicated files in the `docs/` directory for better readability and maintainability.
- **Enhanced Breaking Change Detection**: Improved compatibility checks to detect removed paths, removed operations, and new required fields in request bodies as breaking changes on protected branches. Introduced a dedicated `BreakingChange` error variant for clearer feedback.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
