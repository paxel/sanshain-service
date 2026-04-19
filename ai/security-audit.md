# Security Audit Report — April 2026

This document summarizes the findings of a security audit performed on the Sanshain Service.

## Summary

The service follows security best practices in most areas, including authentication, database access, and session management. Some areas for improvement were identified, primarily related to web security headers and rate limiting.

## Findings

### 1. SQL Injection (Risk: Low)
- **Status**: **VERIFIED SAFE**
- **Details**: All database interactions in `SqliteSpecRepository` and `PostgresSpecRepository` use parameterized queries via `sqlx`. No instances of manual string concatenation for SQL were found.

### 2. Path Traversal (Risk: Low)
- **Status**: **VERIFIED SAFE**
- **Details**: Static files are served using `tower_http::ServeDir`, which contains built-in protections against path traversal. OpenAPI specification splitting in `openapi.rs` operates purely on string content and does not perform file system operations.

### 3. Cross-Site Request Forgery (CSRF) (Risk: Low)
- **Status**: **MITIGATED**
- **Details**: The service uses Bearer tokens stored in `localStorage` for both web UI and API authentication. Since it does not rely on cookie-based authentication for sensitive state-changing operations, it is naturally resistant to CSRF. Furthermore, a dedicated CSRF protection middleware is implemented for extra defense in depth.

### 4. Cross-Site Scripting (XSS) & Content Security Policy (Risk: Medium)
- **Status**: **PARTIALLY MITIGATED**
- **Details**: The current Content Security Policy (CSP) includes `'unsafe-inline'` for `script-src` and `style-src`. This is required by the use of the Tailwind CSS "play" CDN and inline event handlers in templates.
- **Recommendation**: Move away from the Tailwind CDN to a build-time compiled CSS file. Refactor inline event handlers (`onclick`) to external JavaScript files. This will allow the removal of `'unsafe-inline'` from the CSP.

### 5. Authentication & LDAP Shadow Accounts (Risk: Low)
- **Status**: **VERIFIED SAFE**
- **Details**: LDAP users are auto-provisioned as shadow accounts with an invalid password hash placeholder (`!ldap-managed!`). This prevents local authentication takeover if LDAP is disabled. Password storage uses Argon2, and API tokens are stored as SHA-256 hashes.

### 6. Rate Limiting (Risk: Medium)
- **Status**: **NOT IMPLEMENTED**
- **Details**: There is currently no rate limiting on authentication or data-providing endpoints. This could allow for brute-force attacks on passwords or denial-of-service through mass spec uploads.
- **Recommendation**: Implement a rate-limiting middleware (e.g., using `tower-limit`) for sensitive endpoints like `/auth/login` and `/provide`.

### 7. Dependency Vulnerabilities (Risk: Low)
- **Status**: **FIXED / MONITORED**
- **Details**: A comprehensive `cargo audit` identified several vulnerabilities in direct and indirect dependencies.
  - **ring** (v0.16.20): Fixed by upgrading `ldap3` to `v0.12.1`.
  - **rand** (v0.8/v0.9): Fixed by upgrading to `v0.10.1`.
  - **argon2** (v0.6.0-rc.8): Replaced with stable `v0.5.3` to avoid yanked dependencies.
  - **rustls-webpki** & **time**: Resolved via `cargo update`.
  - **rsa** (v0.9.10): Identified in `sqlx-mysql` (Marvin Attack). Since the service does not use MySQL, this is not exploitable. No fixed version is currently available for the 0.9.x branch of `rsa`.
- **Action Taken**: Updated `Cargo.toml` and executed `cargo update`. `cargo audit` now passes (with one ignored non-exploitable finding).
- **Recommendation**: Maintain regular dependency audits and consider moving to `sqlx` 0.9 once a stable release is available, which is expected to resolve the `rsa` issue.

## Code Quality & Bugs
- **Clippy Audit**: A full `cargo clippy` run identified 28 issues including redundant closures, complex types, and collapsible `if` statements. All 28 issues have been fixed in the current development version.
