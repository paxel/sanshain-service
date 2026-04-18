# Production Readiness Audit - Findings

This document lists security and performance issues identified during the code audit, categorized by criticality.

## Critical Issues

### 1. LDAP Injection Vulnerability (FIXED)
- **File**: `src/infrastructure/ldap_provider.rs`
- **Description**: The `username` provided by the user is now escaped before being inserted into the LDAP filter string.
- **Risk**: Resolved.
- **Criticality**: Critical (Security)

### 2. Infinite Memory Leak in CSRF Tokens (FIXED)
- **File**: `src/main.rs`
- **Description**: CSRF tokens are now stored in a `HashMap` with timestamps and are pruned hourly by a background task.
- **Risk**: Resolved.
- **Criticality**: Critical (Stability)

### 3. Non-Expiring and Replayable CSRF Tokens (FIXED)
- **File**: `src/main.rs`
- **Description**: CSRF tokens now have a 24-hour expiration window (TTL) enforced during validation.
- **Risk**: Resolved (mitigated via TTL).
- **Criticality**: High (Security)

## High Issues

### 4. Inefficient Long-Polling (Busy-Wait) (FIXED)
- **File**: `src/application/services.rs`
- **Description**: The `/require` and `/require-bundle` endpoints now use `tokio::sync::broadcast` to wake up immediately when a new spec is provided, instead of polling every 500ms.
- **Risk**: Resolved.
- **Criticality**: High (Performance)

### 5. CSRF Protection Blocking API Integration (FIXED)
- **File**: `src/main.rs`
- **Description**: CSRF protection is now bypassed for requests that provide an `Authorization` header (API tokens), as Bearer tokens are inherently resistant to CSRF.
- **Risk**: Resolved.
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
