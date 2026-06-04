# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [1.3.0] - 2026-06-04

### Added
- **Frontend Deep-Linking and History Support**: Implemented comprehensive client-side deep-linking and browser history support for `services.html`, `clients.html`, `graph.html`, and `reports.html`. Users can now copy URLs directly from their browser's address bar to share specific views (services/clients lists, branches, selected endpoints, diagram configurations, focus tags, protocol filters, and reports). Clicking cards or options dynamically updates the address bar via the HTML5 History API without page reloads, and the browser's Back and Forward controls work seamlessly across all pages.
- **Persistent Database Audit Log**: Implemented a database-backed audit logging system for both SQLite and PostgreSQL. All successful database mutating operations (spec uploads, client token creation/revocation, user registration/approval/deletion, settings updates, and admin database resets) are now securely recorded.
- **Audit Masking and Security**: Built safe, automated username masking (e.g., `root` -> `r**t`) and absolute data sanitization, ensuring raw passwords, secrets, or API keys are never persisted. Exposed secure JSON and CSV retrieval endpoints requiring proper authentication.
- **Observability Audit Log Table**: Integrated a live-updating, auto-refreshing Database Audit Log table in the System Observability dashboard along with single-click CSV export functionality.
- **Reset History UI Action**: Added a "Reset History" button next to each branch under the "Services" list on the Admin Dashboard (`/admin.html`) with interactive confirmation, and generalized the shared confirmation modal in `static/js/common.js` to support dynamic titles and action-specific confirmation button labels.
- **OFF / Maintenance Mode**: Added `AuthMode::Disabled` as a secure-by-default initial installation state. Anonymous and token-based interaction with provide/require API endpoints are blocked in this state, returning `503 Service Unavailable`.
- **OFF / Maintenance Option**: Added "OFF / Maintenance" option to the unified 4-switch Authentication configuration selector on the admin dashboard.

### Changed
- **Admin Dashboard Simplification**: Removed the redundant Developer Mode toggle section card and its obsolete JS functions (`loadDevMode`, `updateDevModeUI`, `toggleDevMode`, and `devModeEnabled`), consolidating all configuration into the single Authentication 4-switch selector.
- **Secure Default State**: Fresh installations now default to the secure "OFF / Maintenance" mode rather than "Dev Mode".

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
