# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [Unreleased]

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

## [1.6.2] - 2026-07-28

### Added
- `GET /branches/protected`, `GET /branches/metadata` and `GET /version` are part of the published contract, so generated SDKs cover them instead of requiring a hand-written call. All three already worked; previously (1.6.1) `api.yaml` described only the `/provide*` and `/require*` surface. The descriptions state what a caller needs and could not tell from the response alone: `/branches/protected` returns the protected-branch *patterns* (`release/*`), not the branch names matching them; `/branches/metadata`'s `last_modified` tracks publishes, not reads; and `/version`'s `instance_id` changes on restart.

### Fixed
- The API docs describe the current parameter names again. Previously (1.6.1) [API Usage](docs/api-usage.md), [CI Integration](docs/ci-integration.md) and [sanshain.yaml](docs/sanshain-yaml.md) still presented the pre-1.6.0 `servicename`/`clientname` as *the* field names in every example — the 1.6.0 rename never reached them — so anyone integrating from the docs built against the compatibility aliases while `api.yaml` documented those same names as legacy-only. The examples now use `producername`/`consumername`, and [API Usage](docs/api-usage.md) states in one place what the rename means for an existing caller: which request fields keep a legacy alias, that supplying both spellings of the same value is rejected as a duplicate field rather than resolved by precedence, and that the admin routes and discovery pages (`/admin/services*`, `/admin/clients*`, `services.html`, `clients.html`) were renamed with **no** alias and still return `404` under their old paths.
- `api.yaml` reports the version it actually describes. Previously (1.6.1) its `info.version` still read `1.6.0`, so a generated SDK or a spec-diffing gateway saw the 1.6.1 contract labelled as the release before it.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
