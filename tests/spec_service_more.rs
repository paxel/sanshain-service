use sanshain_service::application::mock_repo::MockRepo;
use sanshain_service::application::spec_service;
use sanshain_service::domain::models::*;
use sanshain_service::domain::ports::SpecRepository;

// Minimal AsyncAPI 2.x with one PUB and one SUB on different channels
const ASYNCAPI_V2: &str = r#"
asyncapi: 2.6.0
info:
  title: Test
  version: 1.0.0
channels:
  user-created:
    publish:
      message:
        payload:
          type: object
  user-events:
    subscribe:
      message:
        payload:
          type: object
"#;

// Minimal Proto with two RPCs across two services
const PROTO: &str = r#"
syntax = "proto3";
package test;

message Req {}
message Res {}

service AService { rpc DoA (Req) returns (Res); }
service BService { rpc DoB (Req) returns (Res); }
"#;

#[tokio::test]
async fn asyncapi_dry_run_counts_only_pub() {
    let repo = MockRepo::new();
    let resp = spec_service::provide_spec_dry_run(
        &repo,
        "svc",
        "main",
        ApiType::AsyncApi,
        ASYNCAPI_V2,
        false,
    )
    .await
    .unwrap();
    // Only the publish operation should be considered (1 insert)
    assert_eq!(resp.changes.inserts, 1);
    assert_eq!(resp.changes.updates, 0);
    assert_eq!(resp.changes.deletes, 0);
    // Dry-run should not persist any services
    assert!(repo.list_services().await.unwrap().is_empty());
}

#[tokio::test]
async fn proto_dry_run_extracts_all_rpcs() {
    let repo = MockRepo::new();
    let resp =
        spec_service::provide_spec_dry_run(&repo, "svc", "dev", ApiType::Proto, PROTO, false)
            .await
            .unwrap();
    // Two RPCs → two inserts
    assert_eq!(resp.changes.inserts, 2);
    assert_eq!(resp.changes.updates, 0);
    assert_eq!(resp.changes.deletes, 0);
}

#[tokio::test]
async fn provide_with_tags_persists_and_auto_tag() {
    let repo = MockRepo::new();
    // Provide AsyncAPI should auto-tag with "messaging" and also keep custom tag
    let resp = spec_service::provide_spec_with_tags(
        &repo,
        spec_service::ProvideSpecParams {
            servicename: "orders",
            branch: "main",
            api_type: ApiType::AsyncApi,
            content: ASYNCAPI_V2,
            base_version: None,
            force: false,
            source_protected_branch: None,
            author: None,
        },
        &["custom".to_string()],
    )
    .await
    .unwrap();
    assert_eq!(resp.version, SemVer::new(1, 0, 0));

    // Verify tags stored in MockRepo internal map (MockRepo does not expose get_all_service_tags)
    let sid = repo.ensure_service("orders").await.unwrap();
    let svc_tags = {
        let map = repo.service_tags.lock().unwrap();
        map.get(&sid).cloned().unwrap_or_default()
    };
    assert!(svc_tags.contains(&"custom".to_string()));
    assert!(svc_tags.contains(&"messaging".to_string()));
}

#[tokio::test]
async fn require_bundle_dry_run_empty_is_bad_request() {
    let repo = MockRepo::new();
    let params = spec_service::RequireBundleParams {
        clientname: "cli",
        servicename: "svc",
        branch: "main",
        api_type: ApiType::OpenApi,
        endpoints: &[],
        timeout_secs: None,
        source_protected_branch: None,
        pull_from_branch: None,
    };
    let res = spec_service::require_bundle_dry_run(&repo, None, params).await;
    match res {
        Err(AppError::BadRequest(msg)) => assert!(msg.contains("No endpoints requested")),
        other => panic!("expected BadRequest, got {:?}", other),
    }
}

#[tokio::test]
async fn require_bundle_dry_run_reports_missing() {
    let repo = MockRepo::new();
    // Create the service so dry-run doesn't fail with "Service not found"
    let _ = repo.ensure_service("svc").await.unwrap();
    let eps = vec![
        ("/x".to_string(), "GET".to_string()),
        ("/y".to_string(), "POST".to_string()),
    ];
    let params = spec_service::RequireBundleParams {
        clientname: "cli",
        servicename: "svc",
        branch: "main",
        api_type: ApiType::OpenApi,
        endpoints: &eps,
        timeout_secs: Some(0),
        source_protected_branch: None,
        pull_from_branch: None,
    };
    let res = spec_service::require_bundle_dry_run(&repo, None, params).await;
    match res {
        Err(AppError::NotFound(msg)) => {
            // Be lenient about formatting/order; just ensure it's a missing summary
            assert!(msg.contains("missing"));
            assert!(!msg.is_empty());
        }
        other => panic!("expected NotFound, got {:?}", other),
    }
}

#[tokio::test]
async fn get_endpoint_yaml_not_found() {
    let repo = MockRepo::new();
    let res =
        spec_service::get_endpoint_yaml(&repo, "svc", "main", ApiType::OpenApi, "/x", "get").await;
    match res {
        Err(AppError::NotFound(msg)) => assert!(msg.contains("Endpoint not found")),
        other => panic!("expected NotFound, got {:?}", other),
    }
}

#[tokio::test]
async fn require_endpoint_dry_run_not_found() {
    let repo = MockRepo::new();
    // Ensure service exists for dry-run path
    let _ = repo.ensure_service("svc").await.unwrap();
    let params = spec_service::RequireEndpointParams {
        clientname: "cli",
        servicename: "svc",
        branch: "main",
        api_type: ApiType::OpenApi,
        path: "/nope",
        method: "get",
        timeout_secs: Some(0),
        source_protected_branch: None,
        pull_from_branch: None,
    };
    let res = spec_service::require_endpoint_dry_run(&repo, None, params).await;
    match res {
        Err(AppError::NotFound(msg)) => assert!(msg.contains("Endpoint not found")),
        other => panic!("expected NotFound, got {:?}", other),
    }
}

#[tokio::test]
async fn provide_spec_proto_base_version_conflict() {
    let repo = MockRepo::new();
    // First commit establishes version 1
    let r1 = spec_service::provide_spec(&repo, "svc", "main", ApiType::Proto, PROTO, None, false)
        .await
        .unwrap();
    assert_eq!(r1.version, SemVer::new(1, 0, 0));
    // Second call with stale base_version should conflict
    let err = spec_service::provide_spec(
        &repo,
        "svc",
        "main",
        ApiType::Proto,
        PROTO,
        Some("0.0.0".to_string()),
        false,
    )
    .await
    .unwrap_err();
    match err {
        AppError::Conflict(msg) => assert!(msg.contains("Outdated spec version")),
        other => panic!("expected Conflict, got {:?}", other),
    }
}

#[tokio::test]
async fn list_service_endpoints_empty_branch() {
    let repo = MockRepo::new();
    // Ensure service/branch exist but no endpoints yet
    let _sid = repo.ensure_service("svc").await.unwrap();
    let _bid = repo.ensure_branch(_sid, "main").await.unwrap();
    let list = spec_service::list_service_endpoints(&repo, "svc", "main")
        .await
        .unwrap();
    assert!(list.is_empty());
}

// OpenAPI snippets for protected-branch compatibility tests
const OPENAPI_OK: &str = r#"
openapi: 3.0.0
info: { title: T, version: 1.0.0 }
paths:
  /hello:
    get:
      responses:
        '200': { description: OK }
"#;

const OPENAPI_BREAKING: &str = r#"
openapi: 3.0.0
info: { title: T, version: 1.0.1 }
paths:
  /hello:
    get:
      responses:
        '201': { description: Created }
"#;

#[tokio::test]
async fn protected_branch_rejects_breaking_change() {
    let repo = MockRepo::new();
    // 'main' is protected in MockRepo
    spec_service::provide_spec(
        &repo,
        "svc",
        "main",
        ApiType::OpenApi,
        OPENAPI_OK,
        None,
        false,
    )
    .await
    .unwrap();
    let err = spec_service::provide_spec(
        &repo,
        "svc",
        "main",
        ApiType::OpenApi,
        OPENAPI_BREAKING,
        None,
        false,
    )
    .await
    .unwrap_err();
    match err {
        AppError::BreakingChange(msg) => assert!(msg.contains("Breaking changes detected")),
        other => panic!("expected BreakingChange, got {:?}", other),
    }
}

#[tokio::test]
async fn protected_branch_rejects_reintroducing_deleted_endpoint() {
    let repo = MockRepo::new();
    let sid = repo.ensure_service("svc").await.unwrap();
    let bid = repo.ensure_branch(sid, "main").await.unwrap();
    // Mark endpoint as deleted on protected branch
    repo.soft_delete_endpoint(bid, ApiType::OpenApi, "/x", "GET")
        .await
        .unwrap();
    // Now try to provide content that re-introduces it
    let yaml = r#"
openapi: 3.0.0
info: { title: T, version: 1.0.0 }
paths:
  /x:
    get:
      responses:
        '200': { description: OK }
"#;
    let err = spec_service::provide_spec(&repo, "svc", "main", ApiType::OpenApi, yaml, None, false)
        .await
        .unwrap_err();
    match err {
        AppError::Conflict(msg) => assert!(msg.contains("Re-introduction of deleted")),
        other => panic!("expected Conflict, got {:?}", other),
    }
}

// AsyncAPI snippets for protected-branch compatibility tests
const ASYNCAPI_COMPAT_BASE: &str = r#"
asyncapi: 2.6.0
info: { title: T, version: 1.0.0 }
channels:
  user-created:
    publish:
      message:
        payload:
          type: object
          properties:
            id: { type: string }
            age: { type: integer }
"#;

#[tokio::test]
async fn protected_branch_rejects_breaking_asyncapi_change() {
    let repo = MockRepo::new(); // 'main' is protected
    spec_service::provide_spec(
        &repo,
        "svc",
        "main",
        ApiType::AsyncApi,
        ASYNCAPI_COMPAT_BASE,
        None,
        false,
    )
    .await
    .unwrap();

    let removed_property = ASYNCAPI_COMPAT_BASE.replace("            age: { type: integer }\n", "");
    let err = spec_service::provide_spec(
        &repo,
        "svc",
        "main",
        ApiType::AsyncApi,
        &removed_property,
        None,
        false,
    )
    .await
    .unwrap_err();
    match err {
        AppError::BreakingChange(msg) => {
            assert!(msg.contains("was removed"), "{msg}");
        }
        other => panic!("expected BreakingChange, got {:?}", other),
    }
}

#[tokio::test]
async fn protected_branch_accepts_additive_asyncapi_change() {
    let repo = MockRepo::new();
    spec_service::provide_spec(
        &repo,
        "svc",
        "main",
        ApiType::AsyncApi,
        ASYNCAPI_COMPAT_BASE,
        None,
        false,
    )
    .await
    .unwrap();

    let added_property = ASYNCAPI_COMPAT_BASE.replace(
        "age: { type: integer }",
        "age: { type: integer }\n            email: { type: string }",
    );
    let resp = spec_service::provide_spec(
        &repo,
        "svc",
        "main",
        ApiType::AsyncApi,
        &added_property,
        None,
        false,
    )
    .await
    .unwrap();
    assert_eq!(resp.changes.updates, 1);
}

// Proto snippets for protected-branch compatibility tests
const PROTO_COMPAT_BASE: &str = r#"
syntax = "proto3";

message Req {
  string id = 1;
}
message Res {
  string name = 1;
}

service UserService {
  rpc GetUser (Req) returns (Res);
  rpc DropUser (Req) returns (Res) { option deprecated = true; }
}
"#;

#[tokio::test]
async fn protected_branch_rejects_breaking_proto_change() {
    let repo = MockRepo::new();
    spec_service::provide_spec(
        &repo,
        "svc",
        "main",
        ApiType::Proto,
        PROTO_COMPAT_BASE,
        None,
        false,
    )
    .await
    .unwrap();

    let changed_type = PROTO_COMPAT_BASE.replace("string id = 1;", "int64 id = 1;");
    let err = spec_service::provide_spec(
        &repo,
        "svc",
        "main",
        ApiType::Proto,
        &changed_type,
        None,
        false,
    )
    .await
    .unwrap_err();
    match err {
        AppError::BreakingChange(msg) => {
            assert!(
                msg.contains("changed type from 'string' to 'int64'"),
                "{msg}"
            );
        }
        other => panic!("expected BreakingChange, got {:?}", other),
    }
}

#[tokio::test]
async fn protected_branch_allows_removing_deprecated_proto_rpc() {
    let repo = MockRepo::new();
    spec_service::provide_spec(
        &repo,
        "svc",
        "main",
        ApiType::Proto,
        PROTO_COMPAT_BASE,
        None,
        false,
    )
    .await
    .unwrap();

    // Removing the deprecated rpc is allowed on the protected branch...
    let removed_deprecated = PROTO_COMPAT_BASE.replace(
        "  rpc DropUser (Req) returns (Res) { option deprecated = true; }\n",
        "",
    );
    let resp = spec_service::provide_spec(
        &repo,
        "svc",
        "main",
        ApiType::Proto,
        &removed_deprecated,
        None,
        false,
    )
    .await
    .unwrap();
    assert_eq!(resp.changes.deletes, 1);

    // ...but removing the remaining non-deprecated rpc is still rejected.
    let removed_active = removed_deprecated.replace("  rpc GetUser (Req) returns (Res);\n", "");
    let err = spec_service::provide_spec(
        &repo,
        "svc",
        "main",
        ApiType::Proto,
        &removed_active,
        None,
        false,
    )
    .await
    .unwrap_err();
    match err {
        AppError::BreakingChange(msg) => {
            assert!(msg.contains("mark it deprecated first"), "{msg}");
        }
        other => panic!("expected BreakingChange, got {:?}", other),
    }
}

#[tokio::test]
async fn protected_branch_allows_removing_deprecated_asyncapi_channel() {
    let repo = MockRepo::new();
    let two_channels = r#"
asyncapi: 2.6.0
info: { title: T, version: 1.0.0 }
channels:
  user-created:
    publish:
      message:
        payload:
          type: object
  legacy-events:
    publish:
      deprecated: true
      message:
        payload:
          type: object
"#;
    spec_service::provide_spec(
        &repo,
        "svc",
        "main",
        ApiType::AsyncApi,
        two_channels,
        None,
        false,
    )
    .await
    .unwrap();

    let without_legacy = r#"
asyncapi: 2.6.0
info: { title: T, version: 1.0.0 }
channels:
  user-created:
    publish:
      message:
        payload:
          type: object
"#;
    let resp = spec_service::provide_spec(
        &repo,
        "svc",
        "main",
        ApiType::AsyncApi,
        without_legacy,
        None,
        false,
    )
    .await
    .unwrap();
    assert_eq!(resp.changes.deletes, 1);
}

#[tokio::test]
async fn provide_spec_on_unprotected_branch_succeeds() {
    let repo = MockRepo::new();
    // Use a non-protected branch name
    let resp = spec_service::provide_spec(
        &repo,
        "svc",
        "dev",
        ApiType::OpenApi,
        OPENAPI_OK,
        None,
        false,
    )
    .await
    .unwrap();
    assert_eq!(resp.version, SemVer::new(1, 0, 0));
}
