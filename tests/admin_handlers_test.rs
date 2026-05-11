use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::Utc;
use sanshain_service::application::services;
use sanshain_service::domain::models::ApiType;
use sanshain_service::infrastructure::cached_repository::CachedSpecRepository;
use sanshain_service::infrastructure::database::DatabaseRepo;
use sanshain_service::infrastructure::sqlite_repository::SqliteSpecRepository;
use sanshain_service::{AppState, create_app};
use sqlx::sqlite::SqlitePoolOptions;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64};
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

fn test_state(repo: SqliteSpecRepository) -> AppState {
    let (spec_updated_tx, _) = tokio::sync::broadcast::channel(16);
    AppState {
        repo: CachedSpecRepository::new(DatabaseRepo::Sqlite(repo), 64),
        db_url: "sqlite::memory:".into(),
        csrf_tokens: Arc::new(RwLock::new(HashMap::new())),
        instance_id: "test".into(),
        spec_updated_tx,
        error_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
        warn_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
        info_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
        debug_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
        business_logic_debug: Arc::new(AtomicBool::new(false)),
        admin_user_debug: Arc::new(AtomicBool::new(false)),
        requests_total: Arc::new(AtomicU64::new(0)),
        failures_total: Arc::new(AtomicU64::new(0)),
        process_start_time: Utc::now(),
        prometheus_handle: get_test_prometheus_handle(),
        system: Arc::new(std::sync::Mutex::new(sysinfo::System::new_all())),
    }
}

async fn app_with_seed() -> (axum::Router, SqliteSpecRepository, String) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    // Seed initial admin and get token
    unsafe {
        std::env::set_var("INITIAL_ADMIN_USERNAME", "root");
        std::env::set_var("INITIAL_ADMIN_PASSWORD", "root_password");
    }
    services::ensure_initial_admin(&repo).await.unwrap();
    let token = services::login(&repo, "root", "root_password")
        .await
        .unwrap();

    // Seed a demo service and endpoint via provide
    let openapi = r#"openapi: 3.0.0
info: {title: demo, version: v1}
paths:
  /hello:
    get:
      responses:
        '200': { description: ok }
"#;
    let _ = services::provide_spec(
        &repo,
        "demo-svc",
        "main",
        ApiType::OpenApi,
        openapi,
        None,
        false,
    )
    .await
    .unwrap();

    let app = create_app(test_state(repo.clone()));
    (app, repo, token.token)
}

#[tokio::test]
async fn admin_lists_and_cache_endpoints_work() {
    let (app, _repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    // List services (admin authenticated route)
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/services")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // List service endpoints
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/services/demo-svc/branches/main/endpoints")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Cache stats
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/settings/cache")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Clear cache
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/cache/clear")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}
