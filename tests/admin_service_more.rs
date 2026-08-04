use sanshain_service::application::admin_service as admin;
use sanshain_service::application::mock_repo::MockRepo;
use sanshain_service::domain::models::{ApiType, AppError};
use sanshain_service::domain::ports::SpecRepository;

// 1. snapshot_max_age default is 30 (ADR-0003)
#[tokio::test]
async fn snapshot_max_age_default_30() {
    let repo = MockRepo::new();
    let d = admin::get_snapshot_max_age_days(&repo).await.unwrap();
    assert_eq!(d, 30);
}

// 2. set/get snapshot_max_age
#[tokio::test]
async fn snapshot_max_age_set_get() {
    let repo = MockRepo::new();
    admin::set_snapshot_max_age_days(&repo, 7).await.unwrap();
    let d = admin::get_snapshot_max_age_days(&repo).await.unwrap();
    assert_eq!(d, 7);
}

// 3. cleanup_expired_snapshots returns 0 when days==0 (cleanup disabled)
#[tokio::test]
async fn cleanup_expired_snapshots_zero_days() {
    let repo = MockRepo::new();
    admin::set_snapshot_max_age_days(&repo, 0).await.unwrap();
    let deleted = admin::cleanup_expired_snapshots(&repo).await.unwrap();
    assert_eq!(deleted, 0);
}

// 4. dependency_max_age default is 30
#[tokio::test]
async fn dependency_max_age_default_30() {
    let repo = MockRepo::new();
    let d = admin::get_dependency_max_age_days(&repo).await.unwrap();
    assert_eq!(d, 30);
}

// 5. set/get dependency_max_age
#[tokio::test]
async fn dependency_max_age_set_get() {
    let repo = MockRepo::new();
    admin::set_dependency_max_age_days(&repo, 90).await.unwrap();
    let d = admin::get_dependency_max_age_days(&repo).await.unwrap();
    assert_eq!(d, 90);
}

// 6. cleanup_stale_dependencies returns 0 when days==0
#[tokio::test]
async fn cleanup_stale_dependencies_zero_days() {
    let repo = MockRepo::new();
    admin::set_dependency_max_age_days(&repo, 0).await.unwrap();
    let deleted = admin::cleanup_stale_dependencies(&repo).await.unwrap();
    assert_eq!(deleted, 0);
}

// 7. delete non-existing entities: service/client report false
#[tokio::test]
async fn delete_nonexisting_returns_false() {
    let repo = MockRepo::new();
    assert!(!admin::delete_producer(&repo, "nope").await.unwrap());
    assert!(!admin::delete_consumer(&repo, "nope").await.unwrap());
}

// 8. delete_version of an unknown Producer or version is NotFound
#[tokio::test]
async fn delete_version_unknown_is_not_found() {
    let repo = MockRepo::new();
    let v = "1.0.0".parse().unwrap();
    let err = admin::delete_version(&repo, "nope", ApiType::OpenApi, v)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));

    repo.ensure_service("svc").await.unwrap();
    let err = admin::delete_version(&repo, "svc", ApiType::OpenApi, v)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
}

// 9. list_consumer_endpoints empty
#[tokio::test]
async fn list_client_endpoints_empty() {
    let repo = MockRepo::new();
    let v = admin::list_consumer_endpoints(&repo, "c").await.unwrap();
    assert!(v.is_empty());
}

// 10. list_version_dependents for an unknown version is NotFound, and for a
// version nobody pinned it is empty
#[tokio::test]
async fn list_version_dependents_unknown_and_empty() {
    use sanshain_service::application::spec_service::{self, ProvideSpecParams};
    use sanshain_service::domain::models::Stability;

    let repo = MockRepo::new();
    let v = "1.2.0".parse().unwrap();
    let err = admin::list_version_dependents(&repo, "svc", ApiType::OpenApi, v)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));

    let spec = "openapi: 3.0.0\ninfo: {title: t, version: 1.2.0}\npaths:\n  /p:\n    get:\n      responses:\n        '200': { description: ok }\n";
    spec_service::provide_spec(
        &repo,
        ProvideSpecParams {
            producername: "svc",
            api_type: ApiType::OpenApi,
            content: spec,
            stability: Stability::Ga,
            dry_run: false,
            caller: Some(sanshain_service::domain::permissions::Actor::test_releaser()),
            expected_prior_hash: None,
        },
    )
    .await
    .unwrap();
    let dependents = admin::list_version_dependents(&repo, "svc", ApiType::OpenApi, v)
        .await
        .unwrap();
    assert!(dependents.is_empty());
}

// 11. delete_version frees the number and names the pinned Consumers
#[tokio::test]
async fn delete_version_returns_pinned_consumers() {
    use sanshain_service::application::spec_service::{
        self, ProvideSpecParams, RequireEndpointParams,
    };
    use sanshain_service::domain::models::Stability;

    let repo = MockRepo::new();
    let v = "1.0.0".parse().unwrap();
    let spec = "openapi: 3.0.0\ninfo: {title: t, version: 1.0.0}\npaths:\n  /p:\n    get:\n      responses:\n        '200': { description: ok }\n";
    spec_service::provide_spec(
        &repo,
        ProvideSpecParams {
            producername: "svc",
            api_type: ApiType::OpenApi,
            content: spec,
            stability: Stability::Ga,
            dry_run: false,
            caller: Some(sanshain_service::domain::permissions::Actor::test_releaser()),
            expected_prior_hash: None,
        },
    )
    .await
    .unwrap();
    spec_service::require_endpoint(
        &repo,
        RequireEndpointParams {
            consumername: "pinned-consumer",
            producername: "svc",
            version: v,
            api_type: ApiType::OpenApi,
            path: "/p",
            method: "GET",
        },
    )
    .await
    .unwrap();

    let dependents = admin::delete_version(&repo, "svc", ApiType::OpenApi, v)
        .await
        .unwrap();
    assert_eq!(dependents, vec!["pinned-consumer".to_string()]);

    // The entry is gone: deleting again is NotFound.
    let err = admin::delete_version(&repo, "svc", ApiType::OpenApi, v)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
}
