//! Service-seam coverage of the 2.0 spec service (ADR-0003) against the
//! in-memory `MockRepo`: parsing per API type, tag handling, bundle
//! resolution, blame history and the provide timeline.

use sanshain_service::application::mock_repo::MockRepo;
use sanshain_service::application::spec_service;
use sanshain_service::domain::models::*;
use sanshain_service::domain::ports::SpecRepository;

// Minimal AsyncAPI 2.x with one PUB and one SUB on different channels.
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

// Minimal Proto with two RPCs across two services and a version marker.
const PROTO: &str = r#"
syntax = "proto3";
// sanshain-version: 2.1.0
package test;

message Req {}
message Res {}

service AService { rpc DoA (Req) returns (Res); }
service BService { rpc DoB (Req) returns (Res); }
"#;

fn openapi(version: &str, paths: &str) -> String {
    format!("openapi: 3.0.3\ninfo:\n  title: T\n  version: {version}\npaths:\n{paths}")
}

fn one_endpoint(version: &str) -> String {
    openapi(
        version,
        "  /x:\n    get:\n      responses:\n        '200':\n          description: OK\n",
    )
}

fn provide_params<'a>(
    producer: &'a str,
    api_type: ApiType,
    content: &'a str,
    stability: Stability,
    dry_run: bool,
) -> spec_service::ProvideSpecParams<'a> {
    spec_service::ProvideSpecParams {
        producername: producer,
        api_type,
        content,
        stability,
        dry_run,
        caller: Some(if stability == Stability::Ga {
            sanshain_service::domain::permissions::Actor::test_releaser()
        } else {
            sanshain_service::domain::permissions::Actor::test_caller()
        }),
        expected_prior_hash: None,
    }
}

#[tokio::test]
async fn asyncapi_dry_run_counts_only_pub_and_persists_nothing() {
    let repo = MockRepo::new();
    let resp = spec_service::provide_spec(
        &repo,
        provide_params(
            "svc",
            ApiType::AsyncApi,
            ASYNCAPI_V2,
            Stability::Snapshot,
            true,
        ),
    )
    .await
    .unwrap();
    // Only the publish operation is considered (1 insert); SUB is skipped.
    assert_eq!(resp.version, SemVer::new(1, 0, 0));
    assert_eq!(resp.changes.inserts, 1);
    assert_eq!(resp.changes.updates, 0);
    assert_eq!(resp.changes.deletes, 0);
    // Dry-run persists neither the service nor a version-line entry.
    assert!(repo.list_producers().await.unwrap().is_empty());
    assert!(repo.spec_versions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn proto_provide_extracts_all_rpcs_and_the_marker_version() {
    let repo = MockRepo::new();
    let resp = spec_service::provide_spec(
        &repo,
        provide_params("svc", ApiType::Proto, PROTO, Stability::Snapshot, false),
    )
    .await
    .unwrap();
    assert_eq!(resp.version, SemVer::new(2, 1, 0));
    assert_eq!(resp.stability, Stability::Snapshot);
    assert_eq!(resp.changes.inserts, 2);

    let stored =
        spec_service::list_producer_endpoints(&repo, "svc", ApiType::Proto, SemVer::new(2, 1, 0))
            .await
            .unwrap();
    let mut services: Vec<&str> = stored.iter().map(|e| e.path.as_str()).collect();
    services.sort();
    assert_eq!(services, vec!["AService", "BService"]);
}

#[tokio::test]
async fn provide_persists_the_auto_tag() {
    let repo = MockRepo::new();
    let resp = spec_service::provide_spec(
        &repo,
        spec_service::ProvideSpecParams {
            producername: "orders",
            api_type: ApiType::AsyncApi,
            content: ASYNCAPI_V2,
            stability: Stability::Snapshot,
            dry_run: false,
            caller: Some(sanshain_service::domain::permissions::Actor::test_caller()),
            expected_prior_hash: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(resp.version, SemVer::new(1, 0, 0));

    let sid = repo.ensure_service("orders").await.unwrap();
    let svc_tags = {
        let map = repo.service_tags.lock().unwrap();
        map.get(&sid).cloned().unwrap_or_default()
    };
    assert_eq!(svc_tags, vec!["messaging".to_string()], "{svc_tags:?}");
}

#[tokio::test]
async fn require_bundle_with_no_endpoints_is_a_bad_request() {
    let repo = MockRepo::new();
    let params = spec_service::RequireBundleParams {
        consumername: "cli",
        producername: "svc",
        version: SemVer::new(1, 0, 0),
        api_type: ApiType::OpenApi,
        endpoints: &[],
    };
    let res = spec_service::require_bundle_dry_run(&repo, params).await;
    match res {
        Err(AppError::BadRequest(msg)) => {
            assert_eq!(msg, "A bundle needs at least one endpoint")
        }
        other => panic!("expected BadRequest, got {:?}", other),
    }
}

#[tokio::test]
async fn require_bundle_missing_endpoints_are_gone_and_listed() {
    let repo = MockRepo::new();
    spec_service::provide_spec(
        &repo,
        provide_params(
            "svc",
            ApiType::OpenApi,
            &one_endpoint("1.0.0"),
            Stability::Ga,
            false,
        ),
    )
    .await
    .unwrap();

    let eps = vec![
        ("/x".to_string(), "GET".to_string()),
        ("/y".to_string(), "POST".to_string()),
    ];
    let params = spec_service::RequireBundleParams {
        consumername: "cli",
        producername: "svc",
        version: SemVer::new(1, 0, 0),
        api_type: ApiType::OpenApi,
        endpoints: &eps,
    };
    let res = spec_service::require_bundle_dry_run(&repo, params).await;
    match res {
        Err(AppError::Gone(msg)) => {
            assert!(msg.contains("POST /y"), "missing endpoints listed: {msg}");
            assert!(
                !msg.contains("GET /x"),
                "present endpoint not listed: {msg}"
            );
        }
        other => panic!("expected Gone, got {:?}", other),
    }
}

#[tokio::test]
async fn require_unknown_producer_or_version_is_not_found() {
    let repo = MockRepo::new();
    let params = spec_service::RequireEndpointParams {
        consumername: "cli",
        producername: "ghost",
        version: SemVer::new(1, 0, 0),
        api_type: ApiType::OpenApi,
        path: "/x",
        method: "get",
    };
    match spec_service::require_endpoint_dry_run(&repo, params).await {
        Err(AppError::NotFound(msg)) => assert!(msg.contains("unknown"), "{msg}"),
        other => panic!("expected NotFound, got {:?}", other),
    }

    // Producer exists, the pinned version does not: still 404, immediately.
    let _ = repo.ensure_service("svc").await.unwrap();
    let params = spec_service::RequireEndpointParams {
        consumername: "cli",
        producername: "svc",
        version: SemVer::new(9, 9, 9),
        api_type: ApiType::OpenApi,
        path: "/x",
        method: "get",
    };
    match spec_service::require_endpoint_dry_run(&repo, params).await {
        Err(AppError::NotFound(msg)) => {
            assert!(msg.contains("9.9.9"), "{msg}");
            assert!(msg.contains("configuration error"), "{msg}");
        }
        other => panic!("expected NotFound, got {:?}", other),
    }
}

#[tokio::test]
async fn require_absent_endpoint_of_an_existing_version_is_gone() {
    let repo = MockRepo::new();
    spec_service::provide_spec(
        &repo,
        provide_params(
            "svc",
            ApiType::OpenApi,
            &one_endpoint("1.0.0"),
            Stability::Ga,
            false,
        ),
    )
    .await
    .unwrap();

    let params = spec_service::RequireEndpointParams {
        consumername: "cli",
        producername: "svc",
        version: SemVer::new(1, 0, 0),
        api_type: ApiType::OpenApi,
        path: "/nope",
        method: "get",
    };
    match spec_service::require_endpoint(&repo, params).await {
        Err(AppError::Gone(msg)) => {
            assert!(msg.contains("does not include GET /nope"), "{msg}")
        }
        other => panic!("expected Gone, got {:?}", other),
    }
    // A failed require records nothing.
    assert!(repo.dependencies.lock().unwrap().is_empty());
}

#[tokio::test]
async fn successful_require_records_the_pin_and_touches_last_required() {
    let repo = MockRepo::new();
    spec_service::provide_spec(
        &repo,
        provide_params(
            "svc",
            ApiType::OpenApi,
            &one_endpoint("1.0.0"),
            Stability::Snapshot,
            false,
        ),
    )
    .await
    .unwrap();

    let params = spec_service::RequireEndpointParams {
        consumername: "cli",
        producername: "svc",
        version: SemVer::new(1, 0, 0),
        api_type: ApiType::OpenApi,
        path: "/x",
        method: "get",
    };
    let res = spec_service::require_endpoint(&repo, params).await.unwrap();
    assert_eq!(res.state, ResolutionState::Served);
    assert_eq!(res.version, SemVer::new(1, 0, 0));
    assert_eq!(res.stability, Stability::Snapshot);
    assert!(res.yaml.contains("/x"));

    let deps = repo.dependencies.lock().unwrap().clone();
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].path, "/x");
    assert_eq!(deps[0].method, "GET");

    let versions = repo.spec_versions.lock().unwrap().clone();
    assert!(
        versions[0].last_required_at.is_some(),
        "the require must count for use-based expiry"
    );
}

#[tokio::test]
async fn get_endpoint_yaml_reports_unknown_absent_and_served() {
    let repo = MockRepo::new();

    // Nobody has the producer at all.
    let view = spec_service::get_endpoint_yaml(
        &repo,
        "svc",
        SemVer::new(1, 0, 0),
        ApiType::OpenApi,
        "/x",
        "get",
    )
    .await
    .unwrap();
    assert_eq!(view.state, ResolutionState::Unknown);
    assert!(view.yaml.is_none());

    spec_service::provide_spec(
        &repo,
        provide_params(
            "svc",
            ApiType::OpenApi,
            &one_endpoint("1.0.0"),
            Stability::Ga,
            false,
        ),
    )
    .await
    .unwrap();

    // The version exists but deliberately lacks the endpoint.
    let view = spec_service::get_endpoint_yaml(
        &repo,
        "svc",
        SemVer::new(1, 0, 0),
        ApiType::OpenApi,
        "/nope",
        "get",
    )
    .await
    .unwrap();
    assert_eq!(view.state, ResolutionState::Absent);
    assert_eq!(view.version, Some(SemVer::new(1, 0, 0)));
    assert_eq!(view.stability, Some(Stability::Ga));
    assert!(view.yaml.is_none());

    // Served.
    let view = spec_service::get_endpoint_yaml(
        &repo,
        "svc",
        SemVer::new(1, 0, 0),
        ApiType::OpenApi,
        "/x",
        "get",
    )
    .await
    .unwrap();
    assert_eq!(view.state, ResolutionState::Served);
    assert!(view.yaml.unwrap().contains("/x"));
}

#[tokio::test]
async fn list_versions_orders_newest_first_within_each_api_type() {
    let repo = MockRepo::new();
    for version in ["1.0.0", "1.2.0", "1.1.0"] {
        spec_service::provide_spec(
            &repo,
            provide_params(
                "svc",
                ApiType::OpenApi,
                &one_endpoint(version),
                Stability::Snapshot,
                false,
            ),
        )
        .await
        .unwrap();
    }

    let versions = spec_service::list_versions(&repo, "svc").await.unwrap();
    let listed: Vec<String> = versions.iter().map(|v| v.version.to_string()).collect();
    assert_eq!(listed, vec!["1.2.0", "1.1.0", "1.0.0"]);

    // An unknown producer simply has no versions.
    let versions = spec_service::list_versions(&repo, "ghost").await.unwrap();
    assert!(versions.is_empty());
}

#[tokio::test]
async fn diff_versions_produces_a_unified_diff_between_the_stored_documents() {
    let repo = MockRepo::new();
    let old = one_endpoint("1.0.0");
    let new = one_endpoint("1.1.0").replace("description: OK", "description: Fine");
    for content in [&old, &new] {
        spec_service::provide_spec(
            &repo,
            provide_params("svc", ApiType::OpenApi, content, Stability::Snapshot, false),
        )
        .await
        .unwrap();
    }

    let diff = spec_service::diff_versions(
        &repo,
        "svc",
        ApiType::OpenApi,
        SemVer::new(1, 0, 0),
        SemVer::new(1, 1, 0),
    )
    .await
    .unwrap();
    assert!(diff.contains("--- svc 1.0.0"), "{diff}");
    assert!(diff.contains("+++ svc 1.1.0"), "{diff}");
    assert!(diff.contains("-          description: OK"), "{diff}");
    assert!(diff.contains("+          description: Fine"), "{diff}");

    // Diffing against an unknown version is a NotFound, not an empty diff.
    let err = spec_service::diff_versions(
        &repo,
        "svc",
        ApiType::OpenApi,
        SemVer::new(1, 0, 0),
        SemVer::new(9, 9, 9),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)), "{err:?}");
}

#[tokio::test]
async fn endpoint_history_walks_introduction_removal_and_return() {
    let repo = MockRepo::new();
    // 1.0.0 introduces /x, 1.1.0 removes it, 1.2.0 brings it back reworded.
    spec_service::provide_spec(
        &repo,
        provide_params(
            "svc",
            ApiType::OpenApi,
            &one_endpoint("1.0.0"),
            Stability::Snapshot,
            false,
        ),
    )
    .await
    .unwrap();
    let without_x = openapi(
        "1.1.0",
        "  /other:\n    get:\n      responses:\n        '200':\n          description: OK\n",
    );
    spec_service::provide_spec(
        &repo,
        provide_params(
            "svc",
            ApiType::OpenApi,
            &without_x,
            Stability::Snapshot,
            false,
        ),
    )
    .await
    .unwrap();
    let reworded = one_endpoint("1.2.0").replace("description: OK", "description: Fine");
    spec_service::provide_spec(
        &repo,
        provide_params(
            "svc",
            ApiType::OpenApi,
            &reworded,
            Stability::Snapshot,
            false,
        ),
    )
    .await
    .unwrap();

    let history = spec_service::get_endpoint_history(&repo, "svc", ApiType::OpenApi, "/x", "get")
        .await
        .unwrap();
    assert_eq!(history.len(), 3);
    assert_eq!(history[0].version, SemVer::new(1, 0, 0));
    assert!(history[0].changed, "1.0.0 introduced the endpoint");
    assert!(history[0].yaml_content.is_some());
    assert_eq!(history[1].version, SemVer::new(1, 1, 0));
    assert!(history[1].changed, "1.1.0 removed the endpoint");
    assert!(
        history[1].yaml_content.is_none(),
        "the removing version stores nothing for it"
    );
    assert_eq!(history[2].version, SemVer::new(1, 2, 0));
    assert!(history[2].changed, "1.2.0 reintroduced the endpoint");
    assert!(
        history[2]
            .yaml_content
            .as_deref()
            .unwrap()
            .contains("description: Fine")
    );
}

#[tokio::test]
async fn snapshot_overwrite_by_another_actor_is_audited() {
    let repo = MockRepo::new();
    let original = one_endpoint("1.0.0");
    let mut params = provide_params(
        "svc",
        ApiType::OpenApi,
        &original,
        Stability::Snapshot,
        false,
    );
    params.caller =
        Some(sanshain_service::domain::permissions::Actor::test_releaser_named("alice"));
    spec_service::provide_spec(&repo, params).await.unwrap();

    let overwrite = one_endpoint("1.0.0").replace("description: OK", "description: Fine");
    let mut params = provide_params(
        "svc",
        ApiType::OpenApi,
        &overwrite,
        Stability::Snapshot,
        false,
    );
    params.caller = Some(sanshain_service::domain::permissions::Actor::test_releaser_named("bob"));
    spec_service::provide_spec(&repo, params).await.unwrap();

    let logs = repo.audit_logs.lock().unwrap().clone();
    let overwrites: Vec<&AuditLogEntry> = logs
        .iter()
        .filter(|l| l.action == "SNAPSHOT_OVERWRITTEN")
        .collect();
    assert_eq!(overwrites.len(), 1, "{logs:?}");
    assert_eq!(overwrites[0].username, "bob");
    assert!(
        overwrites[0].details.contains("alice"),
        "{:?}",
        overwrites[0]
    );
    assert_eq!(overwrites[0].version.as_deref(), Some("1.0.0"));
}

/// A same-content promotion is a release, not an overwrite: the audit trail
/// carries VERSION_PROMOTED and must not also claim a snapshot was clobbered.
#[tokio::test]
async fn a_promotion_is_not_audited_as_a_snapshot_overwrite() {
    let repo = MockRepo::new();
    let content = one_endpoint("1.0.0");
    let mut params = provide_params(
        "svc",
        ApiType::OpenApi,
        &content,
        Stability::Snapshot,
        false,
    );
    params.caller =
        Some(sanshain_service::domain::permissions::Actor::test_releaser_named("alice"));
    spec_service::provide_spec(&repo, params).await.unwrap();

    let mut params = provide_params("svc", ApiType::OpenApi, &content, Stability::Ga, false);
    params.caller =
        Some(sanshain_service::domain::permissions::Actor::test_releaser_named("jenkins"));
    let resp = spec_service::provide_spec(&repo, params).await.unwrap();
    assert!(resp.promoted, "the response must report the state change");
    assert_eq!(resp.stability, Stability::Ga);
    assert!(resp.changes.is_empty(), "byte-identical content");

    let logs = repo.audit_logs.lock().unwrap().clone();
    assert!(
        logs.iter().any(|l| l.action == "VERSION_PROMOTED"),
        "{logs:?}"
    );
    assert!(
        !logs.iter().any(|l| l.action == "SNAPSHOT_OVERWRITTEN"),
        "a promotion is not an overwrite: {logs:?}"
    );
}

/// The promote race (#28 review): a snapshot overwritten between read and
/// release must refuse instead of going GA with stale bytes.
#[tokio::test]
async fn a_promote_with_a_stale_content_hash_conflicts() {
    let repo = MockRepo::new();
    let content = one_endpoint("1.0.0");
    let mut params = provide_params(
        "svc",
        ApiType::OpenApi,
        &content,
        Stability::Snapshot,
        false,
    );
    params.caller =
        Some(sanshain_service::domain::permissions::Actor::test_releaser_named("alice"));
    spec_service::provide_spec(&repo, params).await.unwrap();

    // The releaser read the snapshot, then someone overwrote it: releasing
    // with the pre-overwrite hash must refuse.
    let mut params = provide_params("svc", ApiType::OpenApi, &content, Stability::Ga, false);
    params.expected_prior_hash = Some("sha256:stale");
    let err = spec_service::provide_spec(&repo, params).await.unwrap_err();
    match &err {
        AppError::Conflict(msg) => assert!(
            msg.contains("snapshot changed"),
            "the message names what actually happened: {msg}"
        ),
        other => panic!("expected Conflict, got {other:?}"),
    }

    let stability = repo.spec_versions.lock().unwrap()[0].stability;
    assert_eq!(stability, Stability::Snapshot, "nothing was released");
}

/// The CAS's third arm: a version deleted between read and release must not
/// be resurrected — and the message says so.
#[tokio::test]
async fn a_promote_of_a_vanished_version_conflicts_with_the_right_message() {
    let repo = MockRepo::new();
    // The service exists but the version does not (deleted after the read).
    repo.ensure_service("svc").await.unwrap();
    let content = one_endpoint("1.0.0");
    // Any asserted prior hash will do: the row is gone, so the INSERT arm
    // must refuse regardless of the value.
    let mut params = provide_params("svc", ApiType::OpenApi, &content, Stability::Ga, false);
    params.expected_prior_hash = Some("sha256:whatever-was-read");
    let err = spec_service::provide_spec(&repo, params).await.unwrap_err();
    match &err {
        AppError::Conflict(msg) => assert!(
            msg.contains("deleted while releasing"),
            "the message names the vanished row: {msg}"
        ),
        other => panic!("expected Conflict, got {other:?}"),
    }
    assert!(
        repo.spec_versions.lock().unwrap().is_empty(),
        "nothing was resurrected"
    );
}

/// An ordinary provide reports `promoted: false`.
#[tokio::test]
async fn a_plain_snapshot_provide_is_not_a_promotion() {
    let repo = MockRepo::new();
    let content = one_endpoint("1.0.0");
    let resp = spec_service::provide_spec(
        &repo,
        provide_params(
            "svc",
            ApiType::OpenApi,
            &content,
            Stability::Snapshot,
            false,
        ),
    )
    .await
    .unwrap();
    assert!(!resp.promoted);
}

/// The idempotent no-op still counts as "provided": a snapshot a CI
/// re-provides nightly must not age out of the use-based expiry.
#[tokio::test]
async fn an_identical_reprovide_refreshes_the_snapshot_provided_time() {
    let repo = MockRepo::new();
    let content = one_endpoint("1.0.0");
    spec_service::provide_spec(
        &repo,
        provide_params(
            "svc",
            ApiType::OpenApi,
            &content,
            Stability::Snapshot,
            false,
        ),
    )
    .await
    .unwrap();

    // Backdate the stored entry, as if the first provide were weeks old.
    let stale = "2020-01-01T00:00:00Z".to_string();
    {
        let mut versions = repo.spec_versions.lock().unwrap();
        versions[0].updated_at = stale.clone();
    }

    let resp = spec_service::provide_spec(
        &repo,
        provide_params(
            "svc",
            ApiType::OpenApi,
            &content,
            Stability::Snapshot,
            false,
        ),
    )
    .await
    .unwrap();
    assert!(resp.changes.is_empty(), "still an idempotent no-op");

    let updated_at = repo.spec_versions.lock().unwrap()[0].updated_at.clone();
    assert_ne!(
        updated_at, stale,
        "the no-op must refresh the provided time so expiry counts it"
    );
}

#[tokio::test]
async fn provide_timeline_lists_newest_provides_with_endpoint_counts() {
    let repo = MockRepo::new();
    spec_service::provide_spec(
        &repo,
        provide_params(
            "alpha",
            ApiType::OpenApi,
            &one_endpoint("1.0.0"),
            Stability::Ga,
            false,
        ),
    )
    .await
    .unwrap();
    spec_service::provide_spec(
        &repo,
        provide_params("beta", ApiType::Proto, PROTO, Stability::Snapshot, false),
    )
    .await
    .unwrap();

    let timeline = spec_service::get_provide_timeline(&repo, 10).await.unwrap();
    assert_eq!(timeline.len(), 2);
    let services: Vec<&str> = timeline.iter().map(|t| t.service.as_str()).collect();
    assert!(services.contains(&"alpha") && services.contains(&"beta"));
    let beta = timeline.iter().find(|t| t.service == "beta").unwrap();
    assert_eq!(beta.api_type, ApiType::Proto);
    assert_eq!(beta.version, SemVer::new(2, 1, 0));
    assert_eq!(beta.stability, Stability::Snapshot);
    assert_eq!(beta.endpoint_count, 2);

    // The limit truncates.
    let timeline = spec_service::get_provide_timeline(&repo, 1).await.unwrap();
    assert_eq!(timeline.len(), 1);
}
