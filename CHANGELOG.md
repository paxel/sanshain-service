# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [1.3.0] - 2026-06-04

### Added
- **OFF / Maintenance Mode**: Added `AuthMode::Disabled` as a secure-by-default initial installation state. Anonymous and token-based interaction with provide/require API endpoints are blocked in this state, returning `503 Service Unavailable`.
- **OFF / Maintenance Option**: Added "OFF / Maintenance" option to the unified 4-switch Authentication configuration selector on the admin dashboard.

### Changed
- **Admin Dashboard Simplification**: Removed the redundant Developer Mode toggle section card and its obsolete JS functions (`loadDevMode`, `updateDevModeUI`, `toggleDevMode`, and `devModeEnabled`), consolidating all configuration into the single Authentication 4-switch selector.
- **Secure Default State**: Fresh installations now default to the secure "OFF / Maintenance" mode rather than "Dev Mode".

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
