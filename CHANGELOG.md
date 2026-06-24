# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [1.4.0] - 2026-06-24

### Added
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

### Changed
- **Documentation Restructuring**: Moved detailed API, Configuration, and Benchmark information from the root README.md to dedicated files in the `docs/` directory for better readability and maintainability.
- **Enhanced Breaking Change Detection**: Improved compatibility checks to detect removed paths, removed operations, and new required fields in request bodies as breaking changes on protected branches.
- **Refactor Error Handling**: Introduced a dedicated `BreakingChange` error variant to provide clearer feedback for contract violations.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
