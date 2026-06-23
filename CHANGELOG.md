# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [1.4.0] - 2026-06-12

### Added
- **OpenTelemetry Tracing**: Integrated OpenTelemetry (OTEL) for distributed tracing using OTLP/gRPC.
- **Instrumentation**: Added tracing instrumentation to application services and database repositories.
- **Interactive Graph Popups**: Enhanced the dependency graph with interactive tooltips for nodes and edges.
- **Node Details**: Node popups show service metadata (branch, tags, source) and provide direct links to service details.
- **Edge Details**: Edge popups list all involved endpoints with direct links to their specific YAML specifications.
- **High-Resolution PNG Export**: Enabled high-quality PNG export with dynamic scaling (up to 10,000px resolution) to ensure readability for complex system graphs.

### Changed
- **Version Bump**: Bumped the version to `1.4.0`.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
