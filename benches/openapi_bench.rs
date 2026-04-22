use criterion::{black_box, criterion_group, criterion_main, Criterion};
use sanshain_service::openapi::{split_openapi, merge_endpoint_yamls};
use std::fs;

fn bench_openapi(c: &mut Criterion) {
    let yaml_content = fs::read_to_string("api.yaml").expect("Failed to read api.yaml");

    c.bench_function("split_openapi", |b| {
        b.iter(|| split_openapi(black_box(&yaml_content)))
    });

    let endpoints = split_openapi(&yaml_content).expect("Failed to split openapi");
    let endpoint_yamls: Vec<String> = endpoints.into_iter().map(|e| e.yaml_content).collect();

    c.bench_function("merge_endpoint_yamls", |b| {
        b.iter(|| merge_endpoint_yamls(black_box(&endpoint_yamls)))
    });
}

criterion_group!(benches, bench_openapi);
criterion_main!(benches);
