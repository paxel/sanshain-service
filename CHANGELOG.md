# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.2.0] - 2026-08-06

### Added
- Version strings — `info.version`, the proto `// sanshain-version:` marker, and pinned `version`
  parameters — now accept an optional leading `v` and omitted MINOR/PATCH with implicit zeroes
  (`v2` → `2.0.0`); previously only exact `MAJOR.MINOR.PATCH` was accepted. Stored and answered
  versions stay canonical three-part.

### Fixed
- The observability page's log copy/download now includes the `[service@version]` context the
  on-screen view shows — previously the export dropped it, leaving provide lines without any
  reference to the producer they concern.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
