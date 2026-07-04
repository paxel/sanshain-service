# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [1.5.0] - 2026-07-04

### Added
- Initial preparation for version 1.5.0.
- Configurable request body size limit: requests with a body larger than `MAX_SPEC_BODY_BYTES` are rejected with `413 Payload Too Large` before any spec parsing happens. Earlier releases silently enforced the web framework's built-in 2 MiB limit; it is now explicit, configurable, and defaults to 4 MiB — so spec uploads between 2 and 4 MiB that previously failed now succeed. Documented in `docs/configuration.md`; the client API contract (`api.yaml`) now lists the `413` response for `/provide*` and `/require-bundle`.
- Real readiness probe: a new unauthenticated `GET /ready` endpoint runs a `SELECT 1` against the database and returns `200` when it is reachable or `503` when it is not, so orchestrators stop routing traffic to a pod with a broken database. Liveness (`GET /health`) stays database-independent. The Kubernetes readiness probe now targets `/ready`.
- The no-panic policy is now machine-enforced: `unwrap()`, `expect()`, `panic!`, `todo!`, `unimplemented!`, and `dbg!` are denied by clippy in production code via the `[lints.clippy]` table in `Cargo.toml` (test code stays exempt through `clippy.toml`).

### Changed
- The Kubernetes manifest now defaults to a single replica: the default SQLite backend uses a `ReadWriteOnce` PVC that only one pod can mount, and concurrent writers would corrupt the database file. Scaling above one replica now requires PostgreSQL — documented in `docs/deployment.md`.

### Fixed
- Removed panic paths from the `/require`, `/require/asyncapi`, `/require/grpc`, and `/require-bundle` handlers: a malformed ETag can no longer abort the response, and unmatched OpenAPI method comparison no longer panics. Malformed ETags now degrade to serving the response without a cache validator instead of failing the request.
- Removed the last production `expect()` calls: the OpenAPI path normalizer and proto splitter no longer have a regex panic path, and a failing OTLP exporter build at startup (`OTEL_ENABLED=true`) now exits with a clear error message instead of panicking.

### Changed
- `/provide*` parse-failure warnings now log a compact fingerprint of the submitted spec (byte length + short SHA-256 prefix) plus the parser error, instead of dumping the entire document. This keeps the in-memory (admin-viewable) log buffer from being flooded by large submissions; the parser error is still returned to the caller.

### Security
- Submitted API specifications are no longer written to the logs when they fail to parse. The `/provide*` parse-failure warnings now record only non-sensitive metadata (service, branch, API type, content length, and a short SHA-256 prefix) plus the parser error — never the raw spec, which can contain internal URLs, schemas, or sample secrets and would otherwise be visible in the admin log viewer. The parser error is still returned to the authenticated caller.
- API tokens are no longer accepted from a `?token=` query parameter — they must be supplied via the `Authorization: Bearer <token>` header. Query-string credentials leak through server/proxy logs, browser history, and referrer headers; header-only auth closes that exposure. Update any client that passed `?token=...` to send the header instead.
- Dev mode (which disables authentication on public API endpoints) now requires an explicit production safety gate: it only activates when requested (`SANSHAIN_DEV_MODE=true` or the persisted admin setting) **and** `ALLOW_INSECURE_DEV_MODE=true` is set in the environment. A requested-but-ungated dev mode fails closed — authentication stays enforced and a clear `SECURITY:` error is logged at startup — so a stray env var or configuration drift can no longer silently expose a production instance. See `docs/administration.md`.
- Excluded in-memory test doubles (`MockRepo`) from release builds behind a new `test-support` Cargo feature, so test-only code no longer ships in the production binary. Its lock handling was also made panic-free.


---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
