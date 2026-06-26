use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use chrono::Utc;
use sanshain_service::application::services;
use sanshain_service::domain::models::AuthMode;
use sanshain_service::infrastructure::cached_repository::CachedSpecRepository;
use sanshain_service::infrastructure::database::DatabaseRepo;
use sanshain_service::infrastructure::postgres_repository::PostgresSpecRepository;
use sanshain_service::{AppState, create_app};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio::sync::RwLock;
use tower::ServiceExt;

const TEST_CSRF_TOKEN: &str = "test-csrf-token";

fn test_app_state(repo: PostgresSpecRepository, db_url: String) -> AppState {
    let mut tokens = HashMap::new();
    tokens.insert(
        TEST_CSRF_TOKEN.to_string(),
        Utc::now() + chrono::Duration::hours(1),
    );
    let (spec_updated_tx, _) = tokio::sync::broadcast::channel(100);

    let prometheus_handle = {
        let (_, handle) = axum_prometheus::PrometheusMetricLayer::pair();
        handle
    };

    AppState {
        repo: CachedSpecRepository::new(DatabaseRepo::Postgres(repo), 256),
        db_url,
        dev_user: None,
        csrf_tokens: Arc::new(RwLock::new(tokens)),
        instance_id: "test-postgres".to_string(),
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
        prometheus_handle,
        system: Arc::new(std::sync::Mutex::new(sysinfo::System::new_all())),
    }
}

#[tokio::test]
async fn test_postgres_full_flow_with_testcontainers() {
    // 1. Start Postgres container
    let postgres_container = Postgres::default().start().await.unwrap();
    let host = postgres_container.get_host().await.unwrap();
    let port = postgres_container.get_host_port_ipv4(5432).await.unwrap();
    let db_url = format!("postgres://postgres:postgres@{}:{}/postgres", host, port);

    // 2. Setup database and repo
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();

    let repo = PostgresSpecRepository::new(pool);
    repo.run_migrations()
        .await
        .expect("Failed to run PostgreSQL migrations");

    // 3. Setup app
    services::ensure_initial_admin(&repo).await.unwrap();
    services::set_auth_mode(&repo, &AuthMode::Dev)
        .await
        .unwrap();
    let dev_user = services::ensure_dev_user(&repo).await.ok();
    let mut state = test_app_state(repo.clone(), db_url);
    state.dev_user = dev_user;
    let app = create_app(state);

    // 4. Run simple flow (Provide -> Require)
    let openapi_yaml = r#"
openapi: 3.0.0
info:
  title: Postgres Test API
  version: 1.0.0
paths:
  /test:
    get:
      responses:
        '200':
          description: OK
"#;

    let provide_payload = json!({
        "servicename": "pg-service",
        "branch": "main",
        "openapi_yaml": openapi_yaml
    });

    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&provide_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::ACCEPTED);

    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require?clientname=pg-client&servicename=pg-service&branch=main&path=/test&method=GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("Postgres Test API"));
}
