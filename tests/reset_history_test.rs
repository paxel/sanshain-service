use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use sanshain_service::application::services;
use sanshain_service::domain::models::*;
use sanshain_service::domain::ports::SpecRepository;
use sanshain_service::infrastructure::cached_repository::CachedSpecRepository;
use sanshain_service::infrastructure::database::DatabaseRepo;
use sanshain_service::infrastructure::sqlite_repository::SqliteSpecRepository;
use sanshain_service::{AppState, create_app};
use sqlx::sqlite::SqlitePoolOptions;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;
use tower::ServiceExt;

static TEST_PROMETHEUS_HANDLE: OnceLock<metrics_exporter_prometheus::PrometheusHandle> =
    OnceLock::new();

fn get_test_prometheus_handle() -> metrics_exporter_prometheus::PrometheusHandle {
    TEST_PROMETHEUS_HANDLE
        .get_or_init(|| {
            let (_, handle) = axum_prometheus::PrometheusMetricLayer::pair();
            handle
        })
        .clone()
}

fn test_app_state(repo: SqliteSpecRepository) -> AppState {
    let (spec_updated_tx, _) = tokio::sync::broadcast::channel(100);

    AppState {
        repo: CachedSpecRepository::new(DatabaseRepo::Sqlite(repo), 256),
        db_url: "sqlite::memory:".to_string(),
        dev_user: None,
        csrf_tokens: Arc::new(RwLock::new(HashMap::new())),
        instance_id: "test".to_string(),
        spec_updated_tx,
        error_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
        warn_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
        info_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
        debug_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
        business_logic_debug: Arc::new(AtomicBool::new(false)),
        admin_user_debug: Arc::new(AtomicBool::new(false)),
        requests_total: Arc::new(AtomicU64::new(0)),
        failures_total: Arc::new(AtomicU64::new(0)),
        process_start_time: chrono::Utc::now(),
        prometheus_handle: get_test_prometheus_handle(),
        system: Arc::new(std::sync::Mutex::new(sysinfo::System::new_all())),
        max_body_bytes: sanshain_service::DEFAULT_MAX_BODY_BYTES,
    }
}

#[tokio::test]
async fn test_reset_branch_history_repository() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();

    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    let service_name = "test-service";
    let branch_name = "master"; // Protected by default

    let service_id = repo.ensure_service(service_name).await.unwrap();
    let branch_id = repo.ensure_branch(service_id, branch_name).await.unwrap();

    // 1. Upload initial version
    let yaml1 = "openapi: 3.0.0\ninfo:\n  title: Test\n  version: 1.0.0\npaths:\n  /test:\n    get:\n      responses:\n        '200':\n          description: OK";
    services::provide_spec(
        &repo,
        service_name,
        branch_name,
        ApiType::OpenApi,
        yaml1,
        None,
        false,
    )
    .await
    .unwrap();

    // 2. Upload version 2
    let yaml2 = "openapi: 3.0.0\ninfo:\n  title: Test\n  version: 1.1.0\npaths:\n  /test:\n    get:\n      responses:\n        '200':\n          description: OK UPDATED";
    services::provide_spec(
        &repo,
        service_name,
        branch_name,
        ApiType::OpenApi,
        yaml2,
        None,
        false,
    )
    .await
    .unwrap();

    // Verify we have 2 versions
    let (v, _hash) = repo
        .get_spec_version(service_id, branch_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(v, SemVer::new(1, 0, 1)); // Two patch versions

    let endpoints = repo.get_endpoints_for_branch(branch_id).await.unwrap();
    assert_eq!(endpoints.len(), 1);
    let endpoint_id = endpoints[0].id.unwrap();

    let versions = repo.get_endpoint_versions(endpoint_id).await.unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].version, 1);
    assert_eq!(versions[1].version, 2);

    // 3. Add a dependency
    let client_id = repo.ensure_client("test-client").await.unwrap();
    repo.record_dependency(sanshain_service::domain::ports::RecordDependencyParams {
        client_id,
        endpoint_id: Some(endpoint_id),
        api_type: ApiType::OpenApi,
        service_id,
        branch_name,
        path: "/test",
        method: "GET",
    })
    .await
    .unwrap();

    // 4. Reset history
    repo.reset_branch_history(service_name, branch_name)
        .await
        .unwrap();

    // 5. Verify results
    // Branch version should be 1
    let (v_reset, _) = repo
        .get_spec_version(service_id, branch_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(v_reset, SemVer::new(1, 0, 0));

    // Endpoint should have only 1 version, and it should be version 1
    let versions_reset = repo.get_endpoint_versions(endpoint_id).await.unwrap();
    assert_eq!(versions_reset.len(), 1);
    assert_eq!(versions_reset[0].version, 1);
    assert!(versions_reset[0].yaml_content.contains("OK UPDATED"));
    assert_eq!(versions_reset[0].diff_from_previous, None);

    // Dependency should still exist
    let report = repo.get_report(branch_name).await.unwrap();
    assert_eq!(report.dependency_graph.len(), 1);
}

#[tokio::test]
async fn test_admin_reset_history_api() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();

    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    // Create admin user manually
    let hash = services::hash_password("admin-pass").unwrap();
    let user = repo.create_user("admin", &hash, true, true).await.unwrap();
    let session = repo
        .create_session(user.id, "2099-12-31T23:59:59")
        .await
        .unwrap();

    let service_name = "test-service";
    let branch_name = "master";

    // Provide a spec to create data
    let yaml = "openapi: 3.0.0\ninfo:\n  title: Test\n  version: 1.0.0\npaths:\n  /test:\n    get:\n      responses:\n        '200':\n          description: OK";
    services::provide_spec(
        &repo,
        service_name,
        branch_name,
        ApiType::OpenApi,
        yaml,
        None,
        false,
    )
    .await
    .unwrap();

    let app = create_app(test_app_state(repo.clone()));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/admin/services/{}/branches/{}/reset-history",
                    service_name, branch_name
                ))
                .header("Authorization", format!("Bearer {}", session.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Verify in repo
    let service_id = repo.find_service(service_name).await.unwrap().unwrap();
    let branch_id = repo
        .find_branch(service_id, branch_name)
        .await
        .unwrap()
        .unwrap();
    let (v, _) = repo
        .get_spec_version(service_id, branch_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(v, SemVer::new(1, 0, 0));
}
