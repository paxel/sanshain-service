# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).


## [1.6.2] - 2026-07-28

### Added
- `GET /branches/protected`, `GET /branches/metadata` and `GET /version` are part of the published contract, so generated SDKs cover them instead of requiring a hand-written call. All three already worked; previously (1.6.1) `api.yaml` described only the `/provide*` and `/require*` surface. The descriptions state what a caller needs and could not tell from the response alone: `/branches/protected` returns the protected-branch *patterns* (`release/*`), not the branch names matching them; `/branches/metadata`'s `last_modified` tracks publishes, not reads; and `/version`'s `instance_id` changes on restart.

### Fixed
- The API docs describe the current parameter names again. Previously (1.6.1) [API Usage](docs/api-usage.md), [CI Integration](docs/ci-integration.md) and [sanshain.yaml](docs/sanshain-yaml.md) still presented the pre-1.6.0 `servicename`/`clientname` as *the* field names in every example — the 1.6.0 rename never reached them — so anyone integrating from the docs built against the compatibility aliases while `api.yaml` documented those same names as legacy-only. The examples now use `producername`/`consumername`, and [API Usage](docs/api-usage.md) states in one place what the rename means for an existing caller: which request fields keep a legacy alias, that supplying both spellings of the same value is rejected as a duplicate field rather than resolved by precedence, and that the admin routes and discovery pages (`/admin/services*`, `/admin/clients*`, `services.html`, `clients.html`) were renamed with **no** alias and still return `404` under their old paths.
- `api.yaml` reports the version it actually describes. Previously (1.6.1) its `info.version` still read `1.6.0`, so a generated SDK or a spec-diffing gateway saw the 1.6.1 contract labelled as the release before it.

---

Historical changes have been moved to [OLDER_CHANGES.md](OLDER_CHANGES.md).
