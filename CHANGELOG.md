# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.12.0]

### Breaking Changes
- The OIDC login refuses a provider that serves its key set (`jwks_uri`) over plain http; serve it over https, with `EXTRA_CA_CERTS_DIR` for a private CA.

### Changed
- The Docker image is based on Alpine 3.24 instead of 3.21, which reaches its end of support in November 2026.
- The SQLite backup CronJob downloads with curl 8.23.0 instead of 8.11.1.
- The release image is signed with cosign 3 in the Sigstore bundle format; verify it with cosign 3, or with cosign 2.6 and `--new-bundle-format`.
- The PostgreSQL backup CronJob runs `pg_dump` 18 instead of 17, so it also backs up a PostgreSQL 18 database; restore its dumps with `pg_restore` 18.
- OpenTelemetry exports retry a failed send up to 3 times with backoff instead of dropping the batch at once.
- Webhook deliveries and the OIDC login trust the platform's certificate authorities plus `EXTRA_CA_CERTS_DIR`, like LDAPS, instead of a root list built into the binary.
- Endpoint snippets and required specs are written in a slightly different YAML style (double quotes, quoted version strings, long single-line texts folded with `>`); their content is unchanged. The first re-provide of a snapshot counts its endpoints as updated once, and a Consumer that commits a required spec sees a one-time text diff.
- Spec documents nested deeper than 64 levels are refused; the limit was 128.
- Static files carry an `ETag` and answer `If-None-Match` with `304 Not Modified`; a file path with a trailing slash returns `404 Not Found`.

### Fixed
- The Docker image's health check no longer reports the container unhealthy where `localhost` resolves to IPv6 first; it asks `127.0.0.1`, where the service listens.

### Security
- YAML is read and written by `serde-saphyr`, which is pure Rust without `unsafe` code, instead of `serde_yaml_ng` on the unmaintained `unsafe-libyaml` (RUSTSEC-2024-0449).
- `rsa` (RUSTSEC-2023-0071, Marvin attack) and `crypto-bigint` 0.5 (RUSTSEC-2026-0387) are no longer part of the build: the OIDC login verifies ID tokens with `openid` on `ring`, and `cargo audit` runs without exceptions.
- The page templates no longer pull in `basic-toml`, which was unmaintained (RUSTSEC-2026-0374).

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
