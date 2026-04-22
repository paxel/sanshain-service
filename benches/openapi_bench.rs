use criterion::{black_box, criterion_group, criterion_main, Criterion};
use sanshain_service::openapi::{split_openapi, merge_endpoint_yamls, generate_diff, check_backward_compatibility, normalize_path};
use sanshain_service::asyncapi::split_asyncapi;
use sanshain_service::proto::split_proto;
use std::fs;

fn bench_split_openapi(c: &mut Criterion) {
    let yaml_content = fs::read_to_string("api.yaml").expect("Failed to read api.yaml");

    c.bench_function("split_openapi", |b| {
        b.iter(|| split_openapi(black_box(&yaml_content)))
    });
}

fn bench_merge_endpoint_yamls(c: &mut Criterion) {
    let yaml_content = fs::read_to_string("api.yaml").expect("Failed to read api.yaml");
    let endpoints = split_openapi(&yaml_content).expect("Failed to split openapi");
    let endpoint_yamls: Vec<String> = endpoints.into_iter().map(|e| e.yaml_content).collect();

    c.bench_function("merge_endpoint_yamls", |b| {
        b.iter(|| merge_endpoint_yamls(black_box(&endpoint_yamls)))
    });
}

fn bench_normalize_path(c: &mut Criterion) {
    let paths = vec![
        "/api/v1/users/{id}",
        "/api/v1/services/{serviceName}/branches/{branchName}/endpoints",
        "/api//v1///users/{id:.*}/posts/{postId}",
        "/simple/path",
    ];

    c.bench_function("normalize_path", |b| {
        b.iter(|| {
            for p in &paths {
                normalize_path(black_box(p));
            }
        })
    });
}

fn bench_generate_diff(c: &mut Criterion) {
    let yaml_content = fs::read_to_string("api.yaml").expect("Failed to read api.yaml");
    let endpoints = split_openapi(&yaml_content).expect("Failed to split openapi");
    if endpoints.len() >= 2 {
        let old = &endpoints[0].yaml_content;
        let new_yaml = &endpoints[1].yaml_content;
        c.bench_function("generate_diff", |b| {
            b.iter(|| generate_diff(black_box(old), black_box(new_yaml)))
        });
    }
}

fn bench_check_backward_compatibility(c: &mut Criterion) {
    let yaml_content = fs::read_to_string("api.yaml").expect("Failed to read api.yaml");
    c.bench_function("check_backward_compatibility", |b| {
        b.iter(|| check_backward_compatibility(black_box(&yaml_content), black_box(&yaml_content)))
    });
}

fn bench_split_asyncapi(c: &mut Criterion) {
    let asyncapi_yaml = r#"
asyncapi: '2.6.0'
info:
  title: Test Service
  version: '1.0.0'
channels:
  user.created:
    publish:
      operationId: onUserCreated
      message:
        payload:
          type: object
          properties:
            userId:
              type: string
  user.updated:
    subscribe:
      operationId: sendUserUpdate
      message:
        payload:
          type: object
          properties:
            userId:
              type: string
            name:
              type: string
  order.placed:
    publish:
      operationId: onOrderPlaced
      message:
        payload:
          type: object
          properties:
            orderId:
              type: string
            amount:
              type: number
  order.shipped:
    subscribe:
      operationId: sendOrderShipped
      message:
        payload:
          type: object
          properties:
            orderId:
              type: string
            trackingNumber:
              type: string
  notification.email:
    publish:
      operationId: onEmailNotification
      message:
        payload:
          type: object
          properties:
            to:
              type: string
            subject:
              type: string
            body:
              type: string
"#;

    c.bench_function("split_asyncapi", |b| {
        b.iter(|| split_asyncapi(black_box(asyncapi_yaml)))
    });
}

fn bench_split_proto(c: &mut Criterion) {
    let proto_content = r#"
syntax = "proto3";
package example.v1;

message GetUserRequest {
  string user_id = 1;
}
message GetUserResponse {
  string user_id = 1;
  string name = 2;
  string email = 3;
}
message CreateUserRequest {
  string name = 1;
  string email = 2;
}
message CreateUserResponse {
  string user_id = 1;
}
message ListUsersRequest {
  int32 page_size = 1;
  string page_token = 2;
}
message ListUsersResponse {
  repeated GetUserResponse users = 1;
  string next_page_token = 2;
}
message DeleteUserRequest {
  string user_id = 1;
}
message DeleteUserResponse {}

service UserService {
  rpc GetUser (GetUserRequest) returns (GetUserResponse);
  rpc CreateUser (CreateUserRequest) returns (CreateUserResponse);
  rpc ListUsers (ListUsersRequest) returns (ListUsersResponse);
  rpc DeleteUser (DeleteUserRequest) returns (DeleteUserResponse);
}

service HealthService {
  rpc Check (GetUserRequest) returns (GetUserResponse);
}
"#;

    c.bench_function("split_proto", |b| {
        b.iter(|| split_proto(black_box(proto_content)))
    });
}

criterion_group!(
    benches,
    bench_split_openapi,
    bench_merge_endpoint_yamls,
    bench_normalize_path,
    bench_generate_diff,
    bench_check_backward_compatibility,
    bench_split_asyncapi,
    bench_split_proto,
);
criterion_main!(benches);
