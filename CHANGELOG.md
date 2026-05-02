# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [1.0.0] - 2026-05-02

### Breaking Changes
- **Database Migration Squash**: All legacy database migrations have been squashed into a single `20240430000000_initial_schema.sql`.
  - **IMPORTANT**: This makes direct upgrades from `v0.13.x` impossible without manual intervention. Users must either nuke their existing database or manually reconcile their schema.
  - This change was necessary to fix critical PostgreSQL production issues and establish a stable baseline for `1.0.0`.

### Major Changes
- **Migration Squashing**: Consolidated all database migrations into a single initial schema for both SQLite and PostgreSQL. This resolves issues with "previously applied but modified" migrations and simplifies fresh installations.
- **PostgreSQL Stability**: Fixed a critical `name[] = text[]` operator error in PostgreSQL migrations.
- **Robust Data Deletion**: Implemented `ON DELETE CASCADE` across all major foreign key relationships, ensuring reliable cleanup of dependent data (branches, endpoints, versions, etc.) during deletion operations.
- **Shared Contract Fix**: Corrected the `shared_contracts` table schema to use `branch_name` instead of `branch_id`, matching the application logic.

### Added
- **PostgreSQL Testing**: Integrated `testcontainers` for automated PostgreSQL integration testing. The test suite now verifies full API flows against a real PostgreSQL instance.

---

Historical pre-1.0.0 changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
