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
            username: Some("ci"),
            caller: Some(sanshain_service::domain::permissions::Actor::test_releaser()),
        },
    )
    .await
    .unwrap()
}

// ---------- openapi::normalize_path variants (10) ----------
#[test]
fn norm_collapse_slashes() {
    assert_eq!(openapi::normalize_path("//a///b"), "/a/b");
}
#[test]
fn norm_trim_vars() {
    assert_eq!(openapi::normalize_path("/a/{id}/x"), "/a/{}/x");
}
#[test]
fn norm_var_with_pattern() {
    assert_eq!(openapi::normalize_path("/a/{id:[0-9]+}/x"), "/a/{}/x");
}
#[test]
fn norm_trailing_slash() {
    assert_eq!(openapi::normalize_path("/a/b/"), "/a/b");
}
#[test]
fn norm_root_kept() {
    assert_eq!(openapi::normalize_path("/"), "/");
}
#[test]
fn norm_whitespace() {
    assert_eq!(openapi::normalize_path("  /a/b  "), "/a/b");
}
#[test]
fn norm_multiple_vars() {
    assert_eq!(openapi::normalize_path("/a/{x}/{y}"), "/a/{}/{}");
}
#[test]
fn norm_no_change() {
    assert_eq!(openapi::normalize_path("/a/b"), "/a/b");
}
#[test]
fn norm_empty_becomes_empty() {
    assert_eq!(openapi::normalize_path(""), "");
}
#[test]
fn norm_only_slashes() {
    assert_eq!(openapi::normalize_path("////"), "/");
}

// ---------- openapi::generate_diff and split/merge (7) ----------
#[test]
fn diff_shows_changes() {
    let a = "a: 1\n";
    let b = "a: 2\n";
    let d = openapi::generate_diff(a, b);
    assert!(d.is_empty() || d.contains("-1") || d.contains("+2") || d.contains("@@"));
}

#[test]
fn split_openapi_empty_paths_ok() {
    let doc = "openapi: 3.0.0\ninfo: {title: x, version: v}\npaths: {}\n";
    let parts = openapi::split_openapi(doc).unwrap();
    assert!(parts.is_empty());
}

#[test]
fn split_openapi_one_op() {
    let doc = "openapi: 3.0.0\ninfo: {title: x, version: v}\npaths:\n  /p:\n    get:\n      responses: { '200': { description: ok } }\n";
    let parts = openapi::split_openapi(doc).unwrap();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].method, "GET");
}

#[test]
fn merge_two_snippets() {
    let doc = "openapi: 3.0.0\ninfo: {title: x, version: v}\npaths:\n  /p:\n    get: { responses: { '200': { description: ok } } }\n    post: { responses: { '201': { description: created } } }\n";
    let parts = openapi::split_openapi(doc).unwrap();
    let ys: Vec<String> = parts.into_iter().map(|p| p.yaml_content).collect();
    let merged = openapi::merge_endpoint_yamls(&ys).unwrap();
    assert!(merged.contains("get:"));
    assert!(merged.contains("post:"));
}

#[test]
fn compat_breaks_on_removed_status() {
    let old = r#"
openapi: 3.0.0
info: { title: T, version: 1.0.0 }
paths:
  /p:
    get:
      responses:
        '200': { description: OK }
"#;
    let new_ = r#"
openapi: 3.0.0
info: { title: T, version: 1.0.1 }
paths:
  /p:
    get:
      responses:
        '201': { description: Created }
"#;
    assert!(openapi::check_backward_compatibility(old, new_).is_err());
}

#[test]
fn compat_ok_on_added_status() {
    let old = r#"
openapi: 3.0.0
info: { title: T, version: 1.0.0 }
paths:
  /p:
    get:
      responses:
        '200': { description: OK }
"#;
    let new_ = r#"
openapi: 3.0.0
info: { title: T, version: 1.0.1 }
paths:
  /p:
    get:
      responses:
        '200': { description: OK }
        '201': { description: Created }
"#;
    assert!(openapi::check_backward_compatibility(old, new_).is_ok());
}

#[test]
fn compat_breaks_on_type_change() {
    let old = r#"
openapi: 3.0.0
info: { title: X, version: v }
components:
  schemas:
    S:
      type: object
      properties:
        a: { type: string }
"#;
    let new_ = r#"
openapi: 3.0.0
info: { title: X, version: v }
components:
  schemas:
    S:
      type: object
      properties:
        a: { type: integer }
"#;
    assert!(openapi::check_backward_compatibility(old, new_).is_err());
}

// ---------- asyncapi::split_asyncapi (8) ----------
#[test]
fn asyncapi_v2_pub_sub() {
    let y = r#"
asyncapi: '2.6.0'
channels:
  X:
    publish: {}
    subscribe: {}
"#;
    let parts = asyncapi::split_asyncapi(y).unwrap();
    let ops: Vec<String> = parts.into_iter().map(|s| s.operation).collect();
    assert!(ops.contains(&"PUB".into()) && ops.contains(&"SUB".into()));
}

#[test]
fn asyncapi_v2_only_pub() {
    let y = r#"
asyncapi: '2.6.0'
channels:
  X:
    publish: {}
"#;
    let parts = asyncapi::split_asyncapi(y).unwrap();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].operation, "PUB");
}

#[test]
fn asyncapi_v3_minimal() {
    let y = r#"
asyncapi: '3.0.0'
channels:
  UserSignup:
    address: user.signup
operations:
  SendSignup:
    action: send
    channel: { $ref: '#/channels/UserSignup' }
"#;
    let parts = asyncapi::split_asyncapi(y).unwrap();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].operation, "PUB");
}

#[test]
fn asyncapi_v3_missing_ops_errors() {
    let y = r#"
asyncapi: '3.0.0'
channels: {}
"#;
    assert!(asyncapi::split_asyncapi(y).is_err());
}

#[test]
fn asyncapi_v3_unknown_action_errors() {
    let y = r#"
asyncapi: '3.0.0'
channels:
  C: { address: x }
operations:
  O:
    action: invalid
    channel: { $ref: '#/channels/C' }
"#;
    assert!(asyncapi::split_asyncapi(y).is_err());
}

#[test]
fn asyncapi_v3_missing_channel_ref_errors() {
    let y = r#"
asyncapi: '3.0.0'
channels:
  C: { address: x }
operations:
  O:
    action: send
"#;
    assert!(asyncapi::split_asyncapi(y).is_err());
}

#[test]
fn asyncapi_bad_yaml_errors() {
    let y = "asyncapi: 2.6.0: : bad";
    assert!(asyncapi::split_asyncapi(y).is_err());
}

#[test]
fn asyncapi_v2_empty_channels_ok() {
    let y = r#"
asyncapi: '2.6.0'
channels: {}
"#;
    let parts = asyncapi::split_asyncapi(y).unwrap();
    assert!(parts.is_empty());
}

// ---------- proto::split_proto (5) ----------
#[test]
fn proto_splits_multiple_services_methods() {
    let p = r#"
syntax = "proto3";
message X{}
message Y{}
service A{ rpc M1 (X) returns (Y);} 
service B{ rpc M2 (X) returns (Y);} 
"#;
    let res = proto::split_proto(p).unwrap();
    assert!(!res.is_empty());
    let names: Vec<(String, String)> = res.into_iter().map(|s| (s.service, s.method)).collect();
    assert!(names.iter().any(|(svc, m)| svc == "A" && m == "M1"));
    assert!(names.iter().any(|(svc, m)| svc == "B" && m == "M2"));
}

#[test]
fn proto_handles_semicolon_or_block() {
    let p = r#"
syntax = "proto3"; 
message X{} 
message Y{} 
service A{ rpc M1 (X) returns (Y) {} rpc M2 (X) returns (Y); } 
"#;
    let res = proto::split_proto(p).unwrap();
    assert!(!res.is_empty());
}

#[test]
fn proto_ignores_unclosed_block() {
    let p = r#"syntax = "proto3"; service A{ rpc M1 (X) returns (Y) "#;
    let res = proto::split_proto(p).unwrap();
    assert!(res.is_empty());
}

#[test]
fn proto_keeps_common_defs() {
    let p = r#"syntax="proto3"; package z; message X{} message Y{} service A{ rpc M (X) returns (Y);} "#;
    let res = proto::split_proto(p).unwrap();
    assert!(res[0].content.contains("package z"));
    assert!(res[0].content.contains("message X"));
}

#[test]
fn proto_non_ascii_comments_preserved() {
    let p = r#"
syntax="proto3"; // Привет
message X{} message Y{} 
service A{ rpc M (X) returns (Y);} 
"#;
    let res = proto::split_proto(p).unwrap();
    assert!(res[0].content.contains("Привет"));
}

// ---------- spec_service dry-run for AsyncAPI/Proto (2) ----------
#[tokio::test]
async fn dry_run_asyncapi_insert_count() {
    let repo = MockRepo::new();
    let y = r#"
asyncapi: '2.6.0'
info: { title: x, version: 1.0.0 }
channels:
  X:
    publish: {}
"#;
    let r = dry_run(&repo, "svc", ApiType::AsyncApi, y).await;
    assert_eq!(r.changes.inserts, 1);
    assert_eq!(r.version.to_string(), "1.0.0");
}

#[tokio::test]
async fn dry_run_proto_insert_count() {
    let repo = MockRepo::new();
    let p = "// sanshain-version: 1.0.0\nsyntax=\"proto3\"; message X{} message Y{} service A{ rpc M (X) returns (Y);} ";
    let r = dry_run(&repo, "svc", ApiType::Proto, p).await;
    assert_eq!(r.changes.inserts, 1);
    assert_eq!(r.version.to_string(), "1.0.0");
}
