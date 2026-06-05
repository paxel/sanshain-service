---
name: changelog-updater
description: Automates changelog updates based on project version and enforces current-only policy.
---

# Changelog Updater Skill

## Purpose
Maintain a clean and accurate `CHANGELOG.md` that only contains entries for the current project version. Historical entries are automatically archived to `OLDER_CHANGES.md`.

## Trigger
React to commands like:
- "update the changelog"
- "add [change] to the changelog"
- "sync changelog with version"
- "fix changelog versioning"

## Guidelines
- **Single Source of Truth**: The project version is defined in `Cargo.toml` (or `Cargo.lock`).
- **Archive Policy**: `CHANGELOG.md` MUST only contain the header and the section for the current version. All previous versions MUST be moved to `OLDER_CHANGES.md`.
- **Format**: Follow [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format.
- **Dating**: Use the current date (YYYY-MM-DD) for new or updated sections.

## Procedures

### 1. Identify Current Version
Check the following files in order of priority:
1. `Cargo.toml`: look for `version = "X.Y.Z"` in the `[package]` section.
2. `Cargo.lock`: look for the version of the root package (usually the first package or matching the name in `Cargo.toml`).

### 2. Enforce Current-Only Policy
If `CHANGELOG.md` contains sections for versions other than the current one:
1. Read the current version from `Cargo.toml`.
2. Locate all `## [V.V.V] - YYYY-MM-DD` headers in `CHANGELOG.md`.
3. For any `[V.V.V]` that does NOT match the current version:
    - Extract the entire section (from its header until the next header or the end of entries).
    - Move this section to the top of `OLDER_CHANGES.md` (below the header).
    - Ensure `OLDER_CHANGES.md` maintains a clean list of historical versions.
4. Ensure `CHANGELOG.md` retains its main header and a link to `OLDER_CHANGES.md` at the bottom.

### 3. Add Change to Current Version
When adding a new entry:
1. Identify the current version.
2. Ensure a section for this version exists in `CHANGELOG.md`. If not, create it: `## [X.Y.Z] - YYYY-MM-DD`.
3. Add the entry under the appropriate category (`Added`, `Changed`, `Fixed`, `Removed`, `Security`).
4. Categories should be sorted as per [Keep a Changelog].

### 4. Post-Update Check
1. Verify that `CHANGELOG.md` only has the current version.
2. Verify that `OLDER_CHANGES.md` has the previous versions.
3. Verify that no information was lost during the move.

## Example CHANGELOG.md Structure
```markdown
# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [1.2.0] - 2026-06-01

### Added
- New feature description

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
```
