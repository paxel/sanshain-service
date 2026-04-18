# Production Readiness Audit - Findings

This document lists security and performance issues identified during the code audit, categorized by criticality.

## Critical Issues

### 1. LDAP Injection Vulnerability (FIXED)
- **File**: `src/infrastructure/ldap_provider.rs`
- **Description**: The `username` provided by the user is now escaped before being inserted into the LDAP filter string.
- **Risk**: Resolved.
- **Criticality**: Critical (Security)

### 2. Infinite Memory Leak in CSRF Tokens
- **File**: `src/main.rs`
- **Description**: CSRF tokens are stored in a `HashSet` that is only appended to and never cleared or pruned.
- **Risk**: Long-running production instances will eventually consume all available memory and crash (OOM).
- **Criticality**: Critical (Stability)

### 3. Non-Expiring and Replayable CSRF Tokens
- **File**: `src/main.rs`
- **Description**: Once a CSRF token is generated and stored, it remains valid indefinitely for any request until the server restarts.
- **Risk**: Compromised CSRF tokens can be used repeatedly. They are not tied to a session or a specific time window.
- **Criticality**: High (Security)

## High Issues

### 4. Inefficient Long-Polling (Busy-Wait)
- **File**: `src/application/services.rs`
- **Description**: The `/require` and `/require-bundle` endpoints implement long-polling by looping and sleeping for 500ms while re-querying the database each time.
- **Risk**: High CPU and Database load when multiple clients are waiting. Scales poorly.
- **Criticality**: High (Performance)

### 5. CSRF Protection Blocking API Integration
- **File**: `src/main.rs`
- **Description**: CSRF protection is applied globally to all state-changing requests (POST/PUT/DELETE), including those authenticated via Bearer API tokens.
- **Risk**: Automated tools (CI/CD, CLI) using API tokens are forced to handle CSRF tokens, which is unnecessary and complex for non-cookie-based authentication.
- **Criticality**: High (Usability/Security)

## Medium Issues

### 6. Inefficient OpenAPI Splitting & Database Bloat
- **File**: `src/openapi.rs`
- **Description**: The current splitting logic is basic and includes ALL schemas/components in every per-endpoint snippet.
- **Risk**: 
    - **Performance**: Frequent serialization/deserialization of large specs.
    - **Storage**: Massive redundancy in the database (O(N*M) where N is endpoints and M is schemas). A large spec with many endpoints will bloat the database rapidly.
- **Criticality**: Medium (Performance/Storage)

### 7. Redundant Backward Compatibility Checks
- **File**: `src/application/services.rs`
- **Description**: Since every endpoint snippet contains all schemas, updating a spec with many endpoints triggers redundant full-schema compatibility checks for every single endpoint.
- **Criticality**: Medium (Performance)

### 8. Potential LDAP Connection SSRF
- **File**: `src/infrastructure/ldap_provider.rs`
- **Description**: The LDAP server URL is used directly from configuration to establish connections. 
- **Risk**: While restricted to admins, a malicious or compromised admin could point the service to internal network resources.
- **Criticality**: Medium (Security)

## Low Issues

### 9. Manual Date/Time Logic & Code Duplication
- **Files**: `src/application/services.rs`, `src/infrastructure/sqlite_repository.rs`
- **Description**: Manual implementation of date-to-YMD conversion and ISO string formatting exists in multiple places, even though `chrono` is a project dependency.
- **Criticality**: Low (Maintainability)

### 10. Use of `unwrap()` in Production Code
- **Files**: Various
- **Description**: Multiple instances of `.unwrap()` on system time durations and other operations that could theoretically fail in edge cases.
- **Criticality**: Low (Robustness)
