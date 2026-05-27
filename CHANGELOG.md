# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [1.2.0] - 2026-05-27

### Fixed
- **Demo Scripts**: Restored functionality of `demo.sh`, `demo2.sh`, `demo3.sh`, and `demo_protocols.sh`.
  - Added automatic authentication support via `SANSHAIN_PASSWORD`.
  - Fixed `require-bundle` payload structure and usage in `demo.sh`.
  - Standardized `BASE_URL` handling across all scripts.
  - Added missing `branch` and `service` parameters to various API calls.
  - Corrected AsyncAPI operations in `demo_protocols.sh` to ensure compatibility with service filtering.
- **Skill Visibility**: Moved skills from `.agents/` to `.junie/skills/` so they are correctly discovered and displayed by the Junie CLI.
- **Skill Creator**: Updated validation script and instructions to use `.junie/skills/` as the primary skill location.

### Added
- **Verify Release Skill**: Created `.junie/skills/verify_release` to automate full service verification, including Rust tests, JS lints, security audits, and integration tests.
- **Skill Creator**: Created `.junie/skills/skill_creator` to automate the creation and validation of AI skills according to the official Agent Skills specification.
- **Favicon**: Added a favicon link to the layout template using the service logo.
- **Modified Endpoint Highlighting**: Service and Client views now display a "modified" label next to endpoints that have diverged from their protected branch baseline, providing immediate visual feedback on changes.

### Changed
- **Skill Conformity**: Updated `.junie/skills/version-management/SKILL.md` to conform to the official AI SKILL definitions (YAML frontmatter and standard sections).
- **Version Bump**: Bumped minor version to `1.2.0`.

## [1.1.0] - 2026-05-11

### Changed
- **Version Bump**: Promoted service to `1.1.0` to reflect accumulated improvements since `1.0.1`.
- **Version Synchronization**: Updated `api.yaml` and `README.md` to reflect the current `1.1.0` version.

### Fixed
- **Bundle Hash Stability**: `/require-bundle` responses now produce identical ETags and response bodies regardless of the order endpoints are requested, improving client-side caching.

### Added
- **Graph Fallback for Feature Branches**: New `GET /report/merged?branch=X&target=Y` endpoint merges dependency reports from two branches, tagging each node/edge as `Branch`, `Target`, or `Both`. The graph UI shows a "Target" dropdown (defaulting to the first protected branch) and renders target-only nodes as faded ghosts (30% opacity, dashed borders, 👻 indicator). Conflict detection highlights services with incompatible endpoint changes (⚠ badge).
- **Public Protected Branches Endpoint**: New `GET /branches/protected` endpoint accessible to any authenticated user, enabling the graph page to populate the target dropdown without admin privileges.
- **Version Management Skill**: Created `.agents/version-management/SKILL.md` to automate version bumping across all project files for AI-assisted development.
- **Auto-Skip for New Services**: Services with no endpoints on any protected branch automatically skip shared contract backward-compatibility checks on feature branches, allowing free iteration during onboarding.
- **Force Mode**: New `force` parameter on all provide endpoints (`/provide`, `/provide/asyncapi`, `/provide/grpc`) resets the shared contract source to the current upload, bypassing compatibility checks. Blocked on protected branches (returns 400).
- **Shared Contract Diff Viewer**: New `GET /admin/shared-contract` endpoint and "Shared Contract" tab in the endpoint detail modal on the Services page. Shows a diff between source (protected branch) and current (feature branch) YAML, owner service badge, and handles no-divergence state.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
