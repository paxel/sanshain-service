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

## Baseline

Before making any performance-related changes, run the benchmarks to establish a baseline:

```bash
cargo bench --bench openapi_bench
```

After making changes, run the benchmarks again to compare the results. Criterion will automatically detect the baseline and report the percentage improvement or regression.
