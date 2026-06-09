# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [1.3.1] - 2026-06-09

### Added
- **Migration Integrity Protection**: Re-implemented automated SHA-256 integrity checksum tests (`migration_checksum_test.rs`) for all SQLite and PostgreSQL schema migration scripts to prevent accidental inline changes that break upgrade compatibility.
- **Upgrades Flow Fix**: Separated the default setting logic of `auth_mode` into a safe new database migration step `20240506000000_add_auth_mode_default.sql`, preventing checksum mismatches for upgrading users of `v1.2.0`.

### Changed
- **Version Bump**: Bumped the version to `1.3.1`.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
