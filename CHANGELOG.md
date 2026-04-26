# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.13.1] - 2026-04-26

### Added
- **DDD Hexagonal Architecture**: refactored the entire service from a monolithic state into a clean Domain-Driven Design structure. Logic is now separated into Domain (models/ports), Application (services), Infrastructure (adapters), and Presentation (Axum handlers) layers.
- **Centralized Error Handling**: implemented a robust error management system using `thiserror` and Axum's `IntoResponse`, ensuring consistent and helpful error messages across the API.
- **Modern Rust 2024 Features**: migrated to Rust 2024 edition, utilizing `let_chains` and other modern language features for cleaner code.

### Changed
- **Code Quality & Maintenance**: eliminated AI-generated anti-patterns, duplicate logic, and primitive obsession. Manual library-call re-implementations were replaced with standard library or crate calls.
- **Unified Logic Consolidation**: consolidated shared logic between JSON and HTMX handlers in the Application layer, improving maintainability and reducing code duplication.
- **Security Hardening**: improved authentication middleware and ensured sensitive fields (like LDAP passwords) are properly redacted in responses.
- **Dependency Refresh**: updated core dependencies to their latest versions, including `axum` 0.8, `rand` 0.10, and `argon2` 0.5.

### Fixed
- **Integration Test Alignment**: resolved all 35/35 integration test failures by aligning the new architecture with legacy API contracts and behavior.

---

Historical changes can be found in [OLDER_CHANGES.md](OLDER_CHANGES.md).
