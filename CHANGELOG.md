# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [2.3.0] - Unreleased

### Changed
- An OpenAPI GA publish without a major bump is now rejected when it removes or newly requires a parameter, changes a parameter's type, drops a request enum value, or changes a nested or inline request/response body schema; moving an inline schema into a `$ref` with the same shape still passes.

### Security
- Updated `h2` to 0.4.19 (RUSTSEC-2026-0258) and `rustls` to 0.23.45 (RUSTSEC-2026-0285).

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
