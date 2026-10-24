# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.12.0]

### Changed
- The Docker image is based on Alpine 3.24 instead of 3.21, which reaches its end of support in November 2026.
- The SQLite backup CronJob downloads with curl 8.23.0 instead of 8.11.1.

### Security
- The page templates no longer pull in `basic-toml`, which was unmaintained (RUSTSEC-2026-0374).

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
