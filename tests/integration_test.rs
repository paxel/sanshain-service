use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use chrono::Utc;
use sanshain_service::application::services;
use sanshain_service::domain::models::{AuthMode, ProvideResponse};
use sanshain_service::infrastructure::cached_repository::CachedSpecRepository;
use sanshain_service::infrastructure::database::DatabaseRepo;
use sanshain_service::infrastructure::sqlite_repository::SqliteSpecRepository;
use sanshain_service::{AppState, create_app};
use serde_json::{Value, json};
use sqlx::sqlite::SqlitePoolOptions;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;
use tower::ServiceExt; // for `oneshot`

const TEST_CSRF_TOKEN: &str = "test-csrf-token";

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
    let mut tokens = HashMap::new();
    tokens.insert(
        TEST_CSRF_TOKEN.to_string(),
        Utc::now() + chrono::Duration::hours(1),
    );
    let (spec_updated_tx, _) = tokio::sync::broadcast::channel(100);

    AppState {
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
        system: Arc::new(std::sync::Mutex::new(sysinfo::System::new_all())),
    }
}

async fn setup_app() -> (axum::Router, SqliteSpecRepository) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();

    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    let app = create_app(test_app_state(repo.clone()));
    (app, repo)
}

/// Setup app with initial admin and dev mode enabled (for tests that don't care about auth)
async fn setup_app_dev_mode() -> axum::Router {
    let (app, repo) = setup_app().await;
    services::ensure_initial_admin(&repo).await.unwrap();
    services::set_auth_mode(&repo, &AuthMode::Dev)
        .await
        .unwrap();
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
    let session = repo
        .create_session(user.id, "2099-12-31T23:59:59")
        .await
        .unwrap();

    services::set_auth_mode(&repo, &AuthMode::Local)
        .await
        .unwrap();

    let app = create_app(test_app_state(repo));
    (app, session.token)
}

#[tokio::test]
async fn test_security_headers() {
    let (app, _) = setup_app().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let headers = response.headers();
    assert_eq!(headers.get("X-Content-Type-Options").unwrap(), "nosniff");
    assert_eq!(headers.get("X-Frame-Options").unwrap(), "SAMEORIGIN");
    assert_eq!(
        headers.get("Referrer-Policy").unwrap(),
        "strict-origin-when-cross-origin"
    );
    assert!(
        headers
            .get("Content-Security-Policy")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("default-src 'self'")
    );
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
async fn test_metrics_endpoint_is_available() {
    let (app, _) = setup_app().await;

    let response: Response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "text/plain; version=0.0.4; charset=utf-8"
    );
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

    assert!(html.contains("href=\"/services.html\" class=\"text-white hover:text-indigo-100 transition-colors\">Services</a>"));
    assert!(html.contains("href=\"/clients.html\" class=\"text-white hover:text-indigo-100 transition-colors\">Clients</a>"));
    assert!(html.contains("href=\"/graph.html\" class=\"text-white hover:text-indigo-100 transition-colors\">Graph</a>"));
    assert!(html.contains("href=\"/reports.html\" class=\"text-white hover:text-indigo-100 transition-colors\">Reports</a>"));
    assert!(html.contains("href=\"/observability.html\" class=\"text-white hover:text-indigo-100 transition-colors\">Observability</a>"));
    assert!(html.contains("id=\"nav-admin-link\" href=\"/admin.html\""));
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
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let report: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(report["dependency_graph"].as_array().unwrap().len(), 1);
    assert_eq!(report["unused_endpoints"].as_array().unwrap().len(), 0);
    assert_eq!(report["missing_endpoints"].as_array().unwrap().len(), 0);

    // 4. Get markdown report
    let response: Response = app
        .clone()
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
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "text/markdown; charset=utf-8"
    );
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
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

    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), 100000)
        .await
        .unwrap();
    let merged_yaml = String::from_utf8(body.to_vec()).unwrap();

    // Both paths present in merged YAML
    assert!(merged_yaml.contains("/users"));
    assert!(merged_yaml.contains("/orders"));
    // Schemas deduplicated
    assert!(merged_yaml.contains("User"));
    assert!(merged_yaml.contains("Order"));

    // Parse and verify structure
    let parsed: openapiv3::OpenAPI = serde_yaml_ng::from_str(&merged_yaml).unwrap();
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

    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(
        body_str.contains("DELETE /nonexistent"),
        "Error body should list the missing endpoint"
    );

    // Test empty endpoints returns error
    let bundle_empty = json!({
        "clientname": "java-client",
        "servicename": "test-service",
        "branch": "main",
        "endpoints": []
    });

    let response: Response = app
        .clone()
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
async fn test_require_bundle_etag_stability() {
    let app = setup_app_dev_mode().await;

    // Provide a spec with two distinct endpoints
    let openapi_yaml = r#"
openapi: 3.0.0
info:
  title: Test API
  version: 1.0.0
paths:
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
        id:
          type: integer
"#;
    let provide_payload = json!({
        "servicename": "etag-svc",
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

    // Request bundle: order A (users first, orders second)
    let bundle_a = json!({
        "clientname": "etag-client",
        "servicename": "etag-svc",
        "branch": "main",
        "endpoints": [
            { "path": "/users", "method": "POST" },
            { "path": "/orders", "method": "GET" }
        ]
    });
    let resp_a: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/require-bundle")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&bundle_a).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_a.status(), StatusCode::OK);
    let etag_a = resp_a
        .headers()
        .get("etag")
        .expect("Response A should have ETag")
        .to_str()
        .unwrap()
        .to_string();
    let body_a = String::from_utf8(
        axum::body::to_bytes(resp_a.into_body(), 100000)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();

    // Request bundle: order B (orders first, users second)
    let bundle_b = json!({
        "clientname": "etag-client",
        "servicename": "etag-svc",
        "branch": "main",
        "endpoints": [
            { "path": "/orders", "method": "GET" },
            { "path": "/users", "method": "POST" }
        ]
    });
    let resp_b: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/require-bundle")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&bundle_b).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_b.status(), StatusCode::OK);
    let etag_b = resp_b
        .headers()
        .get("etag")
        .expect("Response B should have ETag")
        .to_str()
        .unwrap()
        .to_string();
    let body_b = String::from_utf8(
        axum::body::to_bytes(resp_b.into_body(), 100000)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();

    assert_eq!(
        etag_a, etag_b,
        "ETags must match regardless of endpoint order"
    );
    assert_eq!(
        body_a, body_b,
        "Response bodies must match regardless of endpoint order"
    );
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
    let response = app
        .clone()
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
    let response = app
        .clone()
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

    let response = app
        .clone()
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
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&payload_with_schema).unwrap(),
                ))
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
    let response = app
        .clone()
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

    let response = app
        .clone()
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
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let branches: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert!(branches.contains(&"main".to_string()));
    assert!(branches.contains(&"master".to_string()));

    // 2. Add a protected branch
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/protected-branches")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({"pattern": "release"})).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // 3. Delete a protected branch
    let response: Response = app
        .clone()
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
    let response: Response = app
        .clone()
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
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "servicename": "svc",
                        "branch": "feature/test",
                        "openapi_yaml": yaml1
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // Update on feature branch (should succeed, not protected)
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "servicename": "svc",
                        "branch": "feature/test",
                        "openapi_yaml": yaml2
                    }))
                    .unwrap(),
                ))
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
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let content = String::from_utf8(body.to_vec()).unwrap();
    assert!(content.contains("Updated DTO"));
}

#[tokio::test]
async fn test_admin_data_management() {
    let (app, token) = setup_app_with_admin().await;

    // Enable dev mode so API endpoints work without per-request auth
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/dev-mode")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({"enabled": true})).unwrap(),
                ))
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
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "servicename": "svc1",
                        "branch": "main",
                        "openapi_yaml": yaml
                    }))
                    .unwrap(),
                ))
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
    let response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let svcs: Vec<Value> = serde_json::from_slice(&body).unwrap();
    assert!(svcs.iter().any(|s| s["name"] == "svc1"));

    // List branches
    let response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let branches: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert!(branches.contains(&"main".to_string()));

    // List clients
    let response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let clients: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert!(clients.contains(&"client1".to_string()));

    // Delete client
    let response = app
        .clone()
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
    let response = app
        .clone()
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
    let response = app
        .clone()
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
    let response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let svcs: Vec<Value> = serde_json::from_slice(&body).unwrap();
    assert!(!svcs.iter().any(|s| s["name"] == "svc1"));
}

#[tokio::test]
async fn test_admin_auth_requires_session() {
    let (app, _) = setup_app().await;

    // 1. Request without token -> 401
    let response: Response = app
        .clone()
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
    let response: Response = app
        .clone()
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
    let response: Response = app
        .clone()
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
    let (app, repo) = setup_app().await;
    services::set_auth_mode(&repo, &AuthMode::Local)
        .await
        .unwrap();

    // API endpoints should be locked (dev_mode=false by default)
    let response: Response = app
        .clone()
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
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "username": "admin",
                        "password": "admin-pass"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let login_resp: Value = serde_json::from_slice(&body).unwrap();
    assert!(login_resp["token"].is_string());
    assert_eq!(login_resp["is_admin"], true);
    let new_token = login_resp["token"].as_str().unwrap();

    // Use new token to access /auth/me
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let me: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(me["username"], "admin");
    assert_eq!(me["is_admin"], true);

    // Login with wrong password -> 401
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "username": "admin",
                        "password": "wrong"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // Logout
    let response: Response = app
        .clone()
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
async fn test_auth_change_password_route_updates_credentials() {
    let (app, token) = setup_app_with_admin().await;

    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/change-password")
                .header("Authorization", format!("Bearer {}", token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "old_password": "admin-pass",
                        "new_password": "new-admin-pass"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let old_login: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "username": "admin",
                        "password": "admin-pass"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(old_login.status(), StatusCode::UNAUTHORIZED);

    let new_login: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "username": "admin",
                        "password": "new-admin-pass"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(new_login.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_root_change_password_keeps_session_and_allows_relogin() {
    let (app, repo) = setup_app().await;
    let hash = services::hash_password("root-pass").unwrap();
    let user = repo.create_user("root", &hash, true, true).await.unwrap();

    use sanshain_service::domain::ports::SpecRepository;
    let session = repo
        .create_session(user.id, "2099-12-31T23:59:59")
        .await
        .unwrap();

    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/change-password")
                .header("Authorization", format!("Bearer {}", session.token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "old_password": "root-pass",
                        "new_password": "new-root-pass"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response_body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let change_password_json: Value = serde_json::from_slice(&response_body).unwrap();
    let refreshed_token = change_password_json["token"].as_str().unwrap().to_string();
    assert_ne!(refreshed_token, session.token);

    let still_authenticated: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/me")
                .header("Authorization", format!("Bearer {}", refreshed_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(still_authenticated.status(), StatusCode::OK);

    let old_session: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/me")
                .header("Authorization", format!("Bearer {}", session.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(old_session.status(), StatusCode::UNAUTHORIZED);

    let old_login: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "username": "root",
                        "password": "root-pass"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(old_login.status(), StatusCode::UNAUTHORIZED);

    let new_login: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "username": "root",
                        "password": "new-root-pass"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(new_login.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_auto_approve_setting_controls_new_user_approval() {
    let (app, admin_token) = setup_app_with_admin().await;

    let enable_local_users: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/local-users")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({ "enabled": true })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(enable_local_users.status(), StatusCode::OK);

    let get_default_auto_approve: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/settings/auto-approve")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get_default_auto_approve.status(), StatusCode::OK);
    let default_body = axum::body::to_bytes(get_default_auto_approve.into_body(), 10000)
        .await
        .unwrap();
    let default_json: Value = serde_json::from_slice(&default_body).unwrap();
    assert_eq!(default_json, json!({ "auto_approve_users": false }));

    let register_pending: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/register")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "username": "pending-user",
                        "password": "pass123"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(register_pending.status(), StatusCode::CREATED);

    let users_before_toggle: Response = app
        .clone()
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
    assert_eq!(users_before_toggle.status(), StatusCode::OK);
    let users_before_body = axum::body::to_bytes(users_before_toggle.into_body(), 10000)
        .await
        .unwrap();
    let users_before: Value = serde_json::from_slice(&users_before_body).unwrap();
    assert_eq!(
        users_before
            .as_array()
            .unwrap()
            .iter()
            .find(|user| user["username"] == "pending-user")
            .unwrap()["approved"],
        false
    );

    let enable_auto_approve: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/auto-approve")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({ "enabled": true })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(enable_auto_approve.status(), StatusCode::OK);

    let register_approved: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/register")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "username": "approved-user",
                        "password": "pass123"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(register_approved.status(), StatusCode::CREATED);

    let users_after_toggle: Response = app
        .clone()
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
    assert_eq!(users_after_toggle.status(), StatusCode::OK);
    let users_after_body = axum::body::to_bytes(users_after_toggle.into_body(), 10000)
        .await
        .unwrap();
    let users_after: Value = serde_json::from_slice(&users_after_body).unwrap();
    assert_eq!(
        users_after
            .as_array()
            .unwrap()
            .iter()
            .find(|user| user["username"] == "approved-user")
            .unwrap()["approved"],
        true
    );
}

#[tokio::test]
async fn test_role_based_access_control() {
    let (app, admin_token) = setup_app_with_admin().await;

    // Enable local users
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/local-users")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({"enabled": true})).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Create a staff user (non-admin)
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/register")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "username": "staff",
                        "password": "staff-pass"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // Admin lists users to find staff ID
    let response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let users: Value = serde_json::from_slice(&body).unwrap();
    let staff_user = users
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "staff")
        .unwrap();
    let staff_id = staff_user["id"].as_i64().unwrap();

    // Approve staff user (still non-admin)
    let response = app
        .clone()
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
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "username": "staff",
                        "password": "staff-pass"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let login_resp: Value = serde_json::from_slice(&body).unwrap();
    let staff_token = login_resp["token"].as_str().unwrap().to_string();
    assert_eq!(login_resp["is_admin"], false);

    // 1. Staff should be able to see stats
    let response = app
        .clone()
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
    let response = app
        .clone()
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
    let response = app
        .clone()
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
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/observability/debug-config-update")
                .header("Authorization", format!("Bearer {}", staff_token))
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(
                        &json!({"business_logic_debug": true, "admin_user_debug": true}),
                    )
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // 5. Staff should NOT be able to delete a service
    let response = app
        .clone()
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
    let response: Response = app
        .clone()
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
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/dev-mode")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({"enabled": true})).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Now API should work without auth (will get 404 since no data, but not 403)
    let response: Response = app
        .clone()
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
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/register")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "username": "newuser",
                        "password": "secret123"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // Enable local users
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/local-users")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({"enabled": true})).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Register a new user
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/register")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "username": "newuser",
                        "password": "secret123"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // Login should fail (not approved yet)
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "username": "newuser",
                        "password": "secret123"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // Admin lists users
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let users: Value = serde_json::from_slice(&body).unwrap();
    let new_user = users
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "newuser")
        .unwrap();
    assert_eq!(new_user["approved"], false);
    let new_user_id = new_user["id"].as_i64().unwrap();

    // Admin approves user
    let response: Response = app
        .clone()
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
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "username": "newuser",
                        "password": "secret123"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Duplicate registration should fail with 409
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/register")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "username": "newuser",
                        "password": "other"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);

    // Admin deletes user
    let response: Response = app
        .clone()
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
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/tokens")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": "jenkins-ci",
                        "expires_in_days": 365
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let create_resp: Value = serde_json::from_slice(&body).unwrap();
    let api_token = create_resp["token"].as_str().unwrap().to_string();
    let token_id = create_resp["id"].as_str().unwrap().to_string();
    assert!(api_token.starts_with("san_"));
    assert_eq!(create_resp["name"], "jenkins-ci");

    // List tokens
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let tokens_list: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(tokens_list.as_array().unwrap().len(), 1);
    assert_eq!(tokens_list[0]["name"], "jenkins-ci");

    // Disable dev mode so API endpoints require auth
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/dev-mode")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({ "enabled": false })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Use API token for /report (should work)
    let response: Response = app
        .clone()
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
    let response: Response = app
        .clone()
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
    let response: Response = app
        .clone()
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
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let tokens_list: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(tokens_list.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_auth_config_api() {
    let (app, token) = setup_app_with_admin().await;

    // GET auth-config — default should be "local" (configured by setup_app_with_admin)
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let data: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(data["auth_mode"], "local");

    // PUT auth-config — switch to dev
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/admin/auth-config")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "auth_mode": "dev"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify it changed to dev
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let data: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(data["auth_mode"], "dev");

    // PUT auth-config — switch to local
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/admin/auth-config")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "auth_mode": "local"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify it changed
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let data: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(data["auth_mode"], "local");

    // PUT auth-config — switch to ldap with config
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/admin/auth-config")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "auth_mode": "ldap",
                        "ldap_config": {
                            "server_url": "ldap://ldap.example.com:389",
                            "bind_dn": "cn=admin,dc=example,dc=com",
                            "bind_password": "secret",
                            "base_dn": "dc=example,dc=com",
                            "user_filter": "(uid={username})",
                            "admin_group": "cn=admins,ou=groups,dc=example,dc=com"
                        }
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify LDAP config is returned with redacted password
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let data: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(data["auth_mode"], "ldap");
    assert_eq!(
        data["ldap_config"]["server_url"],
        "ldap://ldap.example.com:389"
    );
    assert_eq!(data["ldap_config"]["bind_password"], "****");

    // PUT with invalid auth mode should fail
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/admin/auth-config")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "auth_mode": "invalid"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // PUT ldap without config should fail
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/admin/auth-config")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "auth_mode": "ldap"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Requires admin auth
    let response: Response = app
        .clone()
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
async fn test_disabled_mode_endpoints_return_503() {
    let (app, _) = setup_app().await;

    // By default, the app is in "disabled" mode, so calling /provide should return 503 Service Unavailable
    let provide_payload = json!({
        "servicename": "test-service",
        "branch": "main",
        "openapi_yaml": "openapi: 3.0.0\ninfo:\n  title: Test\n  version: 1.0.0\npaths:\n  /:\n    get:\n      responses:\n        '200':\n          description: OK"
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

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert_eq!(text, "Service is in Maintenance Mode / Disabled");

    // Also /require should return 503
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require?clientname=client-a&servicename=test-service&branch=main&path=/&method=GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert_eq!(text, "Service is in Maintenance Mode / Disabled");
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

    let content_encoding = response
        .headers()
        .get("content-encoding")
        .map(|v| v.to_str().unwrap().to_string());
    assert_eq!(
        content_encoding,
        Some("gzip".to_string()),
        "Response should be gzip-compressed"
    );
}

#[tokio::test]
async fn test_gzip_request_decompression() {
    use flate2::Compression;
    use flate2::write::GzEncoder;
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
    }))
    .unwrap();

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&payload).unwrap();
    let compressed = encoder.finish().unwrap();

    let response: Response = app
        .clone()
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
    assert!(
        response.status().is_success(),
        "Server should accept gzip-compressed request body, got {}",
        response.status()
    );
}

#[tokio::test]
async fn test_branch_max_age_api() {
    let (app, admin_token) = setup_app_with_admin().await;

    // GET default max-age
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["days"], 30);

    // SET max-age to 7 days
    let response: Response = app
        .clone()
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
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["days"], 7);

    // SET 0 should fail
    let response: Response = app
        .clone()
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
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["deleted"], 0);

    // Requires admin auth
    let response: Response = app
        .clone()
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
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["days"], 30);

    // SET max-age to 14 days
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/dependency-max-age")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({"days": 14})).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify it changed
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["days"], 14);

    // SET 0 should fail
    let response: Response = app
        .clone()
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
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["deleted"], 0);
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
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
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
    let response: Response = app
        .clone()
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
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let clients: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert!(clients.contains(&"orphan-client".to_string()));

    // Delete the service — this removes dependencies but leaves the client row
    let response: Response = app
        .clone()
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
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
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
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(
        body_str.contains("GET /missing"),
        "Error body should contain the missing endpoint: {}",
        body_str
    );
    assert!(
        body_str.contains("err-svc"),
        "Error body should contain the service name: {}",
        body_str
    );
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
    let response: Response = app
        .clone()
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

    // Provide breaking change on protected branch — should get 409 with descriptive body
    let payload2 =
        json!({ "servicename": "conflict-svc", "branch": "main", "openapi_yaml": yaml2 });
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload2).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(
        body_str.contains("User"),
        "Error body should contain schema name: {}",
        body_str
    );
    assert!(
        body_str.contains("conflict-svc"),
        "Error body should contain service name: {}",
        body_str
    );
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
    let response: Response = app
        .clone()
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
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
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
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
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

    let res = app
        .clone()
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

    let res = app
        .clone()
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

    let res = app
        .clone()
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
    let res = app
        .clone()
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
    assert_eq!(
        endpoints_list.len(),
        3,
        "Should have 3 endpoints, got: {:?}",
        endpoints_list
    );

    let types: Vec<String> = endpoints_list
        .iter()
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
    let response: Response = app
        .clone()
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
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let clients: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert!(
        clients.contains(&"missing-client".to_string()),
        "Client with missing endpoint should appear in clients list, got: {:?}",
        clients
    );

    // The report should show this in missing_endpoints
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
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
    let response: Response = app
        .clone()
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
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let report: Value = serde_json::from_slice(&body).unwrap();
    let missing = report["missing_endpoints"].as_array().unwrap();
    let dedup_missing: Vec<&Value> = missing
        .iter()
        .filter(|m| {
            m["client"].as_str() == Some("dedup-client")
                && m["service"].as_str() == Some("dedup-svc")
        })
        .collect();
    assert_eq!(
        dedup_missing.len(),
        1,
        "Should have exactly 1 missing endpoint entry, not duplicates. Got: {:?}",
        dedup_missing
    );
}

#[tokio::test]
async fn test_nuke_endpoints() {
    let (app, admin_token) = setup_app_with_admin().await;

    // Provide a service
    let yaml = "openapi: '3.0.0'\ninfo:\n  title: Svc\n  version: '1.0'\npaths:\n  /items:\n    get:\n      operationId: getItems\n      responses:\n        '200':\n          description: OK\n";
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "servicename": "nuke-svc",
                        "branch": "main",
                        "openapi_yaml": yaml
                    }))
                    .unwrap(),
                ))
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
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/nuke/services")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({"confirmation": "wrong"})).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Nuke services with correct confirmation
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/nuke/services")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({"confirmation": "DELETE ALL SERVICES"})).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let result: Value = serde_json::from_slice(&body).unwrap();
    assert!(result["deleted"].as_u64().unwrap() >= 1);

    // Verify services list is empty
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let services: Vec<Value> = serde_json::from_slice(&body).unwrap();
    assert!(services.is_empty(), "Services should be empty after nuke");

    // Nuke clients
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/nuke/clients")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({"confirmation": "DELETE ALL CLIENTS"})).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify clients list is empty
    let response: Response = app
        .clone()
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
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
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

    // 0. Seed ALPHA on protected branch so shared contract checks are active (auto-skip is off).
    let payload_alpha_seed =
        json!({ "servicename": "alpha-svc", "branch": "main", "openapi_yaml": yaml_v1 });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload_alpha_seed).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // 1. ALPHA provides v1 on feature branch.
    let payload_alpha_v1 =
        json!({ "servicename": "alpha-svc", "branch": "feat-problem-2", "openapi_yaml": yaml_v1 });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload_alpha_v1).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // 2. BETA (different service) provides breaking change on same branch. Should SUCCEED (different service = independent).
    let payload_beta_breaking = json!({ "servicename": "beta-svc", "branch": "feat-problem-2", "openapi_yaml": yaml_v2_breaking });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&payload_beta_breaking).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // 3. ALPHA provides compatible modification. OK. ALPHA becomes owner.
    let payload_alpha_v2 = json!({ "servicename": "alpha-svc", "branch": "feat-problem-2", "openapi_yaml": yaml_v2_compatible });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload_alpha_v2).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // 4. ALPHA (same service) provides breaking change. Should fail (conflict with source).
    let payload_alpha_v2_breaking = json!({ "servicename": "alpha-svc", "branch": "feat-problem-2", "openapi_yaml": yaml_v2_breaking });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&payload_alpha_v2_breaking).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("source version"));

    // 5. ALPHA provides v1 again. OK. Reverts to source, owner becomes None.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload_alpha_v1).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // 6. ALPHA (same service) provides breaking change after rollback. Should fail (conflict with source).
    let payload_alpha_v2_breaking_on_v1 = json!({ "servicename": "alpha-svc", "branch": "feat-problem-2", "openapi_yaml": yaml_v2_breaking });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&payload_alpha_v2_breaking_on_v1).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
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
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload1).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let res1: ProvideResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(res1.version, 1);

    // 2. Second provide with correct base_version
    let payload2 =
        json!({ "servicename": "svc", "branch": "main", "openapi_yaml": yaml2, "base_version": 1 });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload2).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let res2: ProvideResponse = serde_json::from_slice(&body).unwrap();
    assert_eq!(res2.version, 2);

    // 3. Third provide with OUTDATED base_version
    let payload3 =
        json!({ "servicename": "svc", "branch": "main", "openapi_yaml": yaml1, "base_version": 1 });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload3).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("Outdated spec version"));
}

/// Exhaustive test: every admin-write endpoint must reject a non-admin (staff) token
/// with 403 FORBIDDEN, while read-only admin endpoints should be accessible.
/// This catches regressions where a new admin route forgets `admin_auth`.
#[tokio::test]
async fn test_all_admin_endpoints_require_admin_token() {
    let (app, admin_token) = setup_app_with_admin().await;

    // --- Setup: create a staff user and get their token ---
    // Enable local users
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/local-users")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({"enabled": true})).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Enable auto-approve
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/auto-approve")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({"enabled": true})).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Register staff user
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/register")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({"username": "staff", "password": "staff-pass"}))
                        .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // Login as staff
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({"username": "staff", "password": "staff-pass"}))
                        .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    let login_resp: Value = serde_json::from_slice(&body).unwrap();
    let staff_token = login_resp["token"].as_str().unwrap().to_string();

    // --- Admin-only (write) endpoints: staff must get 403 FORBIDDEN ---
    let admin_only_endpoints: Vec<(&str, &str, Option<Value>)> = vec![
        // Protected branches
        (
            "POST",
            "/admin/protected-branches",
            Some(json!({"pattern": "release"})),
        ),
        ("DELETE", "/admin/protected-branches/main", None),
        // Service/branch/client deletion
        ("DELETE", "/admin/services/any-service", None),
        ("DELETE", "/admin/services/any-service/branches/main", None),
        ("DELETE", "/admin/clients/any-client", None),
        // Settings (POST = write)
        (
            "POST",
            "/admin/settings/dev-mode",
            Some(json!({"enabled": false})),
        ),
        (
            "POST",
            "/admin/settings/local-users",
            Some(json!({"enabled": false})),
        ),
        (
            "POST",
            "/admin/settings/auto-approve",
            Some(json!({"enabled": false})),
        ),
        // Auth config
        ("PUT", "/admin/auth-config", Some(json!({"mode": "local"}))),
        ("POST", "/admin/auth-config/test", Some(json!({}))),
        // Nuke endpoints
        ("POST", "/admin/nuke/database", None),
        ("POST", "/admin/nuke/services", None),
        ("POST", "/admin/nuke/clients", None),
        ("POST", "/admin/nuke/users", None),
        ("POST", "/admin/nuke/branch/main", None),
        // Branch max-age
        (
            "POST",
            "/admin/settings/branch-max-age",
            Some(json!({"days": 7})),
        ),
        ("POST", "/admin/cleanup/branches", None),
        ("POST", "/admin/settings/branch-cleanup", None),
        // Dependency max-age
        (
            "POST",
            "/admin/settings/dependency-max-age",
            Some(json!({"days": 7})),
        ),
        ("POST", "/admin/cleanup/dependencies", None),
        ("POST", "/admin/settings/dependency-cleanup", None),
        // Cache
        (
            "POST",
            "/admin/settings/cache",
            Some(json!({"memory_mb": 256})),
        ),
        ("POST", "/admin/cache/clear", None),
        // User management
        ("POST", "/admin/users/999/approve", None),
        ("DELETE", "/admin/users/999", None),
    ];

    for (method, uri, body) in &admin_only_endpoints {
        let builder = Request::builder()
            .method(*method)
            .uri(*uri)
            .header("Authorization", format!("Bearer {}", staff_token))
            .header("X-CSRF-Token", TEST_CSRF_TOKEN);
        let req = if let Some(b) = body {
            builder
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(b).unwrap()))
                .unwrap()
        } else {
            builder.body(Body::empty()).unwrap()
        };
        let status = app.clone().oneshot(req).await.unwrap().status();
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "Staff user should get 403 on admin-only endpoint: {} {}",
            method,
            uri
        );
    }

    // --- Read-only admin endpoints: staff should NOT get 401/403 (authenticated_auth) ---
    let readonly_endpoints: Vec<&str> = vec![
        "/admin/services",
        "/admin/clients",
        "/admin/observability/stats",
        "/admin/observability/logs",
        "/admin/endpoint-yaml?servicename=x&branch=main&path=/p&method=GET",
        "/admin/endpoint-versions?servicename=x&branch=main&path=/p&method=GET",
        "/admin/shared-contract?servicename=x&branch=main&api_type=openapi&path=/p&method=GET",
    ];

    for uri in &readonly_endpoints {
        let req = Request::builder()
            .method("GET")
            .uri(*uri)
            .header("Authorization", format!("Bearer {}", staff_token))
            .body(Body::empty())
            .unwrap();
        let status = app.clone().oneshot(req).await.unwrap().status();
        assert!(
            status != StatusCode::UNAUTHORIZED && status != StatusCode::FORBIDDEN,
            "Staff user should NOT get 401/403 on read-only admin endpoint: GET {} (got {})",
            uri,
            status
        );
    }

    // --- Admin-only GET endpoints: staff must get 403 ---
    let admin_get_endpoints: Vec<&str> = vec![
        "/admin/protected-branches",
        "/admin/settings/dev-mode",
        "/admin/settings/local-users",
        "/admin/settings/auto-approve",
        "/admin/auth-config",
        "/admin/settings/branch-max-age",
        "/admin/settings/dependency-max-age",
        "/admin/settings/cache",
        "/admin/users",
    ];

    for uri in &admin_get_endpoints {
        let req = Request::builder()
            .method("GET")
            .uri(*uri)
            .header("Authorization", format!("Bearer {}", staff_token))
            .body(Body::empty())
            .unwrap();
        let status = app.clone().oneshot(req).await.unwrap().status();
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "Staff user should get 403 on admin-only GET endpoint: {}",
            uri
        );
    }

    // --- No auth at all: admin endpoints must reject with 401 UNAUTHORIZED ---
    let no_auth_checks: Vec<(&str, &str)> = vec![
        ("GET", "/admin/protected-branches"),
        ("POST", "/admin/settings/dev-mode"),
        ("GET", "/admin/services"),
        ("DELETE", "/admin/services/x"),
        ("GET", "/admin/users"),
        ("POST", "/admin/nuke/database"),
    ];

    for (method, uri) in &no_auth_checks {
        let builder = Request::builder()
            .method(*method)
            .uri(*uri)
            .header("X-CSRF-Token", TEST_CSRF_TOKEN);
        let req = if *method == "POST" {
            builder
                .header("Content-Type", "application/json")
                .body(Body::from("{}"))
                .unwrap()
        } else {
            builder.body(Body::empty()).unwrap()
        };
        let status = app.clone().oneshot(req).await.unwrap().status();
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "Unauthenticated request should get 401 on admin endpoint: {} {}",
            method,
            uri
        );
    }
}

#[tokio::test]
async fn test_merged_report() {
    let app = setup_app_dev_mode().await;

    // Provide a service on "main" branch
    let yaml_main = r#"
openapi: 3.0.0
info:
  title: Main API
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
"#;
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "servicename": "merge-svc",
                        "branch": "main",
                        "openapi_yaml": yaml_main
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // Require from main to create a dependency
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require?clientname=merge-client&servicename=merge-svc&branch=main&path=/users&method=GET")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Provide a different service on "feature" branch
    let yaml_feature = r#"
openapi: 3.0.0
info:
  title: Feature API
  version: 1.0.0
paths:
  /orders:
    post:
      responses:
        '201':
          description: Created
"#;
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "servicename": "feature-svc",
                        "branch": "feature",
                        "openapi_yaml": yaml_feature
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // Require from feature to create a dependency
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require?clientname=feature-client&servicename=feature-svc&branch=feature&path=/orders&method=POST")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Call merged report: feature branch with main as target
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/report/merged?branch=feature&target=main")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 100000)
        .await
        .unwrap();
    let merged: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Verify structure
    assert_eq!(merged["branch"], "feature");
    assert_eq!(merged["target"], "main");
    assert!(merged["dependency_graph"].is_array());
    assert!(merged["node_sources"].is_object());
    assert!(merged["conflicts"].is_array());

    // feature-svc should be Branch source, merge-svc should be Target source
    let node_sources = &merged["node_sources"];
    assert_eq!(node_sources["feature-svc"], "Branch");
    assert_eq!(node_sources["merge-svc"], "Target");

    // dependency_graph should contain entries from both branches
    let deps = merged["dependency_graph"].as_array().unwrap();
    let feature_dep = deps.iter().find(|d| d["service"] == "feature-svc").unwrap();
    assert_eq!(feature_dep["source"], "Branch");
    let main_dep = deps.iter().find(|d| d["service"] == "merge-svc").unwrap();
    assert_eq!(main_dep["source"], "Target");
}

#[tokio::test]
async fn test_protected_branches_public_endpoint() {
    let app = setup_app_dev_mode().await;

    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/branches/protected")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 100000)
        .await
        .unwrap();
    let branches: Vec<String> = serde_json::from_slice(&body).unwrap();
    // Default protected branches include "main" and "master"
    assert!(branches.contains(&"main".to_string()));
    assert!(branches.contains(&"master".to_string()));
}

#[tokio::test]
async fn test_shared_contract_api() {
    let (app, token) = setup_app_with_admin().await;

    // Enable dev mode so provide/require work without per-request auth
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/dev-mode")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({"enabled": true})).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let yaml_v1 = r#"
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
    let yaml_v2 = r#"
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
        '404':
          description: Not Found
"#;

    // 1. No shared contract on protected branch → 204
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/shared-contract?servicename=sc-svc&branch=main&api_type=openapi&path=/items&method=GET")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // 2. Seed on protected branch
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "servicename": "sc-svc",
                        "branch": "main",
                        "openapi_yaml": yaml_v1
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // 3. Provide same on feature branch → shared contract created, no changes
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "servicename": "sc-svc",
                        "branch": "feat-sc",
                        "openapi_yaml": yaml_v1
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // 4. Fetch shared contract → 200, has_changes=false
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/shared-contract?servicename=sc-svc&branch=feat-sc&api_type=openapi&path=/items&method=GET")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let info: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(info["has_changes"], false);
    assert!(info["source_yaml"].is_string());
    assert!(info["current_yaml"].is_string());

    // 5. Provide compatible change on feature branch → has_changes=true
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "servicename": "sc-svc",
                        "branch": "feat-sc",
                        "openapi_yaml": yaml_v2
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // 6. Fetch shared contract → 200, has_changes=true, owner resolved
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/shared-contract?servicename=sc-svc&branch=feat-sc&api_type=openapi&path=/items&method=GET")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let info: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(info["has_changes"], true);
    assert_eq!(info["owner_service"], "sc-svc");
    assert_ne!(info["source_yaml"], info["current_yaml"]);
}

#[tokio::test]
async fn test_audit_logs_and_security() {
    let (app, token) = setup_app_with_admin().await;

    // 1. Unauthenticated requests should fail with 401
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/observability/audit-logs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/observability/audit-logs/export")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // 1.5 Enable local user registration first
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/local-users")
                .header("Authorization", format!("Bearer {}", token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "enabled": true
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // 2. Perform register action (should be logged with redacted username, no password)
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/register")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "username": "user123",
                        "password": "super-secret-password"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // 3. Perform a settings action (dev mode change)
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/dev-mode")
                .header("Authorization", format!("Bearer {}", token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "enabled": true
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // 4. Authenticated request for audit logs should succeed
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/observability/audit-logs")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let logs: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(logs.is_array());
    let arr = logs.as_array().unwrap();
    assert!(!arr.is_empty());

    // Verify the register user audit log has redacted actor/target, and NO raw password is in logs
    let reg_log = arr.iter().find(|l| l["action"] == "REGISTER_USER").unwrap();
    assert_eq!(reg_log["username"], "DevMode/Anonymous");
    assert!(reg_log["details"].as_str().unwrap().contains("u*****3")); // redacted user123
    assert!(
        !reg_log["details"]
            .as_str()
            .unwrap()
            .contains("super-secret-password")
    );

    // Verify dev mode setting log is registered with redacted actor username (admin -> a***n)
    let dev_log = arr.iter().find(|l| l["action"] == "SET_DEV_MODE").unwrap();
    assert_eq!(dev_log["username"], "a***n");
    assert_eq!(dev_log["details"], "Set dev-mode to true");

    // 5. Authenticated CSV export request should succeed
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/observability/audit-logs/export")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers().get("content-type").unwrap(), "text/csv");
    assert_eq!(
        response.headers().get("content-disposition").unwrap(),
        "attachment; filename=\"audit_logs.csv\""
    );

    let body_csv = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let csv_str = String::from_utf8(body_csv.to_vec()).unwrap();
    assert!(csv_str.starts_with("id,timestamp,username,action,details\n"));
    assert!(csv_str.contains("REGISTER_USER"));
    assert!(csv_str.contains("SET_DEV_MODE"));
}
