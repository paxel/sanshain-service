# Client 2.2 Rollout Plan

Adapting the five Sanshain clients to the 2.2 contract, and publishing them for the first time.

All five clients currently stop at a commit titled "Adapt to Service 2.0.0". None of them speak 2.1
or 2.2. Three of the five cannot be installed by anyone: `sanshain-js` was never published to npm,
`sanshain-rs` has no git remote at all, and `sanshain-conan`'s documented `python_requires`
integration resolves from a Conan remote that does not exist.

## Starting state

| Client | Host | Registry | Publish CI | Secrets | Manifest | Latest tag |
|-----------------|--------------------------|----------------|--------------------------|--------------------------|-------------------|------------|
| SanshainMaven   | sr.ht `origin`, GH mirror | Maven Central  | `release.yml`, works     | `central` env, 4 secrets | pom `2.0.0`       | `v2.0.0`   |
| sanshain-go     | GitHub                   | Go proxy       | binaries only            | n/a                      | —                 | `v1.3.0`   |
| sanshain-js     | GitHub                   | npm            | two workflows, both fire | none                     | pkg `3.0.0`       | `v2.2.0`   |
| sanshain-rs     | none                     | crates.io      | none                     | none                     | `2.0.0`           | none       |
| SanshainConan   | sr.ht `origin`, GH mirror | none           | GitHub release only      | none                     | pyproject `2.0.0` | `v1.4.0`   |

The service has tags `v2.0.0` and `v2.1.0`. There is no `v2.2.0` tag; `release/2.2` is still open.

The `api.yaml` diff from `v2.0.0` to `HEAD` is almost entirely additive — `trunk` and `tag` are
optional parameters, `harvested_subscriptions` is a new response field, and version-string parsing
was loosened server-side. One hard break: `stability: ga` now requires the `release_ga` permission
or answers `403`, including a byte-identical re-provide that would change nothing.

## Decisions

1. Adapt all five clients against the 2.2 contract. No client tag until `v2.2.0` exists.
2. The `release_ga` 403 is fixed operationally, by granting `releaser` to producer CI tokens.
   Clients only translate the error into a message naming the role — no retry, and never a silent
   downgrade to snapshot.
3. Local version validation is loosened to `v?MAJOR[.MINOR[.PATCH]]`. Clients send the string
   verbatim and the service remains the only normalizer, so the two cannot disagree about what
   `v2.1` means.
4. `trunk` and `tag` are supplied by the pipeline — a CLI flag plus an environment variable, never
   `sanshain.yaml`. This matches `docs/ci-integration.md` and the existing `--ga` precedent: stream
   identity is a property of the invocation, not of the repository.
5. `retired` moves onto the provide DTO. The spec body becomes optional when `retired: true`, and
   the flag is gated in-handler on `ManageProducers` for that producer **or** `ReleaseGa`, so both a
   Maintainer and a Releaser (that is, CI) can retire. Plain authenticated callers keep providing
   but cannot retire.
6. The Rust client lives at `paxel/sanshain-rs` and publishes the crates `sanshain` (library) and
   `cargo-sanshain` (subcommand). Repo names are renameable and crate names are not, so the care
   goes on the crate name.
7. Client versioning: the major tracks the service major, minor and patch float per client.
8. Publish auth differs per registry. crates.io uses an existing long-lived token as a repo secret.
   npm and PyPI use OIDC trusted publishing, because npm removed legacy tokens in November 2025 and
   every granular token now expires — a stored npm token would be a standing rotation chore.
9. The npm package is renamed from `sanshainjs` to `sanshain`, matching its own `bin` name.
10. The Conan client distributes via PyPI as `sanshain-conan`; `python_requires` is replaced by a
    plain import, which the code already supports through its import-path fallback.
11. sourcehut stays `origin` for Conan and Maven, GitHub stays the mirror. Release tags are pushed
    to the `github` remote explicitly. Documented, not restructured.

## Phase 0 — service, before the tag — DONE except the tag

Steps 1–6 are implemented and verified: 732 tests pass, `cargo fmt --check` and
`cargo clippy --all-targets -- -D warnings` are clean. Step 7's tag is deliberately left to a human.


The retire endpoint `POST /admin/producers/{name}/retire/{api_type}` is removed. Verification across
the service repo and all five clients found exactly one caller: an integration test. It has no UI
and no client uses it. Its `{api_type}` path parameter is redundant once the flag rides a per-family
provide endpoint, and its `ManageProducers` guard excluded the Releaser role that CI would hold.

1. Add `retired: boolean` to `ProvidePayload`, `ProvideAsyncApiPayload` and `ProvideProtoPayload`.
   The spec body becomes optional when it is set; a request with neither a body nor `retired` is a
   `400`.
2. Gate the flag in-handler on `ManageProducers` for that producer or `ReleaseGa`, following the
   promote precedent in `src/lib.rs` — the route keeps `api_auth` and the refusal is an instructive
   `403` naming both ways in, which a guard-level refusal could not produce.
3. Remove `admin_retire_protocol`, its route, and its `maintenance.yaml` entry.
   `router_matches_maintenance_yaml_contract` enforces that pairing.
4. Port the existing retire integration test to the provide-flag path.
5. Add tests that a Releaser can retire and a plain authenticated caller cannot.
6. Update `api.yaml`, `docs/sanshain-yaml.md` (its "Retiring a protocol" section documents the admin
   call), and `CHANGELOG.md`.
7. Run `just check`, then tag `v2.2.0`. Record that commit — it is the adapt-point for Phase 3.

## Phase 1 — client adaptation, all five

Done: **SanshainMaven**, **sanshain-rs**, **sanshain-go**, **SanshainConan**. Remaining: sanshain-js.

Wire detail found late and fixed in all adapted clients: `/require-bundle` reads `trunk`/`tag`
from the **JSON body** — its handler has no query extractor — while the single-endpoint
`GET /require` takes them as query parameters. The first Maven/Rust implementations put the
bundle stream on the query string, which the server silently ignored.

| Item | Change |
|---|---|
| A | Map `403` on provide to a message naming the `releaser` role. |
| B | Loosen the local version gate. `sanshain-js/src/config.ts`, `sanshain-rs/sanshain-rs/src/lib.rs`. The other three already forward the server's answer. |
| C | Add `trunk` and `tag` as flag plus environment variable on provide and require. They are mutually exclusive; let the server answer `400` and `404`. |
| D | Print `harvested_subscriptions` entries as build warnings. Never fail the build on them — the server already answers `409` for a GA provide whose expectation is unsatisfiable, so failing locally would double-punish. |
| E | Implement `retired: true` in `sanshain.yaml` against the new provide DTO. |
| F | Normalize CRLF to LF immediately before upload. `docs/ci-integration.md` already promises the clients do this and none of them do; content comparison is byte-for-byte, so a Windows checkout generates spurious `409`s. |
| G | Copy the AsyncAPI 2.x perspective warning into each client's AsyncAPI documentation. Users read client docs, and getting this wrong harvests every subscription backwards. |

Per-client repairs on top of the shared list:

- **sanshain-go** — the module path in `go.mod` was `github.com/paxel/sanshain/sanshain-go/v2`,
  which resolved to an unrelated placeholder repository; now `github.com/paxel/sanshain-go/v2`
  (imports, README and docs updated). Client version stays 2.0.0 — no 2.x was ever tagged.
- **sanshain-js** — package renamed to `sanshain`; `deploy.yml` folded into `release.yml`; Node
  raised to 22.14.0 and npm to 11.5.1, both required by trusted publishing.
- **sanshain-rs** — a larger job than an adaptation, because the repository is a single generated
  commit that never went through a release cycle: it had no licence, no CI, no tags, placeholder
  authors, and it was the only client **not** reading `sanshain.yaml` — it read
  `[package.metadata.sanshain]` from `Cargo.toml`. That was an artifact, not a decision, so the
  config source moved to `sanshain.yaml` with no fallback (`serde_yaml_ng`, the reader the service
  already uses) and `cargo_metadata` left the dependency list. Also: Apache-2.0 plus a `LICENSE`
  file, `[workspace.package]` inheritance, edition 2024 with `rust-version = "1.85"`, the library
  crate renamed to `sanshain` with its directory, three READMEs, and the public surface narrowed —
  the payload builders and message formatters became `pub(crate)` and the unused `sync_specs` was
  deleted, because first publish freezes everything that is `pub`.
- **SanshainConan** — `python_requires` is gone: the `Sanshain` helper moved from `conanfile.py`
  into the package (`sanshainconan.integration`), `conanfile.py` was deleted, and consumers
  `pip install sanshain-conan` and import it. `pyproject.toml` gained a hatchling build, a
  `sanshain-conan` console script and a repository URL, and dropped the unused `conan` dependency.
  `release.yml` builds with `uv build` and publishes via PyPI Trusted Publishing (OIDC); the
  sourcehut manifest runs pytest instead of the now-meaningless `conan export`. Docs rewritten.

## Phase 2 — publishing setup

- **sanshain-rs** — create `paxel/sanshain-rs`, push, set `CARGO_REGISTRY_TOKEN` via
  `gh secret set` (reads stdin, so the value never enters shell history), add a release workflow
  that publishes `sanshain` before `cargo-sanshain`.
- **sanshain-js** — manual first publish from a laptop after reviewing `npm pack`, then configure
  the npm trusted publisher naming `release.yml`.
- **SanshainConan** — manual first upload after inspecting the sdist, then configure the PyPI
  trusted publisher, then a release workflow declaring `id-token: write`.
- **sanshain-go** — nothing to wire; the tag is the publish, once the module path is correct.
- **SanshainMaven** — already complete.

The workflow collapse and the manual first publish must both land **before** the trusted publisher
is configured, because that configuration names a specific workflow filename.

## Phase 3 — release

1. Re-diff the contract: `git diff <phase-0-adapt-point>..v2.2.0 -- api.yaml`, and re-check any
   client touching what it reports. This guards against 2.2 moving after the clients were adapted.
2. Set starting versions and tag on the `github` remote.

| Client | Now | First published |
|---|---|---|
| sanshain-js   | pkg `3.0.0`, tag `v2.2.0`      | `sanshain@2.3.0` |
| sanshain-rs   | `2.0.0`, no tags               | `sanshain@2.0.0`, `cargo-sanshain@2.0.0` |
| SanshainConan | pyproject `2.0.0`, tag `v1.4.0` | `sanshain-conan@2.0.0` |
| sanshain-go   | tag `v1.3.0`                   | `v2.0.0`, module `/v2` |
| SanshainMaven | pom `2.0.0`, tag `v2.0.0`      | `2.1.0` |

## Riding along — stale service documentation

`docs/clients.md` carries four claims that are no longer true, all client-facing:

- `cargo install --git https://github.com/paxel/sanshain --path cargo-sanshain` points at a
  repository containing a single 11-byte placeholder file.
- sanshain-go is credited with automatic stability detection from the git branch, which 2.0 removed.
- the documented `go install` path does not match `go.mod`.
- sanshain-js is described as "not yet on NPM".

## Open, to verify at execution time

Whether npm and PyPI allow configuring a trusted publisher for a name that has never been published.
npm documents it as a package setting, which implies not — hence the manual first publish in
Phase 2. If pre-configuration turns out to work, that manual step disappears.
