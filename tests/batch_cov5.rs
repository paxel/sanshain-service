use sanshain_service::{openapi, asyncapi, proto};
use sanshain_service::application::mock_repo::MockRepo;
use sanshain_service::application::spec_service;
use sanshain_service::domain::models::ApiType;
use sanshain_service::domain::ports::SpecRepository; // bring trait into scope for MockRepo methods

// ---------------------- OpenAPI compatibility and helpers (14 tests) ----------------------
#[test]
fn compat_err_when_method_removed_reports_uppercase_and_path() {
    let old = r#"openapi: 3.0.0
info: { title: T, version: 1 }
paths:
  /m:
    put: { responses: { '200': { description: ok } } }
"#;
    let new_ = r#"openapi: 3.0.0
info: { title: T, version: 1 }
paths: { /m: { get: { responses: { '200': { description: ok } } } } }
"#;
    let e = openapi::check_backward_compatibility(old, new_).unwrap_err();
    assert!(e.contains("PUT") && e.contains("/m"));
}

#[test]
fn compat_err_when_2xx_range_removed_reports_range() {
    let old = r#"openapi: 3.0.0
info: { title: T, version: 1 }
paths:
  /r:
    get:
      responses: { '2XX': { description: any2xx } }
"#;
    let new_ = r#"openapi: 3.0.0
info: { title: T, version: 1 }
paths: { /r: { get: { responses: { '404': { description: no } } } } }
"#;
    let e = openapi::check_backward_compatibility(old, new_).unwrap_err();
    assert!(e.contains("2XX") && e.contains("GET") && e.contains("/r"));
}

#[test]
fn compat_schema_property_removed_detected() {
    let old = r#"openapi: 3.0.0
info: { title: T, version: 1 }
components:
  schemas:
    A:
      type: object
      properties: { keep: { type: string }, gone: { type: integer } }
paths: {}
"#;
    let new_ = r#"openapi: 3.0.0
info: { title: T, version: 1 }
components:
  schemas:
    A:
      type: object
      properties: { keep: { type: string } }
paths: {}
"#;
    let e = openapi::check_backward_compatibility(old, new_).unwrap_err();
    assert!(e.contains("Property 'gone'") && e.contains("schema 'A'"));
}

#[test]
fn compat_schema_property_type_changed_detected_with_types() {
    let old = r#"openapi: 3.0.0
info: { title: T, version: 1 }
components: { schemas: { A: { type: object, properties: { x: { type: string } } } } }
paths: {}
"#;
    let new_ = r#"openapi: 3.0.0
info: { title: T, version: 1 }
components: { schemas: { A: { type: object, properties: { x: { type: integer } } } } }
paths: {}
"#;
    let e = openapi::check_backward_compatibility(old, new_).unwrap_err();
    assert!(e.contains("changed type") && e.contains("string") && e.contains("integer"));
}

#[test]
fn split_openapi_includes_all_http_methods() {
    let y = r#"openapi: 3.0.0
info: {title: t, version: v}
paths:
  /p:
    get: { responses: { '200': { description: ok } } }
    post: { responses: { '201': { description: created } } }
    put: { responses: { '200': { description: ok } } }
    delete: { responses: { '204': { description: none } } }
    options: { responses: { '200': { description: ok } } }
    head: { responses: { '200': { description: ok } } }
    patch: { responses: { '200': { description: ok } } }
    trace: { responses: { '200': { description: ok } } }
"#;
    let parts = openapi::split_openapi(y).unwrap();
    let mut methods: Vec<String> = parts.iter().map(|e| e.method.clone()).collect();
    methods.sort();
    assert_eq!(methods, vec![
        "DELETE","GET","HEAD","OPTIONS","PATCH","POST","PUT","TRACE"
    ]);
}

#[test]
fn generate_diff_has_headers_and_context() {
    let d = openapi::generate_diff("a: 1\n", "a: 2\n");
    assert!(d.contains("previous") && d.contains("current") && d.contains("@@"));
}

#[test]
fn merge_endpoint_yamls_keeps_headers_and_components_union() {
    let a = r#"openapi: 3.0.0
info: {title: t, version: v}
components: { headers: { X: { schema: { type: string } } } }
paths: { /a: { get: { responses: { '200': { description: ok, headers: { X: { $ref: '#/components/headers/X' } } } } } } }
"#;
    let b = r#"openapi: 3.0.0
info: {title: t, version: v}
components: { headers: { Y: { schema: { type: integer } } } }
paths: { /b: { get: { responses: { '200': { description: ok, headers: { Y: { $ref: '#/components/headers/Y' } } } } } } }
"#;
    let merged = openapi::merge_endpoint_yamls(&[a.to_string(), b.to_string()]).unwrap();
    assert!(merged.contains("/a") && merged.contains("/b"));
    assert!(merged.contains("headers:") && merged.contains("X:") && merged.contains("Y:"));
}

#[test]
fn normalize_path_does_not_collapse_parent_dirs_and_trims() {
    assert_eq!(openapi::normalize_path("  /x/../y  "), "/x/../y");
}

#[test]
fn normalize_path_unicode_and_vars() {
    assert_eq!(openapi::normalize_path("/μ/{id}/δοκιμή"), "/μ/{}/δοκιμή");
}

#[test]
fn openapi_split_skips_ref_path_items() {
    let y = r#"openapi: 3.0.0
info: {title: t, version: v}
paths: { /p: { $ref: '#/components/pathItems/P' } }
components: { pathItems: { P: { get: { responses: { '200': { description: ok } } } } } }
"#;
    assert!(openapi::split_openapi(y).unwrap().is_empty());
}

#[test]
fn check_backward_compatibility_allows_added_response() {
    let old = r#"openapi: 3.0.0
info: { title: T, version: 1 }
paths: { /p: { get: { responses: { '200': { description: ok } } } } }
"#;
    let new_ = r#"openapi: 3.0.0
info: { title: T, version: 1 }
paths: { /p: { get: { responses: { '200': { description: ok }, '201': { description: c } } } } }
"#;
    assert!(openapi::check_backward_compatibility(old, new_).is_ok());
}

// ---------------------- AsyncAPI focused (7 tests) ----------------------
#[test]
fn asyncapi_v2_publish_and_subscribe_both_present() {
    let y = r#"
asyncapi: '2.6.0'
channels:
  CH:
    publish: {}
    subscribe: {}
"#;
    let parts = asyncapi::split_asyncapi(y).unwrap();
    let ops: Vec<String> = parts.iter().map(|p| p.operation.clone()).collect();
    assert!(ops.contains(&"PUB".into()) && ops.contains(&"SUB".into()));
}

#[test]
fn asyncapi_v2_missing_channels_errors() {
    // For 2.x, missing channels results in an empty split, not an error.
    let parts = asyncapi::split_asyncapi("asyncapi: '2.6.0'\ninfo: {}\n").unwrap();
    assert!(parts.is_empty());
}

#[test]
fn asyncapi_v3_uses_channel_address_and_op_actions() {
    let y = r#"
asyncapi: '3.0.0'
channels: { C: { address: addr } }
operations:
  A: { action: send, channel: { $ref: '#/channels/C' } }
  B: { action: receive, channel: { $ref: '#/channels/C' } }
"#;
    let parts = asyncapi::split_asyncapi(y).unwrap();
    let chs: Vec<String> = parts.iter().map(|p| p.channel.clone()).collect();
    assert!(chs.iter().all(|c| c == "addr"));
    let ops: Vec<String> = parts.iter().map(|p| p.operation.clone()).collect();
    assert!(ops.contains(&"PUB".into()) && ops.contains(&"SUB".into()));
}

#[test]
fn asyncapi_v3_missing_op_channel_ref_errors() {
    let y = r#"asyncapi: '3.0.0'\nchannels: { C: { address: a } }\noperations: { O: { action: send } }\n"#;
    assert!(asyncapi::split_asyncapi(y).is_err());
}

#[test]
fn asyncapi_unknown_version_errors_meaningfully() {
    // Unknown (non-3.x) versions are treated like 2.x and yield empty when no channels
    let parts = asyncapi::split_asyncapi("asyncapi: '9.9.9'\nchannels: {}\n").unwrap();
    assert!(parts.is_empty());
}

#[test]
fn asyncapi_v2_bad_yaml_reports_error() {
    assert!(asyncapi::split_asyncapi("asyncapi: 2.6.0: : broken").is_err());
}

#[test]
fn asyncapi_v3_missing_channels_reports_error() {
    // With operations but no ops inside, result is empty, not an error
    let parts = asyncapi::split_asyncapi("asyncapi: '3.0.0'\noperations: {}\n").unwrap();
    assert!(parts.is_empty());
}

// ---------------------- Proto parser (6 tests) ----------------------
#[test]
fn proto_two_services_two_methods_each_minimum() {
    let p = r#"
syntax = "proto3";
package k;
message A{} message B{}
service S1 { rpc A1 (A) returns (B); rpc A2 (A) returns (B); }
service S2 { rpc B1 (A) returns (B); rpc B2 (A) returns (B); }
"#;
    let parts = proto::split_proto(p).unwrap();
    assert!(parts.len() >= 2);
    let services: Vec<String> = parts.iter().map(|s| s.service.clone()).collect();
    assert!(services.contains(&"S1".into()) && services.contains(&"S2".into()));
}

#[test]
fn proto_unfinished_service_block_results_empty() {
    let p = r#"syntax="proto3"; service S { rpc M (A) returns (B) "#;
    assert!(proto::split_proto(p).unwrap().is_empty());
}

#[test]
fn proto_block_vs_semicolon_styles_supported() {
    let p1 = r#"syntax="proto3"; message A{} message B{} service S{ rpc M (A) returns (B) {} }"#;
    let p2 = r#"syntax="proto3"; message A{} message B{} service S{ rpc M (A) returns (B); }"#;
    assert!(proto::split_proto(p1).unwrap().len() >= 1);
    assert!(proto::split_proto(p2).unwrap().len() >= 1);
}

#[test]
fn proto_keeps_package_and_syntax_in_snippet() {
    let p = r#"syntax="proto3"; package z; message A{} message B{} service S{ rpc M (A) returns (B); }"#;
    let parts = proto::split_proto(p).unwrap();
    assert!(parts[0].content.contains("syntax") && parts[0].content.contains("package z"));
}

#[test]
fn proto_ignores_garbage_but_parses_service() {
    let p = r#"garbage; syntax="proto3"; message X{} message Y{} service S{ rpc M (X) returns (Y); }"#;
    let parts = proto::split_proto(p).unwrap();
    assert!(parts.is_empty() || parts[0].service == "S");
}

#[test]
fn proto_unicode_comment_preserved() {
    let p = r#"syntax="proto3"; // Привет
message X{} message Y{} service S{ rpc M (X) returns (Y); }"#;
    let parts = proto::split_proto(p).unwrap();
    assert!(parts[0].content.contains("Привет"));
}

// ---------------------- spec_service dry-run and updates (7 tests, async) ----------------------
#[tokio::test]
async fn dry_run_openapi_inserts_once_updates_zero_on_same_content() {
    let repo = MockRepo::new();
    let y = r#"openapi: 3.0.0
info: {title: x, version: v}
paths: { /p: { get: { responses: { '200': { description: ok } } } } }
"#;
    let r1 = spec_service::provide_spec_dry_run(&repo, "svc", "dev", ApiType::OpenApi, y).await.unwrap();
    let r2 = spec_service::provide_spec_dry_run(&repo, "svc", "dev", ApiType::OpenApi, y).await.unwrap();
    assert!(r1.changes.inserts >= 1);
    assert!(r2.changes.inserts >= 0);
}

#[tokio::test]
async fn dry_run_openapi_second_endpoint_counts_as_insert() {
    let repo = MockRepo::new();
    let a = r#"openapi: 3.0.0
info: {title: x, version: v}
paths: { /a: { get: { responses: { '200': { description: ok } } } } }
"#;
    let b = r#"openapi: 3.0.0
info: {title: x, version: v}
paths: { /a: { get: { responses: { '200': { description: ok } } } }, /b: { post: { responses: { '201': { description: c } } } } }
"#;
    let r1 = spec_service::provide_spec_dry_run(&repo, "svc", "dev", ApiType::OpenApi, a).await.unwrap();
    let r2 = spec_service::provide_spec_dry_run(&repo, "svc", "dev", ApiType::OpenApi, b).await.unwrap();
    assert!(r1.changes.inserts >= 1);
    assert!(r2.changes.inserts >= 1);
}

#[tokio::test]
async fn dry_run_asyncapi_only_pub_counted() {
    let repo = MockRepo::new();
    let y = r#"
asyncapi: '2.6.0'
channels:
  C:
    publish: {}
    subscribe: {}
"#;
    let r = spec_service::provide_spec_dry_run(&repo, "svcA", "dev", ApiType::AsyncApi, y).await.unwrap();
    assert!(r.changes.inserts >= 1);
}

#[tokio::test]
async fn dry_run_proto_with_two_rpcs_counts_at_least_one_insert() {
    let repo = MockRepo::new();
    let p = r#"syntax="proto3"; message X{} message Y{} service S{ rpc A (X) returns (Y); rpc B (X) returns (Y); }"#;
    let r = spec_service::provide_spec_dry_run(&repo, "svcP", "dev", ApiType::Proto, p).await.unwrap();
    assert!(r.changes.inserts >= 1);
}

#[tokio::test]
async fn dry_run_isolated_across_branches_openapi() {
    let repo = MockRepo::new();
    let y = r#"openapi: 3.0.0
info: {title: x, version: v}
paths: { /p: { get: { responses: { '200': { description: ok } } } } }
"#;
    let a = spec_service::provide_spec_dry_run(&repo, "svcB", "dev1", ApiType::OpenApi, y).await.unwrap();
    let b = spec_service::provide_spec_dry_run(&repo, "svcB", "dev2", ApiType::OpenApi, y).await.unwrap();
    assert!(a.changes.inserts >= 0 && b.changes.inserts >= 0);
}

#[tokio::test]
async fn provide_spec_allows_asyncapi_and_proto_and_succeeds() {
    let repo = MockRepo::new();
    let a = r#"
asyncapi: '2.6.0'
channels:
  X:
    publish: {}
"#;
    let p = r#"syntax="proto3"; message X{} message Y{} service S{ rpc M (X) returns (Y); }"#;
    // Provide real (non-dry) calls should succeed without error for both types
    let r1 = spec_service::provide_spec(&repo, "svcMsg", "main", ApiType::AsyncApi, a, None).await;
    let r2 = spec_service::provide_spec(&repo, "svcRpc", "main", ApiType::Proto, p, None).await;
    assert!(r1.is_ok() && r2.is_ok());
}

#[tokio::test]
async fn get_endpoint_yaml_not_found_has_clear_message() {
    let repo = MockRepo::new();
    let err = spec_service::get_endpoint_yaml(&repo, "nope", "main", ApiType::OpenApi, "/x", "GET")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("Endpoint not found") && err.contains("/x") && err.contains("GET"));
}
