# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [1.2.0]

### Added
- **Reset History Feature**: Added a new admin feature to reset version history for a branch. This prunes all old versions of endpoints, renumbers the latest version to 1, and resets the branch-level version counter, while preserving existing endpoints and client dependencies.
- **Admin API**: New endpoint `POST /admin/services/{name}/branches/{branch}/reset-history` to trigger the history reset.
- **Verify Release Skill**: Created `.junie/skills/verify_release` to automate full service verification, including Rust tests, JS lints, security audits, and integration tests.
- **Skill Creator**: Created `.junie/skills/skill_creator` to automate the creation and validation of AI skills according to the official Agent Skills specification.
- **Favicon**: Added a favicon link to the layout template using the service logo.
- **Graph Visualization**: Fixed a bug where circular dependency lines were incorrectly rendered as dashed lines; they are now solid purple as intended.
- **Modified Endpoint Highlighting**: Service and Client views now display a "modified" label next to endpoints that have diverged from their protected branch baseline, providing immediate visual feedback on changes.

### Fixed
- **OpenAPI Splitting Determinism**: Ensured that OpenAPI splitting is bit-for-bit deterministic by using ordered collections and explicit sorting of components, preventing false-positive change detection.
- **CI Stability**: Fixed a persistent hang during Playwright installation in GitHub Actions by purging `needrestart`, which was causing interactive prompts on Ubuntu 24.04 runners. Added verbose diagnostics to the installation process. Added `trap` to ensure background services are properly terminated on failure.
- **Repository Cleanup**: Removed untracked temporary files (`demo_test.db`, `playwright-report/`) and removed `verify_service.log` from version control.
- **UI Tests Timeout**: Resolved a critical issue where UI tests could hang for up to 6 hours in CI due to an incomplete `dialog` event listener in Playwright. Added a robust `playwright.config.js` with a 10-minute global timeout and automatic dialog dismissal to prevent future hangs.
- **Demo Scripts**: Restored functionality of `demo.sh`, `demo2.sh`, `demo3.sh`, and `demo_protocols.sh`.
  - Added automatic authentication support via `SANSHAIN_PASSWORD`.
  - Fixed `require-bundle` payload structure and usage in `demo.sh`.
  - Standardized `BASE_URL` handling across all scripts.
  - Added missing `branch` and `service` parameters to various API calls.
  - Corrected AsyncAPI operations in `demo_protocols.sh` to ensure compatibility with service filtering.
- **Skill Visibility**: Moved skills from `.agents/` to `.junie/skills/` so they are correctly discovered and displayed by the Junie CLI.
- **Skill Creator**: Updated validation script and instructions to use `.junie/skills/` as the primary skill location.

### Changed
- **Skill Conformity**: Updated `.junie/skills/version-management/SKILL.md` to conform to the official AI SKILL definitions (YAML frontmatter and standard sections).
- **Version Bump**: Bumped minor version to `1.2.0`.

### Security
- **Removed CSRF Test Backdoor**: Removed a hardcoded `X-CSRF-Token: test-csrf-token` bypass that was shipped in production CSRF middleware and allowed any caller to skip CSRF validation. Tests now register a real, non-expired token through the normal validation path.
- **Secure CSRF Skill**: Added `.junie/skills/secure-csrf` to prevent test-only bypasses or hardcoded secrets from leaking into production security checks.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
