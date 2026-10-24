# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.12.0]

### Changed
- The Docker image is based on Alpine 3.24 instead of 3.21, which reaches its end of support in November 2026.
- The SQLite backup CronJob downloads with curl 8.23.0 instead of 8.11.1.
- The release image is signed with cosign 3 in the Sigstore bundle format; verify it with cosign 3, or with cosign 2.6 and `--new-bundle-format`.
- The PostgreSQL backup CronJob runs `pg_dump` 18 instead of 17, so it also backs up a PostgreSQL 18 database; restore its dumps with `pg_restore` 18.
- OpenTelemetry exports retry a failed send up to 3 times with backoff instead of dropping the batch at once.
- Static files carry an `ETag` and answer `If-None-Match` with `304 Not Modified`; a file path with a trailing slash returns `404 Not Found`.

### Fixed
- The Docker image's health check no longer reports the container unhealthy where `localhost` resolves to IPv6 first; it asks `127.0.0.1`, where the service listens.

### Security
- The page templates no longer pull in `basic-toml`, which was unmaintained (RUSTSEC-2026-0374).

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
