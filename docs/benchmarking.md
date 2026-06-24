# Benchmarking Sanshain Service

We use [Criterion.rs](https://github.com/bheisler/criterion.rs) for micro-benchmarking performance-critical parts of the Sanshain Service, such as OpenAPI specification processing.

## Running Benchmarks

To run all benchmarks:

```bash
cargo bench
```

This will compile the project in release mode and run the benchmarks. Criterion will provide detailed statistics in the terminal and generate a HTML report in `target/criterion/report/index.html`.

## OpenAPI Benchmarks

The OpenAPI benchmarks are located in `benches/openapi_bench.rs`. They measure the performance of:

- `split_openapi`: Splitting a large OpenAPI file into individual endpoint snippets.
- `merge_endpoint_yamls`: Merging multiple endpoint snippets back into a single OpenAPI specification.

### Performance Results

Micro-benchmarks (via `criterion.rs`) show the following results on a typical development machine:

| Operation | Time | Notes |
|---|---|---|
| `split_openapi` | ~900 µs | Splitting an OpenAPI spec into per-endpoint snippets |
| `merge_endpoint_yamls` | ~850 µs | Bundling multiple endpoint snippets with shared components |
| `split_asyncapi` | ~119 µs | Splitting an AsyncAPI spec into per-channel snippets |
| `split_proto` | ~8 µs | Splitting a Proto file into per-method snippets |
| `normalize_path` | ~2.5 µs | Normalizing 4 endpoint paths (collapsing slashes, unifying variables) |
| `generate_diff` | ~22 µs | Generating a unified diff between two endpoint YAML snippets |
| `check_backward_compatibility` | ~913 µs | Checking backward compatibility between two OpenAPI specs |

These optimizations — including static regex compilation via `LazyLock` — ensure that even large-scale API changes are processed in sub-millisecond time, maintaining a fast feedback loop in CI/CD pipelines.

## Baseline

Before making any performance-related changes, run the benchmarks to establish a baseline:

```bash
cargo bench --bench openapi_bench
```

After making changes, run the benchmarks again to compare the results. Criterion will automatically detect the baseline and report the percentage improvement or regression.
