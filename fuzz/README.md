# Fuzzing

The parsers that read what outside systems send must not panic (the no-panic
policy in `AGENTS.md`); these targets feed them random input to find the cases
the lints and tests miss.

| Target      | What it runs                                                              |
|-------------|---------------------------------------------------------------------------|
| `openapi`   | `src/openapi.rs`: version, split, server URLs, summaries, self-comparison |
| `asyncapi`  | `src/asyncapi.rs`: split, published/subscribed/unnamed messages, URLs     |
| `proto`     | `src/proto.rs`: version marker, split, summaries, self-comparison         |
| `backstage` | `src/backstage.rs`: the Component of a pushed catalog-info.yaml           |
| `helm`      | `src/helm.rs`: the dependency names of a Product's Chart.yaml             |

`.github/workflows/fuzz.yml` runs each for 3 minutes every week, through the shared fuzz
workflow of [paxel/gates](https://github.com/paxel/gates). It runs on the toolchain
`rust-toolchain.toml` pins: cargo-fuzz needs nightly compiler flags, and `RUSTC_BOOTSTRAP=1`
gives them to the pinned stable toolchain, so the parsers are fuzzed with the compiler that
builds the release. Locally, with `cargo install cargo-fuzz --locked`, from the repository
root:

```bash
RUSTC_BOOTSTRAP=1 cargo fuzz run --dev openapi -- -max_total_time=60
```

Always `--dev`: the optimised build of the service crate with the sanitizer
needs more memory than a 16 GB machine has.

`corpus/<target>` holds the seed inputs; libFuzzer adds what it finds there
(commit only hand-picked seeds).

## When it finds a crash

1. Take the input from the failed job's `fuzz-<number>` artifact (locally:
   `fuzz/artifacts/<target>/`).
2. Fix the parser, with a unit test next to it that feeds the same input.
3. Add the input to `regressions/<target>/`: every run replays those first
   (`cargo fuzz run --dev <target> fuzz/regressions/<target> -- -runs=0`), so
   the crash cannot come back unnoticed.
