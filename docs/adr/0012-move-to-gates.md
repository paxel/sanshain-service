---
status: accepted
---

# The service moves to the gates standard

Decided on 2026-11-18.

The service had its own gates: a CI and a Quality workflow with format, clippy, the tests, a
tarpaulin floor, `cargo audit` and `npm audit`, eslint and prettier on the page scripts, the
UI tests and a SonarQube Cloud job on `summer`; its own CodeQL, Scorecard, fuzz and weekly
Security workflows; a release that built the binaries and the image; and a build manifest
for sourcehut. Every paxel repository had done so, each differently.
[paxel/gates](https://github.com/paxel/gates) is the one standard that replaces those, as
shared workflows a repository calls, and its audit found 20 checks of it missing here
([issue 101](https://github.com/paxel/sanshain-service/issues/101)).

The service meets the standard of gates v1.2.0
([san-1000](https://github.com/paxel/sanshain-service/issues/104) to
[san-1010](https://github.com/paxel/sanshain-service/issues/114)). Where the standard decides,
it is followed without a note here: the shared OSV gate, Rust gate, TypeScript gate,
SonarQube Cloud job, CodeQL, Scorecard, fuzz, release gate and provenance workflows, one
coverage ratchet, the lint set in `Cargo.toml`, the lint config of the JavaScript, the files
and the badge row. What follows is where the service adds to it, or had a choice.

**JavaScript stays JavaScript.** The page scripts in `static/js/` are classic scripts the
service serves as they are, with no build step, and the site build in `scripts/` runs on
Node directly. The standard's stack for them is TypeScript, and it is met without a
rewrite: `tsc` checks the JavaScript (`checkJs`, the standard's strict compiler settings,
types in JSDoc), and the standard's eslint config with type information lints it
([san-1003](https://github.com/paxel/sanshain-service/issues/107)).

**Two JavaScript floors.** The shared TypeScript gate measures what `npm run test:coverage`
runs, which is the site tests: the part `site`, the site build in `scripts/`. The page
scripts are run by the Playwright tests in a browser against a started service, which no
shared gate can do. So the `ui` job of `ci.yml` measures them (`UI_COVERAGE=1`,
`tests/ui/coverage.js` writes `coverage/ui.lcov`) and checks the part `ui` itself, with
gates' ratchet action ([san-1004](https://github.com/paxel/sanshain-service/issues/108)).

**Jobs of the service's own.** CI keeps three jobs the standard has no check for: the
integration tests of `scripts/itest.sh` against a started service, the check that the
committed stylesheet is the built one, and the UI tests. The weekly Security run keeps the
grype scan of the latest released image beside the shared gate: a CVE in the image's OS
packages is in no lockfile.

**The lint set.** The service already denied unreachable, indexing, slicing and
overflowing arithmetic, but at the crate roots and only without the feature `test-support`.
The set is in `Cargo.toml` now, and the shared gate runs clippy with `--all-features`, so
the in-memory repository of the tests (`src/application/mock_repo.rs`) is held to it like
shipped code. Twelve findings in it were fixed; nothing is silenced with `#[expect]`.

**PostgreSQL tests.** They start their database through testcontainers. The shared Rust
gate runs the tests under one command, so `scripts/ci-test.sh` pulls the image once and
runs what it is given; the gate, the coverage measurement and a workstation use the same
script.

**The Rust floor went from 86.83 to 84.61, with the same tests.** The old floor stood two
points under 88.83 %, measured on Rust 1.99.1 with tarpaulin's default engine. The shared
gate measures with tarpaulin's llvm engine, on Rust 1.100.0 and with `--all-features`:
84.71 % of the 19,344 lines in `src/`, measured on a workstation. Every test target takes
part, the PostgreSQL tests included, and everything in `src/` counts. Where the four points
come from was not taken apart here; wipTracker saw the same direction in its move and found
that Rust 1.100.0 counts a closure on a line of its own as uncovered when it never ran. The
floor is the measurement minus 0.1, as the ratchet writes it; the old rule left two points
of room, the new one leaves none to lose.

**Fuzzing.** The shared workflow builds the targets unoptimised (`rust-dev`): the optimised
sanitizer build of the service crate does not fit a runner's memory. It replays the saved
crashes in `fuzz/regressions/<target>` before it fuzzes, as the service's own job did;
gates 1.2.0 offers both. The targets run on the pinned stable toolchain
with `RUSTC_BOOTSTRAP=1` instead of nightly, three minutes each as before.

**CodeQL.** `.github/codeql.yml` keeps the vendored bundles in `static/vendor`, installed
and built output, and the test directories `tests` and `benches` out of the analysis. The
tests hold the passwords, tokens and keys of throwaway instances, which CodeQL reported one
by one as hard-coded credentials: 102 of the 184 open CodeQL alerts, which close with the
config and need no entry. The unit tests inside `src/` cannot be left out by path; their
findings are accepted per file.

**Release.** The shared gate and the provenance workflow come first; the binaries are
named `sanshain-linux-x86_64` and `sanshain-linux-aarch64` when they are built, as the
release has always called them. The image stays the service's own: built from
`Dockerfile.release.template` around the attested binaries, pushed to ghcr.io, with its
SBOM as a cosign attestation and a keyless cosign signature. The standard's provenance
covers files, not images. The release still attaches `docker-compose.yaml`, `Dockerfile` and
`CHANGELOG.md`; they are not attested. `cross` is installed from a named commit instead of
whatever its main branch holds on the day.

**The sourcehut build is gone.** `.build.yml` was the build manifest for sourcehut, where
the repository used to live. `origin` is GitHub, and nothing reads the file.

**Accepted findings.** In `osv-scanner.toml`: RUSTSEC-2026-0459, the SQLite bundled by
`libsqlite3-sys`, which no released sqlx can replace (moved from `.cargo/audit.toml`); the
unmaintained notices for chrono and for astral-tokio-tar, which comes with testcontainers;
the licences of webpki-roots and webpki-root-certs (CDLA-Permissive-2.0), and of minimatch
and its glob matcher (BlueOak-1.0.0), which come with eslint. `fuzz/osv-scanner.toml`
repeats what applies to the fuzz crate's lockfile and adds libfuzzer-sys (NCSA). In
`code-scanning-ignores.toml`: the three Scorecard findings of a single maintainer,
Scorecard's count of the two accepted advisories, CodeQL's findings in the unit tests of
five source files, and the twelve SonarQube Cloud findings `sonar-project.properties` used
to silence, one entry per file. All until 2027-02-18.

**Base images.** The `Dockerfile` pins both base images by digest, which closes Scorecard's
finding. `Dockerfile.release.template`, which builds the released image, keeps `alpine:3.24`
by tag and upgrades the packages when it builds, so that a release picks up Alpine's fixes.

## Consequences

- The unit tests inside `src/` and the tests in `tests/` and `benches/` are linted in the
  second clippy run, under `-D warnings`.
- SonarQube Cloud analyses pull requests as well as `summer`, fails CI on its quality gate
  and on every open finding that is not accepted, and no longer imports a clippy report:
  clippy fails CI on any warning, so the report was always empty.
- `cargo audit` and `npm audit` are gone; osv-scanner reads `Cargo.lock`, `fuzz/Cargo.lock`
  and `package-lock.json` and also checks licences.
- The weekly Security run fails on open alerts, SonarQube Cloud findings, secret alerts,
  unmaintained dependencies and passed dates, not only on vulnerabilities.
- The release and the weekly Security run need the secret `GATES_ALERTS_TOKEN`, a
  fine-grained token that can read the repository's alerts.
- A release carries, beside what it carried before, the build provenance of the binaries as
  `sanshain-service-<tag>-provenance.sigstore.json`; the source SBOM keeps its name,
  `sanshain-service-<tag>-source.cdx.json`.
- The provenance is signed by gates' workflow, not by one of this repository, so a check
  names it: `gh attestation verify <file> -R paxel/sanshain-service --signer-repo paxel/gates`.
- The required checks on `summer` change their names with the workflows; setting them is
  [issue 115](https://github.com/paxel/sanshain-service/issues/115).
