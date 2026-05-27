---
name: version-management
description: Automate version bumping, changelog maintenance, and release documentation.
---

# Version Management Skill

## Purpose
Automate version bumping, changelog maintenance, and release documentation for the Sanshain Service project.

## Trigger
React to commands like:
- "bump version to X.Y.Z"
- "release version X.Y.Z"
- "move changelog to old changelog"
- "update version everywhere"

## Version Locations

When bumping the version, **all** of the following files must be updated:

| File | Field / Location | Example |
|------|-----------------|---------|
| `Cargo.toml` | `version = "X.Y.Z"` (line ~3) | `version = "1.2.0"` |
| `api.yaml` | `info.version` (line ~7) | `version: "1.2.0"` |
| `README.md` | Example output for `GET /version` | `{"version": "1.2.0"}` |
| `CHANGELOG.md` | New section header | `## [X.Y.Z] - YYYY-MM-DD` |

### Files that should NOT be updated
- `OLDER_CHANGES.md` — historical versions are correct as-is
- `ai/plan.md` — references to old versions are historical context
- `tests/integration_test.rs` — contains spec versions in test fixtures, not the service version
- `src/presentation/handlers/pages.rs` — `text/plain; version=0.0.4` is the Prometheus exposition format version

## Guidelines

- **Consistency**: Ensure the version is updated in all required files simultaneously.
- **Traceability**: Always update the `CHANGELOG.md` when bumping the version.
- **Verification**: Never consider a version bump complete without running the verification tests.

## Procedures

### Bump Version

1. Update `Cargo.toml`: change `version = "..."` to the new version.
2. Update `api.yaml`: change `info.version` to the new version string.
3. Update `README.md`: change the `GET /version` example JSON to show the new version.
4. Update `CHANGELOG.md`: add a new `## [X.Y.Z] - YYYY-MM-DD` section at the top (below the header). Use today's date unless told otherwise.

### Move Changelog to OLDER_CHANGES.md

When releasing a new version or when explicitly requested:

1. Cut all version sections **below** the current release from `CHANGELOG.md`.
2. Paste them at the top of `OLDER_CHANGES.md` (below the file header).
3. Ensure `CHANGELOG.md` ends with a link: `Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).`
4. Verify dates are correct — do not invent dates; use git history or ask the user.

### Post-Bump Verification

After any version bump, run:
```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

Ensure all checks pass before considering the bump complete.

## Changelog Format

Follow [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format:

```markdown
## [X.Y.Z] - YYYY-MM-DD

### Added
- New features

### Changed
- Changes to existing functionality

### Fixed
- Bug fixes

### Breaking Changes
- Only for major version bumps
```

## Semantic Versioning Rules

- **Patch** (X.Y.Z+1): Bug fixes, documentation, minor improvements.
- **Minor** (X.Y+1.0): New features, non-breaking schema migrations, new endpoints.
- **Major** (X+1.0.0): Breaking database migrations, removed endpoints, incompatible API changes.
