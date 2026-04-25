use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;
use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::collections::{HashMap, VecDeque};
use tokio::sync::RwLock;
use chrono::Utc;
use tower::ServiceExt; // for `oneshot`
use sanshain_service::{create_app, AppState};
use sanshain_service::domain::models::ProvideResponse;
use sanshain_service::infrastructure::sqlite_repository::SqliteSpecRepository;
use sanshain_service::infrastructure::database::DatabaseRepo;
use sanshain_service::infrastructure::cached_repository::CachedSpecRepository;
use sanshain_service::application::services;

const TEST_CSRF_TOKEN: &str = "test-csrf-token";

static TEST_PROMETHEUS_HANDLE: OnceLock<metrics_exporter_prometheus::PrometheusHandle> = OnceLock::new();

fn get_test_prometheus_handle() -> metrics_exporter_prometheus::PrometheusHandle {
    TEST_PROMETHEUS_HANDLE.get_or_init(|| {
        let (_, handle) = axum_prometheus::PrometheusMetricLayer::pair();
        handle
    }).clone()
}

async fn setup_app() -> (axum::Router, SqliteSpecRepository) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();

    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    let mut tokens = HashMap::new();
    tokens.insert(TEST_CSRF_TOKEN.to_string(), Utc::now());
    let (spec_updated_tx, _) = tokio::sync::broadcast::channel(100);
    let state = AppState {
        repo: CachedSpecRepository::new(DatabaseRepo::Sqlite(repo.clone()), 256),
        db_url: "sqlite::memory:".to_string(),
        csrf_tokens: Arc::new(RwLock::new(tokens)),
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
        process_start_time: Utc::now(),
        prometheus_handle: get_test_prometheus_handle(),
    };
    let app = create_app(state);
    (app, repo)
}

/// Setup app with initial admin and dev mode enabled (for tests that don't care about auth)
async fn setup_app_dev_mode() -> axum::Router {
    let (app, repo) = setup_app().await;
    services::ensure_initial_admin(&repo).await.unwrap();
    services::set_dev_mode(&repo, true).await.unwrap();
    app
}

/// Setup app with initial admin, return app + admin token
async fn setup_app_with_admin() -> (axum::Router, String) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();

    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    // Create admin user manually with known password
    let hash = services::hash_password("admin-pass").unwrap();
    let user = repo.create_user("admin", &hash, true, true).await.unwrap();
    use sanshain_service::domain::ports::SpecRepository;
    let session = repo.create_session(user.id, "2099-12-31T23:59:59").await.unwrap();

    let mut tokens = HashMap::new();
    tokens.insert(TEST_CSRF_TOKEN.to_string(), Utc::now());
    let (spec_updated_tx, _) = tokio::sync::broadcast::channel(100);
    let state = AppState {
        repo: CachedSpecRepository::new(DatabaseRepo::Sqlite(repo), 256),
        db_url: "sqlite::memory:".to_string(),
        csrf_tokens: Arc::new(RwLock::new(tokens)),
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
        process_start_time: Utc::now(),
        prometheus_handle: get_test_prometheus_handle(),
    };
    let app = create_app(state);
    (app, session.token)
}

#[tokio::test]
async fn test_health_endpoint() {
    let (app, _) = setup_app().await;

    let response: Response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_root_page_banner_links_to_all_discovery_views() {
    let (app, _) = setup_app().await;

    let response: Response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1_000_000)
        .await
        .unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();

    assert!(html.contains("href=\"/service.html\" class=\"text-white hover:text-indigo-100 transition-colors\">Services</a>"));
    assert!(html.contains("href=\"/service.html#clients\" class=\"text-white hover:text-indigo-100 transition-colors\">Clients</a>"));
    assert!(html.contains("href=\"/service.html#graph\" class=\"text-white hover:text-indigo-100 transition-colors\">Graph</a>"));
    assert!(html.contains("href=\"/service.html#reports\" class=\"text-white hover:text-indigo-100 transition-colors\">Reports</a>"));
    assert!(html.contains("href=\"/admin.html\" class=\"text-white hover:text-indigo-100 transition-colors\">Admin</a>"));
}

#[tokio::test]
async fn test_license_file_is_served_from_root_path() {
    let (app, _) = setup_app().await;

    let response: Response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/LICENSE")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1_000_000)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("Apache License"));
}

#[tokio::test]
async fn test_full_flow() {
    let app = setup_app_dev_mode().await;

    // 1. Provide a spec
    let openapi_yaml = r#"
openapi: 3.0.0
info:
  title: Test API
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
"#;
    let provide_payload = json!({
        "servicename": "test-service",
        "branch": "main",
        "openapi_yaml": openapi_yaml
    });

    let response: Response = app.clone()
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

    // 2. Require an endpoint
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require?clientname=client-a&servicename=test-service&branch=main&path=/users&method=GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    
    // 3. Get report
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/report?branch=main")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000).await.unwrap();
    let report: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(report["dependency_graph"].as_array().unwrap().len(), 1);
    assert_eq!(report["unused_endpoints"].as_array().unwrap().len(), 0);
    assert_eq!(report["missing_endpoints"].as_array().unwrap().len(), 0);

    // 4. Get markdown report
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/report/markdown?branch=main")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers().get("content-type").unwrap(), "text/markdown; charset=utf-8");
    let body = axum::body::to_bytes(response.into_body(), 10000).await.unwrap();
    let md_report = String::from_utf8(body.to_vec()).unwrap();
    assert!(md_report.contains("# Sanshain Dependency Report: Branch `main`"));
    assert!(md_report.contains("| client-a | test-service | OpenApi | `/users` | `GET` |"));
}

#[tokio::test]
async fn test_require_bundle() {
    let app = setup_app_dev_mode().await;

    // Provide a spec with multiple endpoints sharing schemas
    let openapi_yaml = r#"
openapi: 3.0.0
info:
  title: Test API
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/User'
  /users:
    post:
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/User'
      responses:
        '201':
          description: Created
  /orders:
    get:
      responses:
        '200':
          description: OK
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Order'
components:
  schemas:
    User:
      type: object
      properties:
        name:
          type: string
    Order:
      type: object
      properties:
        user:
          $ref: '#/components/schemas/User'
        id:
          type: integer
"#;
    let provide_payload = json!({
        "servicename": "test-service",
        "branch": "main",
        "openapi_yaml": openapi_yaml
    });

    let response: Response = app.clone()
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

    // Require bundle with two endpoints
    let bundle_payload = json!({
        "clientname": "java-client",
        "servicename": "test-service",
        "branch": "main",
        "endpoints": [
            { "path": "/users", "method": "POST" },
            { "path": "/orders", "method": "GET" }
        ]
    });

    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/require-bundle")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&bundle_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 100000).await.unwrap();
    let merged_yaml = String::from_utf8(body.to_vec()).unwrap();

    // Both paths present in merged YAML
    assert!(merged_yaml.contains("/users"));
    assert!(merged_yaml.contains("/orders"));
    // Schemas deduplicated
    assert!(merged_yaml.contains("User"));
    assert!(merged_yaml.contains("Order"));

    // Parse and verify structure
    let parsed: openapiv3::OpenAPI = serde_yaml::from_str(&merged_yaml).unwrap();
    assert_eq!(parsed.paths.paths.len(), 2);
    let components = parsed.components.unwrap();
    assert_eq!(components.schemas.len(), 2);

    // Test missing endpoint returns error
    let bundle_missing = json!({
        "clientname": "java-client",
        "servicename": "test-service",
        "branch": "main",
        "endpoints": [
            { "path": "/users", "method": "GET" },
            { "path": "/nonexistent", "method": "DELETE" }
        ]
    });

    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/require-bundle")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&bundle_missing).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("DELETE /nonexistent"), "Error body should list the missing endpoint");

    // Test empty endpoints returns error
    let bundle_empty = json!({
        "clientname": "java-client",
        "servicename": "test-service",
        "branch": "main",
        "endpoints": []
    });

    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/require-bundle")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&bundle_empty).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_idempotency_and_conflict() {
    let app = setup_app_dev_mode().await;

    let openapi_v1 = r#"
openapi: 3.0.0
info:
  title: Test API
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
"#;
    let payload_v1 = json!({
        "servicename": "test-service",
        "branch": "main",
        "openapi_yaml": openapi_v1
    });

    // 1. First provide
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload_v1).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // 2. Second provide (identical) -> should be ACCEPTED (idempotent)
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload_v1).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // 3. Third provide (backward-compatible change) -> should be ACCEPTED
    let openapi_v1_compat = r#"
openapi: 3.0.0
info:
  title: Test API
  version: 1.0.0
paths:
  /users:
    get:
      description: Added description
      responses:
        '200':
          description: OK
"#;
    let payload_compat = json!({
        "servicename": "test-service",
        "branch": "main",
        "openapi_yaml": openapi_v1_compat
    });

    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload_compat).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // 3b. Breaking change (removed response code) -> should be CONFLICT
    let openapi_v1_breaking = r#"
openapi: 3.0.0
info:
  title: Test API
  version: 1.0.0
paths:
  /users:
    get:
      description: Added description
      responses:
        '200':
          description: OK
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/User'
components:
  schemas:
    User:
      type: object
      properties:
        name:
          type: string
"#;
    // First provide the version with schema
    let payload_with_schema = json!({
        "servicename": "test-service",
        "branch": "main",
        "openapi_yaml": openapi_v1_breaking
    });
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload_with_schema).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // Now try to change the type — breaking
    let openapi_v1_type_change = r#"
openapi: 3.0.0
info:
  title: Test API
  version: 1.0.0
paths:
  /users:
    get:
      description: Added description
      responses:
        '200':
          description: OK
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/User'
components:
  schemas:
    User:
      type: object
      properties:
        name:
          type: integer
"#;
    let payload_breaking = json!({
        "servicename": "test-service",
        "branch": "main",
        "openapi_yaml": openapi_v1_type_change
    });
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload_breaking).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);

    // 4. Fourth provide (new path added, existing /users unchanged) -> should be ACCEPTED
    let openapi_v2 = r#"
openapi: 3.0.0
info:
  title: Test API
  version: 1.0.0
paths:
  /users:
    get:
      description: Added description
      responses:
        '200':
          description: OK
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/User'
  /v2/users:
    get:
      responses:
        '200':
          description: OK
components:
  schemas:
    User:
      type: object
      properties:
        name:
          type: string
"#;
    let payload_v2 = json!({
        "servicename": "test-service",
        "branch": "main",
        "openapi_yaml": openapi_v2
    });

    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload_v2).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn test_protected_branches_api() {
    let (app, token) = setup_app_with_admin().await;

    // 1. List default protected branches
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/protected-branches")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000).await.unwrap();
    let branches: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert!(branches.contains(&"main".to_string()));
    assert!(branches.contains(&"master".to_string()));

    // 2. Add a protected branch
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/protected-branches")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({"pattern": "release"})).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // 3. Delete a protected branch
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/admin/protected-branches/release")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // 4. Delete non-existent -> 404
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/admin/protected-branches/nonexistent")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_feature_branch_allows_dto_update() {
    let app = setup_app_dev_mode().await;

    let yaml1 = r#"
openapi: 3.0.0
info:
  title: Test API
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
"#;
    let yaml2 = r#"
openapi: 3.0.0
info:
  title: Test API
  version: 1.0.0
paths:
  /users:
    get:
      description: Updated DTO
      responses:
        '200':
          description: OK
"#;

    // Provide on feature branch
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({
                    "servicename": "svc",
                    "branch": "feature/test",
                    "openapi_yaml": yaml1
                })).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // Update on feature branch (should succeed, not protected)
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({
                    "servicename": "svc",
                    "branch": "feature/test",
                    "openapi_yaml": yaml2
                })).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // Verify updated content
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require?clientname=client&servicename=svc&branch=feature/test&path=/users&method=GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000).await.unwrap();
    let content = String::from_utf8(body.to_vec()).unwrap();
    assert!(content.contains("Updated DTO"));
}

#[tokio::test]
async fn test_admin_data_management() {
    let (app, token) = setup_app_with_admin().await;

    // Enable dev mode so API endpoints work without per-request auth
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/dev-mode")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({"enabled": true})).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let yaml = r#"
openapi: 3.0.0
info:
  title: Test API
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
"#;

    // Provide a spec
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({
                    "servicename": "svc1",
                    "branch": "main",
                    "openapi_yaml": yaml
                })).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // Require to create a client
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require?clientname=client1&servicename=svc1&branch=main&path=/users&method=GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // List services
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/services")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000).await.unwrap();
    let svcs: Vec<Value> = serde_json::from_slice(&body).unwrap();
    assert!(svcs.iter().any(|s| s["name"] == "svc1"));

    // List branches
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/services/svc1/branches")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000).await.unwrap();
    let branches: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert!(branches.contains(&"main".to_string()));

    // List clients
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/clients")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000).await.unwrap();
    let clients: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert!(clients.contains(&"client1".to_string()));

    // Delete client
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/admin/clients/client1")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Delete non-existent client -> 404
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/admin/clients/client1")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // Delete service (cascades branches and endpoints)
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/admin/services/svc1")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify service is gone
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/services")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000).await.unwrap();
    let svcs: Vec<Value> = serde_json::from_slice(&body).unwrap();
    assert!(!svcs.iter().any(|s| s["name"] == "svc1"));
}

#[tokio::test]
async fn test_admin_auth_requires_session() {
    let (app, _) = setup_app().await;

    // 1. Request without token -> 401
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/protected-branches")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // 2. Request with invalid token -> 401
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/protected-branches")
                .header("Authorization", "Bearer invalid-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // 3. Non-admin endpoint (health) still works
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_api_locked_without_dev_mode() {
    let (app, _) = setup_app().await;

    // API endpoints should be locked (dev_mode=false by default)
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require?clientname=c&servicename=s&branch=b&path=/p&method=GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_auth_login_and_session() {
    let (app, token) = setup_app_with_admin().await;

    // Login with correct credentials
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({
                    "username": "admin",
                    "password": "admin-pass"
                })).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000).await.unwrap();
    let login_resp: Value = serde_json::from_slice(&body).unwrap();
    assert!(login_resp["token"].is_string());
    assert_eq!(login_resp["is_admin"], true);
    let new_token = login_resp["token"].as_str().unwrap();

    // Use new token to access /auth/me
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/me")
                .header("Authorization", format!("Bearer {}", new_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000).await.unwrap();
    let me: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(me["username"], "admin");
    assert_eq!(me["is_admin"], true);

    // Login with wrong password -> 401
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({
                    "username": "admin",
                    "password": "wrong"
                })).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // Logout
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/logout")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_role_based_access_control() {
    let (app, admin_token) = setup_app_with_admin().await;

    // Enable local users
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/local-users")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({"enabled": true})).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Create a staff user (non-admin)
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/register")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({
                    "username": "staff",
                    "password": "staff-pass"
                })).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    
    // Admin lists users to find staff ID
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/users")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000).await.unwrap();
    let users: Value = serde_json::from_slice(&body).unwrap();
    let staff_user = users.as_array().unwrap().iter().find(|u| u["username"] == "staff").unwrap();
    let staff_id = staff_user["id"].as_i64().unwrap();

    // Approve staff user (still non-admin)
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/admin/users/{}/approve", staff_id))
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Login as staff
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({
                    "username": "staff",
                    "password": "staff-pass"
                })).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000).await.unwrap();
    let login_resp: Value = serde_json::from_slice(&body).unwrap();
    let staff_token = login_resp["token"].as_str().unwrap().to_string();
    assert_eq!(login_resp["is_admin"], false);

    // 1. Staff should be able to see stats
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/observability/stats")
                .header("Authorization", format!("Bearer {}", staff_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // 2. Staff should be able to see logs
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/observability/logs")
                .header("Authorization", format!("Bearer {}", staff_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // 3. Staff should be able to see discovery routes (e.g. /admin/services)
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/services")
                .header("Authorization", format!("Bearer {}", staff_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // 4. Staff should NOT be able to change debug config
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/observability/debug-config")
                .header("Authorization", format!("Bearer {}", staff_token))
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&json!({"business_logic_debug": true, "admin_user_debug": true})).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // 5. Staff should NOT be able to delete a service
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/admin/services/any-service")
                .header("Authorization", format!("Bearer {}", staff_token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_dev_mode_toggle() {
    let (app, token) = setup_app_with_admin().await;

    // API should be locked by default
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require?clientname=c&servicename=s&branch=b&path=/p&method=GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // Enable dev mode via admin
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/dev-mode")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({"enabled": true})).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Now API should work without auth (will get 404 since no data, but not 403)
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require?clientname=c&servicename=s&branch=b&path=/p&method=GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_user_registration_and_approval() {
    let (app, token) = setup_app_with_admin().await;

    // Registration should fail when local users disabled (default)
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/register")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({
                    "username": "newuser",
                    "password": "secret123"
                })).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // Enable local users
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/local-users")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({"enabled": true})).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Register a new user
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/register")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({
                    "username": "newuser",
                    "password": "secret123"
                })).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // Login should fail (not approved yet)
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({
                    "username": "newuser",
                    "password": "secret123"
                })).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // Admin lists users
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/users")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000).await.unwrap();
    let users: Value = serde_json::from_slice(&body).unwrap();
    let new_user = users.as_array().unwrap().iter().find(|u| u["username"] == "newuser").unwrap();
    assert_eq!(new_user["approved"], false);
    let new_user_id = new_user["id"].as_i64().unwrap();

    // Admin approves user
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/admin/users/{}/approve", new_user_id))
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Login should now succeed
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({
                    "username": "newuser",
                    "password": "secret123"
                })).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Duplicate registration should fail with 409
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/register")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({
                    "username": "newuser",
                    "password": "other"
                })).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);

    // Admin deletes user
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/admin/users/{}", new_user_id))
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_api_token_crud_and_bearer_auth() {
    let (app, token) = setup_app_with_admin().await;

    // Create an API token
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/tokens")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({
                    "name": "jenkins-ci",
                    "expires_in_days": 365
                })).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000).await.unwrap();
    let create_resp: Value = serde_json::from_slice(&body).unwrap();
    let api_token = create_resp["token"].as_str().unwrap().to_string();
    let token_id = create_resp["id"].as_str().unwrap().to_string();
    assert!(api_token.starts_with("san_"));
    assert_eq!(create_resp["name"], "jenkins-ci");

    // List tokens
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/tokens")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000).await.unwrap();
    let tokens_list: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(tokens_list.as_array().unwrap().len(), 1);
    assert_eq!(tokens_list[0]["name"], "jenkins-ci");

    // Disable dev mode so API endpoints require auth
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/dev-mode")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({ "enabled": false })).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Use API token for /report (should work)
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/report?branch=main")
                .header("Authorization", format!("Bearer {}", api_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Use invalid API token (should fail)
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/report?branch=main")
                .header("Authorization", "Bearer san_invalid_token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // Revoke the token
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/auth/tokens/{}", token_id))
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // List should be empty now
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/tokens")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000).await.unwrap();
    let tokens_list: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(tokens_list.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_auth_config_api() {
    let (app, token) = setup_app_with_admin().await;

    // GET auth-config — default should be "dev"
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/auth-config")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000).await.unwrap();
    let data: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(data["auth_mode"], "dev");

    // PUT auth-config — switch to local
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/admin/auth-config")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({
                    "auth_mode": "local"
                })).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify it changed
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/auth-config")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000).await.unwrap();
    let data: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(data["auth_mode"], "local");

    // PUT auth-config — switch to ldap with config
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/admin/auth-config")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({
                    "auth_mode": "ldap",
                    "ldap_config": {
                        "server_url": "ldap://ldap.example.com:389",
                        "bind_dn": "cn=admin,dc=example,dc=com",
                        "bind_password": "secret",
                        "base_dn": "dc=example,dc=com",
                        "user_filter": "(uid={username})",
                        "admin_group": "cn=admins,ou=groups,dc=example,dc=com"
                    }
                })).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify LDAP config is returned with redacted password
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/auth-config")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000).await.unwrap();
    let data: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(data["auth_mode"], "ldap");
    assert_eq!(data["ldap_config"]["server_url"], "ldap://ldap.example.com:389");
    assert_eq!(data["ldap_config"]["bind_password"], "****");

    // PUT with invalid auth mode should fail
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/admin/auth-config")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({
                    "auth_mode": "invalid"
                })).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // PUT ldap without config should fail
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/admin/auth-config")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({
                    "auth_mode": "ldap"
                })).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Requires admin auth
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/auth-config")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_gzip_compression() {
    let (app, _) = setup_app().await;

    let response: Response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/health")
                .header("Accept-Encoding", "gzip")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    // When Accept-Encoding: gzip is sent, the server should respond with compressed content
    // For very small responses the server may skip compression, so we just verify the request succeeds.
    // For larger responses, verify Content-Encoding header is set.
}

#[tokio::test]
async fn test_gzip_compression_on_provide_response() {
    let app = setup_app_dev_mode().await;

    // Provide a spec (creates data for a larger response)
    let openapi_yaml = r#"
openapi: 3.0.0
info:
  title: Compression Test API
  version: 1.0.0
paths:
  /users:
    get:
      summary: Get all users
      description: Returns a list of all users in the system with their details
      responses:
        '200':
          description: OK
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/UserList'
components:
  schemas:
    UserList:
      type: object
      properties:
        users:
          type: array
          items:
            $ref: '#/components/schemas/User'
    User:
      type: object
      properties:
        id:
          type: integer
        name:
          type: string
        email:
          type: string
"#;

    let provide_payload = json!({
        "servicename": "compress-test-service",
        "branch": "main",
        "openapi_yaml": openapi_yaml
    });

    let response: Response = app.clone()
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
    assert!(response.status().is_success());

    // Require with Accept-Encoding: gzip
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require?clientname=gzip-client&servicename=compress-test-service&branch=main&path=/users&method=GET&timeout=1")
                .header("Accept-Encoding", "gzip")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let content_encoding = response.headers().get("content-encoding").map(|v| v.to_str().unwrap().to_string());
    assert_eq!(content_encoding, Some("gzip".to_string()), "Response should be gzip-compressed");
}

#[tokio::test]
async fn test_gzip_request_decompression() {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    let app = setup_app_dev_mode().await;

    let openapi_yaml = r#"
openapi: 3.0.0
info:
  title: Gzip Request Test
  version: 1.0.0
paths:
  /items:
    get:
      summary: Get items
      responses:
        '200':
          description: OK
"#;

    let payload = serde_json::to_vec(&json!({
        "servicename": "gzip-request-test",
        "branch": "main",
        "openapi_yaml": openapi_yaml
    })).unwrap();

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&payload).unwrap();
    let compressed = encoder.finish().unwrap();

    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("Content-Encoding", "gzip")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(compressed))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success(), "Server should accept gzip-compressed request body, got {}", response.status());
}

#[tokio::test]
async fn test_branch_max_age_api() {
    let (app, admin_token) = setup_app_with_admin().await;

    // GET default max-age
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/settings/branch-max-age")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["days"], 30);

    // SET max-age to 7 days
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/branch-max-age")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({"days": 7})).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify it changed
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/settings/branch-max-age")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["days"], 7);

    // SET 0 should fail
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/branch-max-age")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({"days": 0})).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Trigger cleanup (should delete 0 on empty DB)
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/branch-cleanup")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["deleted"], 0);

    // Requires admin auth
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/settings/branch-max-age")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_dependency_max_age_api() {
    let (app, admin_token) = setup_app_with_admin().await;

    // GET default max-age
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/settings/dependency-max-age")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["days"], 30);

    // SET max-age to 14 days
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/dependency-max-age")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({"days": 14})).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify it changed
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/settings/dependency-max-age")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["days"], 14);

    // SET 0 should fail
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/dependency-max-age")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({"days": 0})).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Trigger cleanup (should delete 0 on empty DB)
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/dependency-cleanup")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["deleted"], 0);
}

#[tokio::test]
async fn test_htmx_fragment_endpoints() {
    let (app, admin_token) = setup_app_with_admin().await;

    // GET /fragments/admin/users — should return HTML fragment
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/fragments/admin/users")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = String::from_utf8(
        axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec()
    ).unwrap();
    assert!(body.contains("admin")); // at least the admin user should appear

    // GET /fragments/admin/dev-mode — should return toggle HTML
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/fragments/admin/dev-mode")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = String::from_utf8(
        axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec()
    ).unwrap();
    assert!(body.contains("hx-post")); // should contain htmx attributes

    // POST /fragments/admin/dev-mode/toggle — should toggle and return HTML
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/fragments/admin/dev-mode/toggle")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = String::from_utf8(
        axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec()
    ).unwrap();
    assert!(body.contains("Enabled")); // dev mode was off, now on

    // GET /fragments/admin/services — should return HTML (empty list)
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/fragments/admin/services")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = String::from_utf8(
        axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec()
    ).unwrap();
    assert!(body.contains("No services"));

    // GET /fragments/admin/database-info — should return backend info
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/fragments/admin/database-info")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = String::from_utf8(
        axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec()
    ).unwrap();
    assert!(body.contains("sqlite") || body.contains("SQLite") || body.contains("Backend"));

    // GET /admin.html — should return the full admin page template
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/admin.html")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = String::from_utf8(
        axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec()
    ).unwrap();
    assert!(body.contains("htmx.org"));
    assert!(body.contains("hx-get"));
}

#[tokio::test]
async fn test_require_does_not_create_phantom_service() {
    let (app, admin_token) = setup_app_with_admin().await;

    // A client requires an endpoint from a service that was never provided
    let _require_payload = json!({
        "clientname": "my-client",
        "servicename": "phantom-service",
        "branch": "main",
        "path": "/health",
        "method": "GET"
    });

    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/require?clientname=my-client&servicename=phantom-service&branch=main&path=/health&method=GET")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // The require may succeed or return not-found, either way the service list should be clean
    let _ = response.status();

    // List services via admin API — phantom-service should NOT appear
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/admin/services")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let services: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert!(
        !services.contains(&"phantom-service".to_string()),
        "phantom-service should not appear in services list when it has no branches"
    );
}

#[tokio::test]
async fn test_delete_service_does_not_create_phantom_client() {
    let (app, admin_token) = setup_app_with_admin().await;

    // Upload a spec so the service exists
    let yaml = "openapi: '3.0.0'\ninfo:\n  title: Svc\n  version: '1.0'\npaths:\n  /health:\n    get:\n      operationId: getHealth\n      responses:\n        '200':\n          description: OK\n";
    let provide_payload = json!({
        "servicename": "temp-service",
        "branch": "main",
        "openapi_yaml": yaml
    });
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&provide_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());

    // A client requires an endpoint from that service
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/require?clientname=orphan-client&servicename=temp-service&branch=main&path=/health&method=GET")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify client appears in the list
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/admin/clients")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let clients: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert!(clients.contains(&"orphan-client".to_string()));

    // Delete the service — this removes dependencies but leaves the client row
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/admin/services/temp-service")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Client should NOT appear in the list anymore (no dependencies left)
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/admin/clients")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let clients: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert!(
        !clients.contains(&"orphan-client".to_string()),
        "orphan-client should not appear in clients list after its service was deleted"
    );
}

#[tokio::test]
async fn test_require_missing_endpoint_returns_descriptive_error() {
    let app = setup_app_dev_mode().await;

    // Provide a spec with only /users
    let yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
"#;
    let payload = json!({
        "servicename": "err-svc",
        "branch": "main",
        "openapi_yaml": yaml
    });
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // Require a non-existent endpoint — should get 404 with descriptive body
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/require?clientname=test-client&servicename=err-svc&branch=main&path=/missing&method=GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("GET /missing"), "Error body should contain the missing endpoint: {}", body_str);
    assert!(body_str.contains("err-svc"), "Error body should contain the service name: {}", body_str);
}

#[tokio::test]
async fn test_provide_conflict_returns_descriptive_error() {
    let app = setup_app_dev_mode().await;

    let yaml1 = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/User'
components:
  schemas:
    User:
      type: object
      properties:
        name:
          type: string
"#;
    let yaml2 = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/User'
components:
  schemas:
    User:
      type: object
      properties:
        name:
          type: integer
"#;

    // Provide first version
    let payload = json!({ "servicename": "conflict-svc", "branch": "main", "openapi_yaml": yaml1 });
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST").uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        ).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // Provide breaking change on protected branch — should get 409 with descriptive body
    let payload2 = json!({ "servicename": "conflict-svc", "branch": "main", "openapi_yaml": yaml2 });
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST").uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload2).unwrap()))
                .unwrap(),
        ).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("User"), "Error body should contain schema name: {}", body_str);
    assert!(body_str.contains("conflict-svc"), "Error body should contain service name: {}", body_str);
}

#[tokio::test]
async fn test_provide_dry_run_does_not_store_data() {
    let app = setup_app_dev_mode().await;

    let yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /items:
    get:
      responses:
        '200':
          description: OK
"#;

    // Provide with dry_run=true
    let payload = json!({ "servicename": "dry-svc", "branch": "main", "openapi_yaml": yaml, "dry_run": true });
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST").uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        ).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // Require the endpoint — should NOT be found since dry_run didn't store it
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/require?clientname=dry-client&servicename=dry-svc&branch=main&path=/items&method=GET")
                .body(Body::empty())
                .unwrap(),
        ).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_require_dry_run_does_not_create_dependency() {
    let (app, admin_token) = setup_app_with_admin().await;

    let yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /health:
    get:
      responses:
        '200':
          description: OK
"#;

    // Provide a real spec
    let payload = json!({ "servicename": "dryreq-svc", "branch": "main", "openapi_yaml": yaml });
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST").uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        ).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // Require with dry_run=true — should return the YAML but not create a client
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/require?clientname=dry-client&servicename=dryreq-svc&branch=main&path=/health&method=GET&dry_run=true")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        ).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Client should NOT appear in the clients list
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/admin/clients")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        ).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let clients: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert!(
        !clients.contains(&"dry-client".to_string()),
        "dry-client should not appear after dry_run require"
    );
}

#[tokio::test]
async fn test_multiple_provides() {
    let (app, admin_token) = setup_app_with_admin().await;

    // 1. Provide OpenAPI
    let openapi_payload = json!({
        "servicename": "multi-svc",
        "branch": "main",
        "openapi_yaml": "openapi: 3.0.0\ninfo:\n  title: Test\n  version: 1.0.0\npaths:\n  /hello:\n    get:\n      responses:\n        '200':\n          description: OK"
    });

    let res = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::from(serde_json::to_vec(&openapi_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);

    // 2. Provide Proto
    let proto_payload = json!({
        "servicename": "multi-svc",
        "branch": "main",
        "proto_content": "syntax = \"proto3\";\npackage test;\nservice TestService {\n  rpc Hello (HelloRequest) returns (HelloResponse);\n}\nmessage HelloRequest {}\nmessage HelloResponse {}"
    });

    let res = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide/grpc")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::from(serde_json::to_vec(&proto_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);

    // 2.5 Provide AsyncAPI
    let asyncapi_payload = json!({
        "servicename": "multi-svc",
        "branch": "main",
        "asyncapi_yaml": "asyncapi: 2.0.0\ninfo:\n  title: Test\n  version: 1.0.0\nchannels:\n  events:\n    publish:\n      message:\n        payload:\n          type: object"
    });

    let res = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide/asyncapi")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::from(serde_json::to_vec(&asyncapi_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);

    // 3. Verify all 3 exist via admin list endpoints
    let res = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/services/multi-svc/branches/main/endpoints")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    
    let body = axum::body::to_bytes(res.into_body(), 10000).await.unwrap();
    let endpoints: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let endpoints_list = endpoints.as_array().expect("Response should be an array");
    
    // OpenAPI has 1 endpoint, Proto has 1 endpoint, AsyncAPI has 1 endpoint. Total 3.
    assert_eq!(endpoints_list.len(), 3, "Should have 3 endpoints, got: {:?}", endpoints_list);
    
    let types: Vec<String> = endpoints_list.iter()
        .map(|e| e["api_type"].as_str().unwrap().to_string())
        .collect();
    assert!(types.contains(&"openapi".to_string()));
    assert!(types.contains(&"proto".to_string()));
    assert!(types.contains(&"asyncapi".to_string()));
}

#[tokio::test]
async fn test_client_with_missing_endpoint_appears_in_list() {
    let (app, admin_token) = setup_app_with_admin().await;

    // Provide a spec with only /users
    let yaml = "openapi: '3.0.0'\ninfo:\n  title: Svc\n  version: '1.0'\npaths:\n  /users:\n    get:\n      operationId: getUsers\n      responses:\n        '200':\n          description: OK\n";
    let provide_payload = json!({
        "servicename": "missing-ep-svc",
        "branch": "main",
        "openapi_yaml": yaml
    });
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&provide_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());

    // Client requires a NON-EXISTENT endpoint from that service
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/require?clientname=missing-client&servicename=missing-ep-svc&branch=main&path=/nonexistent&method=GET&timeout=0")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // Should get 404 since endpoint doesn't exist
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // Client should still appear in the clients list
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/admin/clients")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let clients: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert!(
        clients.contains(&"missing-client".to_string()),
        "Client with missing endpoint should appear in clients list, got: {:?}", clients
    );

    // The report should show this in missing_endpoints
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/report?branch=main")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let report: Value = serde_json::from_slice(&body).unwrap();
    let missing = report["missing_endpoints"].as_array().unwrap();
    assert!(
        !missing.is_empty(),
        "Report should contain missing endpoints for the unresolved require"
    );
    assert_eq!(missing[0]["client"].as_str().unwrap(), "missing-client");
}

#[tokio::test]
async fn test_no_duplicate_null_endpoint_dependencies() {
    let (app, admin_token) = setup_app_with_admin().await;

    // Provide a spec
    let yaml = "openapi: '3.0.0'\ninfo:\n  title: Svc\n  version: '1.0'\npaths:\n  /users:\n    get:\n      operationId: getUsers\n      responses:\n        '200':\n          description: OK\n";
    let provide_payload = json!({
        "servicename": "dedup-svc",
        "branch": "main",
        "openapi_yaml": yaml
    });
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&provide_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());

    // Require a missing endpoint TWICE
    for _ in 0..2 {
        let _response: Response = app.clone()
            .oneshot(
                Request::builder()
                    .uri("/require?clientname=dedup-client&servicename=dedup-svc&branch=main&path=/missing&method=GET&timeout=0")
                    .header("Authorization", format!("Bearer {}", admin_token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
    }

    // Check report: should have exactly ONE missing endpoint entry, not two
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/report?branch=main")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let report: Value = serde_json::from_slice(&body).unwrap();
    let missing = report["missing_endpoints"].as_array().unwrap();
    let dedup_missing: Vec<&Value> = missing.iter()
        .filter(|m| m["client"].as_str() == Some("dedup-client") && m["service"].as_str() == Some("dedup-svc"))
        .collect();
    assert_eq!(
        dedup_missing.len(), 1,
        "Should have exactly 1 missing endpoint entry, not duplicates. Got: {:?}", dedup_missing
    );
}

#[tokio::test]
async fn test_nuke_endpoints() {
    let (app, admin_token) = setup_app_with_admin().await;

    // Provide a service
    let yaml = "openapi: '3.0.0'\ninfo:\n  title: Svc\n  version: '1.0'\npaths:\n  /items:\n    get:\n      operationId: getItems\n      responses:\n        '200':\n          description: OK\n";
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&json!({
                    "servicename": "nuke-svc",
                    "branch": "main",
                    "openapi_yaml": yaml
                })).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());

    // Require to create a client
    let _response: Response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/require?clientname=nuke-client&servicename=nuke-svc&branch=main&path=/items&method=GET&timeout=0")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Nuke services with wrong confirmation should fail
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/nuke/services")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({"confirmation": "wrong"})).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Nuke services with correct confirmation
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/nuke/services")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({"confirmation": "DELETE ALL SERVICES"})).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let result: Value = serde_json::from_slice(&body).unwrap();
    assert!(result["deleted"].as_u64().unwrap() >= 1);

    // Verify services list is empty
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/admin/services")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let services: Vec<Value> = serde_json::from_slice(&body).unwrap();
    assert!(services.is_empty(), "Services should be empty after nuke");

    // Nuke clients
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/nuke/clients")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&json!({"confirmation": "DELETE ALL CLIENTS"})).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify clients list is empty
    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/admin/clients")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let clients: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert!(clients.is_empty(), "Clients should be empty after nuke");
}

#[tokio::test]
async fn test_problem_2_multi_publisher_conflict_integration() {
    let app = setup_app_dev_mode().await;

    let yaml_v1 = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /orders:
    get:
      responses:
        '200':
          description: OK
"#;
    let yaml_v2_compatible = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /orders:
    get:
      responses:
        '200':
          description: OK
        '404':
          description: Not Found
"#;
    let yaml_v2_breaking = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /orders:
    get:
      responses:
        '404':
          description: Not Found
"#;

    // 1. ALPHA provides v1 on feature branch.
    let payload_alpha_v1 = json!({ "servicename": "alpha-svc", "branch": "feat-problem-2", "openapi_yaml": yaml_v1 });
    let response = app.clone().oneshot(
        Request::builder().method("POST").uri("/provide")
            .header("Content-Type", "application/json")
            .header("X-CSRF-Token", TEST_CSRF_TOKEN)
            .body(Body::from(serde_json::to_vec(&payload_alpha_v1).unwrap())).unwrap()
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // 2. BETA provides v1 on same branch. Should be OK (same as source).
    let payload_beta_v1 = json!({ "servicename": "beta-svc", "branch": "feat-problem-2", "openapi_yaml": yaml_v1 });
    let response = app.clone().oneshot(
        Request::builder().method("POST").uri("/provide")
            .header("Content-Type", "application/json")
            .header("X-CSRF-Token", TEST_CSRF_TOKEN)
            .body(Body::from(serde_json::to_vec(&payload_beta_v1).unwrap())).unwrap()
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // 3. ALPHA provides compatible modification. OK. ALPHA becomes owner.
    let payload_alpha_v2 = json!({ "servicename": "alpha-svc", "branch": "feat-problem-2", "openapi_yaml": yaml_v2_compatible });
    let response = app.clone().oneshot(
        Request::builder().method("POST").uri("/provide")
            .header("Content-Type", "application/json")
            .header("X-CSRF-Token", TEST_CSRF_TOKEN)
            .body(Body::from(serde_json::to_vec(&payload_alpha_v2).unwrap())).unwrap()
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // 4. BETA provides breaking change. Should fail (conflict with current ALPHA owner).
    let payload_beta_v2_breaking = json!({ "servicename": "beta-svc", "branch": "feat-problem-2", "openapi_yaml": yaml_v2_breaking });
    let response = app.clone().oneshot(
        Request::builder().method("POST").uri("/provide")
            .header("Content-Type", "application/json")
            .header("X-CSRF-Token", TEST_CSRF_TOKEN)
            .body(Body::from(serde_json::to_vec(&payload_beta_v2_breaking).unwrap())).unwrap()
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("current owner's version"));

    // 5. ALPHA provides v1 again. OK. Reverts to source, owner becomes None.
    let response = app.clone().oneshot(
        Request::builder().method("POST").uri("/provide")
            .header("Content-Type", "application/json")
            .header("X-CSRF-Token", TEST_CSRF_TOKEN)
            .body(Body::from(serde_json::to_vec(&payload_alpha_v1).unwrap())).unwrap()
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // 6. BETA provides breaking change. Should fail (conflict with source).
    let response = app.clone().oneshot(
        Request::builder().method("POST").uri("/provide")
            .header("Content-Type", "application/json")
            .header("X-CSRF-Token", TEST_CSRF_TOKEN)
            .body(Body::from(serde_json::to_vec(&payload_beta_v2_breaking).unwrap())).unwrap()
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("source version"));
}

#[tokio::test]
async fn test_problem_3_optimistic_concurrency_integration() {
    let app = setup_app_dev_mode().await;
    let yaml1 = "openapi: 3.0.0\ninfo:\n  title: T1\n  version: 1.0.0\npaths: {}";
    let yaml2 = "openapi: 3.0.0\ninfo:\n  title: T2\n  version: 1.0.0\npaths: {}";

    // 1. First provide
    let payload1 = json!({ "servicename": "svc", "branch": "main", "openapi_yaml": yaml1 });
    let response = app.clone().oneshot(
        Request::builder().method("POST").uri("/provide")
            .header("Content-Type", "application/json")
            .header("X-CSRF-Token", TEST_CSRF_TOKEN)
            .body(Body::from(serde_json::to_vec(&payload1).unwrap())).unwrap()
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let res1: ProvideResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(res1.version, 1);

    // 2. Second provide with correct base_version
    let payload2 = json!({ "servicename": "svc", "branch": "main", "openapi_yaml": yaml2, "base_version": 1 });
    let response = app.clone().oneshot(
        Request::builder().method("POST").uri("/provide")
            .header("Content-Type", "application/json")
            .header("X-CSRF-Token", TEST_CSRF_TOKEN)
            .body(Body::from(serde_json::to_vec(&payload2).unwrap())).unwrap()
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let res2: ProvideResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(res2.version, 2);

    // 3. Third provide with OUTDATED base_version
    let payload3 = json!({ "servicename": "svc", "branch": "main", "openapi_yaml": yaml1, "base_version": 1 });
    let response = app.clone().oneshot(
        Request::builder().method("POST").uri("/provide")
            .header("Content-Type", "application/json")
            .header("X-CSRF-Token", TEST_CSRF_TOKEN)
            .body(Body::from(serde_json::to_vec(&payload3).unwrap())).unwrap()
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("Outdated spec version"));
}
