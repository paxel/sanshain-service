# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.2.0] - 2026-08-06

### Added
- Version strings — `info.version`, the proto `// sanshain-version:` marker, and pinned `version`
  parameters — now accept an optional leading `v` and omitted MINOR/PATCH with implicit zeroes
  (`v2` → `2.0.0`); previously only exact `MAJOR.MINOR.PATCH` was accepted. Stored and answered
  versions stay canonical three-part.
- Version rows on the producers page now offer "View spec": the full stored document opens in the
  viewer with the same version-history sidebar, diff and blame as the endpoint view — previously
  the UI only offered it as a file download.
- When a GA re-provide differs only in bytes the splitter doesn't see (whitespace, line endings,
  comments), the `409` now says so instead of only asking whether you forgot to bump — content
  comparison is and stays byte-for-byte.

### Changed
- UI messages and docs no longer explain resolution by contrast with removed 1.x mechanics
  ("fails immediately", "hard-fail", "no fallback and nothing waits"); they now state the
  current behavior plainly (a missing Pin answers 404, a missing endpoint 410).

### Fixed
- The observability page's log copy/download now includes the `[service@version]` context the
  on-screen view shows — previously the export dropped it, leaving provide lines without any
  reference to the producer they concern.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
