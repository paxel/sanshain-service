# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [1.7.0] - 2026-07-29

### Added
- `EXTRA_CA_CERTS_DIR`: directory of CA certificates to trust for LDAPS, added to the platform trust store. Startup fails, naming the file, on a bad certificate. See [Configuration](docs/configuration.md).
- Helm chart: `extraVolumes` and `extraVolumeMounts`. Unset, the chart renders as in 1.6.2.

### Security
- The audit timeline (`GET /api/audit/timeline`) is now admin-only; previously (1.6.2) any signed-in account or API token could read it. Non-admins now get `403`, anonymous `401`, and API tokens no longer work on this route. The Audit nav link is hidden from non-admins.

### Changed
- A Provide that changes no endpoint no longer bumps the version, stores a revision, or notifies listeners. Previously (1.6.2) every Provide bumped the patch version, so re-publishing an unchanged spec produced a version per build.
- Consequently, an edit confined to `info`, `servers` or other document-level fields is no longer versioned or stored. The first Provide still establishes `1.0.0`.

### Fixed
- The **Edit** button on the endpoint view now appears for admins, and the editor opens instead of answering "Admin access required." Editing an endpoint from the UI has never worked since the button was added; the auth helper resolved the signed-in Actor but did not pass it to the code gating the button.
- LDAPS no longer panics on first use with "Could not automatically determine the process-level CryptoProvider", which terminated the process in 1.6.2. Local authentication and plain LDAP were unaffected.
- LDAPS no longer silently trusts nothing when reading the platform certificates partly fails; previously (1.6.2) any such error left an empty trust store.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
