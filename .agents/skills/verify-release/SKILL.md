---
name: verify-release
description: Thoroughly verify the service for release by running all Rust and JS tests, lints, security audits, and integration tests.
---

# Verify Release Skill

Use this skill when you need to perform a full quality check of the Sanshain Service before a release or major change. It ensures that the backend, frontend, and overall integration are stable and conform to project standards.

## Trigger
React to commands like:
- "verify release"
- "run all tests"
- "full quality check"
- "is the project ready for release?"
- "run clippy, audit, and itests"

## Procedures

### 1. Run Comprehensive Verification
The primary way to use this skill is to run the bundled verification script:

```bash
./.junie/skills/verify_release/scripts/verify_all.sh
```

This script will sequentially run:
- **Rust Checks**: Formatting (`fmt`), Linting (`clippy`), Unit Tests (`test`), Security Audit (`audit`), and Unsafe Code Check (`geiger`).
- **JS/UI Checks**: JavaScript Linting (`eslint`) and Formatting (`prettier`).
- **Integration Tests**: It starts the service and runs the bash integration suite (`itest.sh`) and Playwright UI tests.

### 2. Manual Verification (Optional)
If the automated script fails, you may need to run individual components:

- **Rust Tests**: `cargo test`
- **Clippy**: `cargo clippy -- -D warnings`
- **JS Lint**: `npx eslint static/js/`
- **Integration**: `./scripts/itest.sh` (requires service running)

## Guidelines

- **Zero Warnings**: `clippy` and `eslint` must have zero warnings.
- **Full Coverage**: All 59+ Rust lib tests must pass.
- **Security**: `cargo audit` must pass without unvetted vulnerabilities.
- **No Unsafe**: `cargo geiger` is used to monitor and minimize `unsafe` usage.
- **Clean Slate**: Ensure the database is in a clean state (e.g., using `INITIAL_ADMIN_PASSWORD`) if running integration tests locally.

## Quality Checklist
- [ ] `cargo fmt --check` passed?
- [ ] `cargo clippy -- -D warnings` passed?
- [ ] All Rust tests passed?
- [ ] `cargo audit` and `cargo geiger` checked?
- [ ] `eslint` and `prettier` passed?
- [ ] `itest.sh` passed against a running service?
- [ ] Playwright UI tests passed?
