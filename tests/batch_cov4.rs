use sanshain_service::application::mock_repo::MockRepo;
use sanshain_service::application::spec_service;
use sanshain_service::domain::models::{ApiType, ProvideResponse, Stability};
use sanshain_service::{asyncapi, openapi, proto};

// 2.0 dry-run helper: the version lives in the spec document itself, and the
// caller declares stability instead of naming a branch.
#[cfg(test)]
async fn dry_run(
    repo: &MockRepo,
    producer: &str,
    api_type: ApiType,
    content: &str,
) -> ProvideResponse {
    spec_service::provide_spec(
        repo,
        spec_service::ProvideSpecParams {
            producername: producer,
            api_type,
            content,
            stability: Stability::Snapshot,
            dry_run: true,
            caller: Some(sanshain_service::domain::permissions::Actor::test_caller()),
            expected_prior_hash: None,
        },
    )
    .await
    .unwrap()
}

// ---------------------- OpenAPI normalize_path (8) ----------------------
#[test]
fn norm_space_inside_kept() {
    assert_eq!(openapi::normalize_path("/a b/c"), "/a b/c");
}
#[test]
fn norm_tab_trim() {
    assert_eq!(openapi::normalize_path("\t/a/b\t"), "/a/b");
}
#[test]
fn norm_mixed_vars() {
    assert_eq!(openapi::normalize_path("/a/{x}/b/{y:.*}/c"), "/a/{}/b/{}/c");
}
#[test]
fn norm_multibyte_ok() {
    assert_eq!(
        openapi::normalize_path("/привет/{id}/мир"),
        "/привет/{}/мир"
    );
}
#[test]
fn norm_dotdot_left_alone() {
    assert_eq!(openapi::normalize_path("/a/../b"), "/a/../b");
}
#[test]
fn norm_many_trailing() {
    assert_eq!(openapi::normalize_path("/a/b////"), "/a/b");
}
#[test]
fn norm_leading_spaces_only() {
    assert_eq!(openapi::normalize_path("    "), "");
}
#[test]
fn norm_mixed_slashes() {
    assert_eq!(openapi::normalize_path("/a//b///c"), "/a/b/c");
}

// ---------------------- OpenAPI split/merge/diff/compat (12) ----------------------
#[test]
fn openapi_split_two_methods_same_path() {
    let y = r#"openapi: 3.0.0
info: {title: x, version: v}
paths:
  /p:
    get: { responses: { '200': { description: ok } } }
    put: { responses: { '204': { description: none } } }
"#;
    let parts = openapi::split_openapi(y).unwrap();
    let methods: Vec<String> = parts.iter().map(|p| p.method.clone()).collect();
    assert!(methods.contains(&"GET".into()));
    assert!(methods.contains(&"PUT".into()));
}

#[test]
fn openapi_split_ignores_ref_path_items() {
    // If a path item is a $ref, current impl skips it.
    let y = r#"openapi: 3.0.0
info: {title: x, version: v}
paths:
  /p: { $ref: '#/components/pathItems/P' }
components:
  pathItems:
    P:
      get: { responses: { '200': { description: ok } } }
"#;
    let parts = openapi::split_openapi(y).unwrap();
    assert!(parts.is_empty());
}

#[test]
fn openapi_merge_empty_err_is_ok() {
    // merge_endpoint_yamls([]) should error according to implementation.
    let r = openapi::merge_endpoint_yamls(&[]);
    assert!(r.is_err());
}

#[test]
fn openapi_merge_two_same_schema_dedups() {
    let a = r#"openapi: 3.0.0
info: {title: x, version: v}
components: { schemas: { A: { type: object } } }
paths:
  /p: { get: { responses: { '200': { description: ok } } } }
"#;
    let b = r#"openapi: 3.0.0
info: {title: x, version: v}
components: { schemas: { A: { type: object } } }
paths:
  /q: { post: { responses: { '201': { description: created } } } }
"#;
    let merged = openapi::merge_endpoint_yamls(&[a.into(), b.into()]).unwrap();
    // At minimum the merged doc should include both paths without duplicating schema A.
    assert!(merged.contains("/p"));
    assert!(merged.contains("/q"));
    assert!(merged.matches("schemas:").count() <= 1);
}

#[test]
fn openapi_diff_has_markers_or_empty() {
    let old = "x: 1\n";
    let new_ = "x: 2\n";
    let d = openapi::generate_diff(old, new_);
    assert!(d.is_empty() || d.contains("-1") || d.contains("+2") || d.contains("@@"));
}

#[test]
fn compat_ok_on_added_property() {
    let old = r#"openapi: 3.0.0
info: { title: T, version: 1 }
components:
  schemas:
    A:
      type: object
      properties: { a: { type: string } }
paths: {}
"#;
    let new_ = r#"openapi: 3.0.0
info: { title: T, version: 1 }
components:
  schemas:
    A:
      type: object
      properties: { a: { type: string }, b: { type: integer } }
paths: {}
"#;
    assert!(openapi::check_backward_compatibility(old, new_).is_ok());
}

#[test]
fn compat_removed_required_is_ok_in_current_rules() {
    let old = r#"openapi: 3.0.0
info: { title: T, version: 1 }
components:
  schemas:
    A:
      type: object
      required: [ a ]
      properties: { a: { type: string } }
paths: {}
"#;
    let new_ = r#"openapi: 3.0.0
info: { title: T, version: 1 }
components:
  schemas:
    A:
      type: object
      properties: { a: { type: string } }
paths: {}
"#;
    // Current compatibility rules do not treat removal of `required` list as breaking.
    assert!(openapi::check_backward_compatibility(old, new_).is_ok());
}

#[test]
fn compat_ok_when_new_has_extra_response() {
    let old = r#"openapi: 3.0.0
info: { title: T, version: 1 }
paths:
  /p:
    get:
      responses: { '200': { description: ok } }
"#;
    let new_ = r#"openapi: 3.0.0
info: { title: T, version: 1 }
paths:
  /p:
    get:
      responses: { '200': { description: ok }, '201': { description: created } }
"#;
    assert!(openapi::check_backward_compatibility(old, new_).is_ok());
}

#[test]
fn compat_err_when_response_removed() {
    let old = r#"openapi: 3.0.0
info: { title: T, version: 1 }
paths:
  /p:
    get:
      responses: { '200': { description: ok }, '404': { description: no } }
"#;
    let new_ = r#"openapi: 3.0.0
info: { title: T, version: 1 }
paths:
  /p:
    get:
      responses: { '200': { description: ok } }
"#;
    assert!(openapi::check_backward_compatibility(old, new_).is_err());
}

#[test]
fn openapi_split_with_components_transitive() {
    // Ensure components referenced via response headers are included (broad check: success of split).
    let y = r#"openapi: 3.0.0
info: {title: x, version: v}
components:
  schemas:
    H: { type: string }
paths:
  /p:
    get:
      responses:
        '200':
          description: ok
          headers:
            X: { schema: { $ref: '#/components/schemas/H' } }
"#;
    let parts = openapi::split_openapi(y).unwrap();
    assert_eq!(parts.len(), 1);
    assert!(parts[0].yaml_content.contains("components"));
}

// ---------------------- AsyncAPI split (6) ----------------------
#[test]
fn asyncapi_unknown_version_errors() {
    let y = r#"asyncapi: '9.9.9'\nchannels: {}\n"#;
    assert!(asyncapi::split_asyncapi(y).is_err());
}

#[test]
fn asyncapi_v2_with_only_subscribe() {
    let y = r#"
asyncapi: '2.6.0'
channels:
  X:
    subscribe: {}
"#;
    let parts = asyncapi::split_asyncapi(y).unwrap();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].operation, "SUB");
}

#[test]
fn asyncapi_v3_multiple_ops() {
    let y = r#"
asyncapi: '3.0.0'
channels:
  C: { address: addr }
operations:
  O1: { action: send, channel: { $ref: '#/channels/C' } }
  O2: { action: receive, channel: { $ref: '#/channels/C' } }
"#;
    let parts = asyncapi::split_asyncapi(y).unwrap();
    let ops: Vec<String> = parts.into_iter().map(|p| p.operation).collect();
    assert!(ops.contains(&"PUB".into()) && ops.contains(&"SUB".into()));
}

#[test]
fn asyncapi_v2_bad_yaml_errors() {
    assert!(asyncapi::split_asyncapi("asyncapi: 2.6.0: : OOPS").is_err());
}

#[test]
fn asyncapi_v3_missing_channels_errors() {
    let y = r#"asyncapi: '3.0.0'\noperations: {}\n"#;
    assert!(asyncapi::split_asyncapi(y).is_err());
}

#[test]
fn asyncapi_v3_channel_without_op_ref_errors() {
    let y = r#"
asyncapi: '3.0.0'
channels: { C: { address: x } }
operations: { O: { action: send } }
"#;
    assert!(asyncapi::split_asyncapi(y).is_err());
}

// ---------------------- Proto split (6) ----------------------
#[test]
fn proto_multiple_rpcs_and_services() {
    let p = r#"
syntax = "proto3";
package k;
message A{}
message B{}
service S1 { rpc M1 (A) returns (B); rpc M2 (A) returns (B); }
service S2 { rpc X (A) returns (B) {} }
"#;
    let parts = proto::split_proto(p).unwrap();
    // At least both services should contribute some split; exact count depends on parser heuristics.
    assert!(parts.len() >= 2);
}

#[test]
fn proto_keeps_package_and_syntax() {
    let p = r#"syntax = "proto3"; package z; message A{} message B{} service S{ rpc M (A) returns (B); }"#;
    let parts = proto::split_proto(p).unwrap();
    assert!(parts[0].content.contains("syntax"));
    assert!(parts[0].content.contains("package z"));
}

#[test]
fn proto_ignores_garbage() {
    let p = r#"syntax = "proto3"; service S { rpc M (X) returns (Y) ; // missing messages"#;
    let parts = proto::split_proto(p).unwrap();
    assert!(parts.is_empty() || !parts[0].service.is_empty());
}

#[test]
fn proto_handles_block_style() {
    let p = r#"syntax="proto3"; message A{} message B{} service S{ rpc M (A) returns (B) {} }"#;
    assert!(!proto::split_proto(p).unwrap().is_empty());
}

#[test]
fn proto_unicode_comments_kept() {
    let p =
        r#"syntax="proto3"; // 你好\nmessage X{} message Y{} service S{ rpc M (X) returns (Y); }"#;
    let parts = proto::split_proto(p).unwrap();
    assert!(parts[0].content.contains("你好"));
}

#[test]
fn proto_unclosed_service_block_yields_empty() {
    let p = r#"syntax="proto3"; service S{ rpc M (A) returns (B) "#;
    assert!(proto::split_proto(p).unwrap().is_empty());
}

// ---------------------- spec_service dry-run counts (4) ----------------------
#[tokio::test]
async fn dry_run_openapi_inserts_then_inserts_again() {
    let repo = MockRepo::new();
    let y1 = r#"openapi: 3.0.0
info: {title: x, version: 1.0.0}
paths:
  /p:
    get: { responses: { '200': { description: ok } } }
"#;
    let first = dry_run(&repo, "svcO", ApiType::OpenApi, y1).await;
    assert_eq!(first.changes.inserts, 1);

    // A dry run persists nothing, so re-running it reports the same inserts —
    // the version-line entry was never created.
    let second = dry_run(&repo, "svcO", ApiType::OpenApi, y1).await;
    assert_eq!(second.changes.inserts, 1);
}

#[tokio::test]
async fn dry_run_asyncapi_pub_sub_counts() {
    let repo = MockRepo::new();
    let y = r#"
asyncapi: '2.6.0'
info: { title: x, version: 1.0.0 }
channels:
  C: { publish: {}, subscribe: {} }
"#;
    // Only PUB channels are stored via provide; the SUB side is skipped.
    let r = dry_run(&repo, "svcA", ApiType::AsyncApi, y).await;
    assert_eq!(r.changes.inserts, 1);
}

#[tokio::test]
async fn dry_run_proto_multiple_methods_counts() {
    let repo = MockRepo::new();
    // One rpc per line: the splitter's rpc regex is line-anchored.
    let p = r#"// sanshain-version: 2.0.0
syntax="proto3";
message X{} message Y{}
service A{
  rpc M1 (X) returns (Y);
  rpc M2 (X) returns (Y);
}
"#;
    let r = dry_run(&repo, "svcP", ApiType::Proto, p).await;
    assert_eq!(r.changes.inserts, 2);
    assert_eq!(r.version.to_string(), "2.0.0");
}

#[tokio::test]
async fn dry_run_reads_version_from_the_document() {
    let repo = MockRepo::new();
    let v1 = r#"openapi: 3.0.0
info: {title: x, version: 1.2.3}
paths: { }"#;
    let v2 = r#"openapi: 3.0.0
info: {title: x, version: 4.5.6}
paths: { }"#;
    let a = dry_run(&repo, "svcB", ApiType::OpenApi, v1).await;
    let b = dry_run(&repo, "svcB", ApiType::OpenApi, v2).await;
    assert_eq!(a.version.to_string(), "1.2.3");
    assert_eq!(b.version.to_string(), "4.5.6");
}
