# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [1.1.0] - 2026-05-11

### Changed
- **Version Bump**: Promoted service to `1.1.0` to reflect accumulated improvements since `1.0.1`.
- **Version Synchronization**: Updated `api.yaml` and `README.md` to reflect the current `1.1.0` version.

### Fixed
- **Bundle Hash Stability**: `/require-bundle` responses now produce identical ETags and response bodies regardless of the order endpoints are requested, improving client-side caching.

### Added
- **Version Management Skill**: Created `.agents/version-management/SKILL.md` to automate version bumping across all project files for AI-assisted development.
- **Auto-Skip for New Services**: Services with no endpoints on any protected branch automatically skip shared contract backward-compatibility checks on feature branches, allowing free iteration during onboarding.
- **Force Mode**: New `force` parameter on all provide endpoints (`/provide`, `/provide/asyncapi`, `/provide/grpc`) resets the shared contract source to the current upload, bypassing compatibility checks. Blocked on protected branches (returns 400).

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
