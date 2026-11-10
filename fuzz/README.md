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

`.github/workflows/fuzz.yml` runs each for 3 minutes every week. Locally, with
nightly and `cargo install cargo-fuzz`, from this directory:

```bash
cargo +nightly fuzz run openapi corpus/openapi -- -max_total_time=60
```

`corpus/<target>` holds the seed inputs; libFuzzer adds what it finds there
(commit only hand-picked seeds).

## When it finds a crash

1. Take the input from the job's `fuzz-crash-<target>` artifact (locally:
   `artifacts/<target>/`).
2. Fix the parser, with a unit test next to it that feeds the same input.
3. Add the input to `regressions/<target>/`: every run replays those first, so
   the crash cannot come back unnoticed.
