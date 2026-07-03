# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [1.5.0] - 2026-07-03

### Added
- Initial preparation for version 1.5.0.
- The no-panic policy is now machine-enforced: `unwrap()`, `expect()`, `panic!`, `todo!`, `unimplemented!`, and `dbg!` are denied by clippy in production code via the `[lints.clippy]` table in `Cargo.toml` (test code stays exempt through `clippy.toml`).

### Fixed
- Removed panic paths from the `/require`, `/require/asyncapi`, `/require/grpc`, and `/require-bundle` handlers: a malformed ETag can no longer abort the response, and unmatched OpenAPI method comparison no longer panics. Malformed ETags now degrade to serving the response without a cache validator instead of failing the request.
- Removed the last production `expect()` calls: the OpenAPI path normalizer and proto splitter no longer have a regex panic path, and a failing OTLP exporter build at startup (`OTEL_ENABLED=true`) now exits with a clear error message instead of panicking.

### Security
- Excluded in-memory test doubles (`MockRepo`) from release builds behind a new `test-support` Cargo feature, so test-only code no longer ships in the production binary. Its lock handling was also made panic-free.


---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
