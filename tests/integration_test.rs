use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use chrono::Utc;
use sanshain_service::application::services;
use sanshain_service::domain::models::{AuthMode, ProvideResponse, SemVer};
use sanshain_service::domain::ports::SpecRepository;
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

/// Open the `ALLOW_INSECURE_DEV_MODE` safety gate once for this test binary.
/// Several tests here exercise the dev-mode auth bypass, which now requires this
/// gate. Tests that do not request dev mode stay locked regardless, so opening
/// the gate does not weaken their assertions. Set exactly once, synchronized, so
/// it is in place before any app handles a request.
fn ensure_dev_mode_gate_open() {
    use std::sync::Once;
    static GATE: Once = Once::new();
    GATE.call_once(|| unsafe {
        std::env::set_var("ALLOW_INSECURE_DEV_MODE", "true");
    });
}

fn test_app_state(repo: SqliteSpecRepository) -> AppState {
    ensure_dev_mode_gate_open();
    let mut tokens = HashMap::new();
    tokens.insert(
        TEST_CSRF_TOKEN.to_string(),
        Utc::now() + chrono::Duration::hours(1),
    );
    let (spec_updated_tx, _) = tokio::sync::broadcast::channel(100);

    AppState {
        repo: CachedSpecRepository::new(DatabaseRepo::Sqlite(repo), 256),
        db_url: "sqlite::memory:".to_string(),
        dev_user: None,
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
        max_body_bytes: sanshain_service::DEFAULT_MAX_BODY_BYTES,
        directory_roles: sanshain_service::application::directory_roles::DirectoryRoleCache::new(
            std::time::Duration::from_secs(300),
        ),
        root_users: std::sync::Arc::new(sanshain_service::domain::permissions::RootUsers::resolve(
            Some("root"),
            None,
        )),
    }
}

// `cfg(test)` is always true in this crate; the attribute marks the helpers as
// test code for clippy's `allow-unwrap-in-tests`.
#[cfg(test)]
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
#[cfg(test)]
async fn setup_app_dev_mode() -> axum::Router {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();

    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();
    services::ensure_initial_admin(&repo).await.unwrap();
    services::set_auth_mode(&repo, &AuthMode::Dev)
        .await
        .unwrap();
    let dev_user = services::ensure_dev_user(&repo).await.ok();

    let mut state = test_app_state(repo);
    state.dev_user = dev_user;
    create_app(state)
}

/// Setup app with initial admin, return app + admin token + repo
#[cfg(test)]
async fn setup_app_with_admin() -> (axum::Router, String, SqliteSpecRepository) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();

    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    // Create admin user manually with known password
    let hash = services::hash_password("admin-pass").unwrap();
    let user = repo.create_user("admin", &hash, true).await.unwrap();
    repo.grant_user_role(user.id, "admin")
        .await
        .expect("admin grant");
    use sanshain_service::domain::ports::SpecRepository;
    let session = repo
        .create_session(user.id, "2099-12-31T23:59:59")
        .await
        .unwrap();

    services::set_auth_mode(&repo, &AuthMode::Local)
        .await
        .unwrap();

    let app = create_app(test_app_state(repo.clone()));
    (app, session.token, repo)
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

    assert!(html.contains("href=\"/producers.html\" class=\"text-white hover:text-indigo-100 transition-colors\">Producers</a>"));
    assert!(html.contains("href=\"/consumers.html\" class=\"text-white hover:text-indigo-100 transition-colors\">Consumers</a>"));
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
        "producername": "test-service",
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
                .uri("/require?consumername=client-a&producername=test-service&branch=main&path=/users&method=GET")
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
        "producername": "test-service",
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
        "consumername": "java-client",
        "producername": "test-service",
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
        "consumername": "java-client",
        "producername": "test-service",
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
    // The branch published a spec without this endpoint, so it is deliberately
    // not part of that branch's API: 410 Gone, not 404. See
    // docs/adr/0001-endpoint-resolution-model.md.
    assert_eq!(response.status(), StatusCode::GONE);
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
        "consumername": "java-client",
        "producername": "test-service",
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
        "producername": "etag-svc",
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
        "consumername": "etag-client",
        "producername": "etag-svc",
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
        "consumername": "etag-client",
        "producername": "etag-svc",
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
async fn test_require_returns_304_when_if_none_match_matches() {
    let app = setup_app_dev_mode().await;

    // Provide a spec with a single endpoint
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
        "producername": "etag-304-svc",
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

    // First require: expect 200 and capture the ETag
    let resp1: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require?consumername=etag-client&producername=etag-304-svc&branch=main&path=/users&method=GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp1.status(), StatusCode::OK);
    let etag = resp1
        .headers()
        .get("etag")
        .expect("first response should carry an ETag")
        .to_str()
        .unwrap()
        .to_string();

    // Second require with a matching If-None-Match: expect 304 Not Modified and an empty body
    let resp2: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require?consumername=etag-client&producername=etag-304-svc&branch=main&path=/users&method=GET")
                .header("If-None-Match", &etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp2.status(), StatusCode::NOT_MODIFIED);
    let body = axum::body::to_bytes(resp2.into_body(), 100_000)
        .await
        .unwrap();
    assert!(body.is_empty(), "304 response must have an empty body");
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
        "producername": "test-service",
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
        "producername": "test-service",
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
        "producername": "test-service",
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
        "producername": "test-service",
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
        "producername": "test-service",
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
    let (app, token, _) = setup_app_with_admin().await;

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
                        "producername": "svc",
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
                        "producername": "svc",
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
                .uri("/require?consumername=client&producername=svc&branch=feature/test&path=/users&method=GET")
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
    let (app, token, _) = setup_app_with_admin().await;

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
                        "producername": "svc1",
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
                .uri("/require?consumername=client1&producername=svc1&branch=main&path=/users&method=GET")
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
                .uri("/admin/producers")
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
                .uri("/admin/producers/svc1/branches")
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
                .uri("/admin/consumers")
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
                .uri("/admin/consumers/client1")
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
                .uri("/admin/consumers/client1")
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
                .uri("/admin/producers/svc1")
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
                .uri("/admin/producers")
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
                .uri("/require?consumername=c&producername=s&branch=b&path=/p&method=GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_auth_login_and_session() {
    let (app, token, _) = setup_app_with_admin().await;

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
    // Login answers with a token only; what the caller may do comes from
    // /auth/me, which can express a partial administrator.
    assert!(login_resp["token"].is_string());
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
    assert_eq!(me["roles"], serde_json::json!(["admin"]));

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
    let (app, token, _) = setup_app_with_admin().await;

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
    let user = repo.create_user("root", &hash, true).await.unwrap();
    repo.grant_user_role(user.id, "admin")
        .await
        .expect("admin grant");

    use sanshain_service::domain::ports::SpecRepository;
    let session = repo
        .create_session(user.id, "2099-12-31T23:59:59")
        .await
        .unwrap();

    services::set_auth_mode(&repo, &AuthMode::Local)
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
    let (app, admin_token, _) = setup_app_with_admin().await;

    let enable_local_users: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/admin/auth-config")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({ "auth_mode": "local", "ldap_config": null }))
                        .unwrap(),
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
    let (app, admin_token, _) = setup_app_with_admin().await;

    // Enable local users
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/admin/auth-config")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({"auth_mode": "local", "ldap_config": null}))
                        .unwrap(),
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
    assert!(login_resp["token"].is_string());

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

    // 3. Staff should be able to see discovery routes (e.g. /admin/producers)
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/producers")
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
                .uri("/admin/producers/any-service")
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
    let (app, token, _) = setup_app_with_admin().await;

    // API should be locked by default
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require?consumername=c&producername=s&branch=b&path=/p&method=GET")
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
                .uri("/require?consumername=c&producername=s&branch=b&path=/p&method=GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_user_registration_and_approval() {
    let (app, token, repo) = setup_app_with_admin().await;

    // Registration should fail when local users disabled (default)
    services::set_auth_mode(&repo, &AuthMode::Disabled)
        .await
        .unwrap();

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
                .method("PUT")
                .uri("/admin/auth-config")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({"auth_mode": "local", "ldap_config": null}))
                        .unwrap(),
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
    let (app, token, _) = setup_app_with_admin().await;

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
    // "san_" + 32 CSPRNG bytes hex-encoded
    assert_eq!(api_token.len(), 68);
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

// Regression test: API tokens must only be accepted from the `Authorization`
// header. A token supplied via a `?token=` query parameter must be rejected, so
// long-lived credentials cannot leak through logs, history, or referrers.
#[tokio::test]
async fn test_api_token_query_param_is_rejected() {
    let (app, admin_token, _) = setup_app_with_admin().await;

    // Create a valid API token (auth mode is Local, so dev-mode bypass is off).
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/tokens")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({ "name": "ci", "expires_in_days": 30 })).unwrap(),
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

    // 1. A valid token in the query string must NOT authenticate.
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/report?branch=main&token={}", api_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // 2. The same token in the Authorization header still works.
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

    // 3. No credentials at all are rejected.
    let response: Response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/report?branch=main")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_auth_config_api() {
    let (app, token, _) = setup_app_with_admin().await;

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
    let (app, repo) = setup_app().await;
    repo.set_setting("auth_mode", "disabled").await.unwrap();

    // By default, the app is in "disabled" mode, so calling /provide should return 503 Service Unavailable
    let provide_payload = json!({
        "producername": "test-service",
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
                .uri("/require?consumername=client-a&producername=test-service&branch=main&path=/&method=GET")
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
        "producername": "compress-test-service",
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
                .uri("/require?consumername=gzip-client&producername=compress-test-service&branch=main&path=/users&method=GET&timeout=1")
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
        "producername": "gzip-request-test",
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
    let (app, admin_token, _) = setup_app_with_admin().await;

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
    let (app, admin_token, _) = setup_app_with_admin().await;

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
    let (app, admin_token, _) = setup_app_with_admin().await;

    // A client requires an endpoint from a service that was never provided
    let _require_payload = json!({
        "consumername": "my-client",
        "producername": "phantom-service",
        "branch": "main",
        "path": "/health",
        "method": "GET"
    });

    let response: Response = app.clone()
        .oneshot(
            Request::builder()
                .uri("/require?consumername=my-client&producername=phantom-service&branch=main&path=/health&method=GET")
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
                .uri("/admin/producers")
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
    let (app, admin_token, _) = setup_app_with_admin().await;

    // Upload a spec so the service exists
    let yaml = "openapi: '3.0.0'\ninfo:\n  title: Svc\n  version: '1.0'\npaths:\n  /health:\n    get:\n      operationId: getHealth\n      responses:\n        '200':\n          description: OK\n";
    let provide_payload = json!({
        "producername": "temp-service",
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
                .uri("/require?consumername=orphan-client&producername=temp-service&branch=main&path=/health&method=GET")
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
                .uri("/admin/consumers")
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
                .uri("/admin/producers/temp-service")
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
                .uri("/admin/consumers")
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
        "producername": "err-svc",
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
                .uri("/require?consumername=test-client&producername=err-svc&branch=main&path=/missing&method=GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // The branch published a spec without this endpoint, so it is deliberately
    // not part of that branch's API: 410 Gone, not 404. See
    // docs/adr/0001-endpoint-resolution-model.md.
    assert_eq!(response.status(), StatusCode::GONE);
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
    let payload =
        json!({ "producername": "conflict-svc", "branch": "main", "openapi_yaml": yaml1 });
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
        json!({ "producername": "conflict-svc", "branch": "main", "openapi_yaml": yaml2 });
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
    let payload = json!({ "producername": "dry-svc", "branch": "main", "openapi_yaml": yaml, "dry_run": true });
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
                .uri("/require?consumername=dry-client&producername=dry-svc&branch=main&path=/items&method=GET")
                .body(Body::empty())
                .unwrap(),
        ).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_require_dry_run_does_not_create_dependency() {
    let (app, admin_token, _) = setup_app_with_admin().await;

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
    let payload = json!({ "producername": "dryreq-svc", "branch": "main", "openapi_yaml": yaml });
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
                .uri("/require?consumername=dry-client&producername=dryreq-svc&branch=main&path=/health&method=GET&dry_run=true")
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
                .uri("/admin/consumers")
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
    let (app, admin_token, _) = setup_app_with_admin().await;

    // 1. Provide OpenAPI
    let openapi_payload = json!({
        "producername": "multi-svc",
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
        "producername": "multi-svc",
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
        "producername": "multi-svc",
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
                .uri("/admin/producers/multi-svc/branches/main/endpoints")
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
    let (app, admin_token, _) = setup_app_with_admin().await;

    // Provide a spec with only /users
    let yaml = "openapi: '3.0.0'\ninfo:\n  title: Svc\n  version: '1.0'\npaths:\n  /users:\n    get:\n      operationId: getUsers\n      responses:\n        '200':\n          description: OK\n";
    let provide_payload = json!({
        "producername": "missing-ep-svc",
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
                .uri("/require?consumername=missing-client&producername=missing-ep-svc&branch=main&path=/nonexistent&method=GET&timeout=0")
                .header("Authorization", format!("Bearer {}", admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // Should get 404 since endpoint doesn't exist
    // The branch published a spec without this endpoint, so it is deliberately
    // not part of that branch's API: 410 Gone, not 404. See
    // docs/adr/0001-endpoint-resolution-model.md.
    assert_eq!(response.status(), StatusCode::GONE);

    // Client should still appear in the clients list
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/consumers")
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
    let (app, admin_token, _) = setup_app_with_admin().await;

    // Provide a spec
    let yaml = "openapi: '3.0.0'\ninfo:\n  title: Svc\n  version: '1.0'\npaths:\n  /users:\n    get:\n      operationId: getUsers\n      responses:\n        '200':\n          description: OK\n";
    let provide_payload = json!({
        "producername": "dedup-svc",
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
                    .uri("/require?consumername=dedup-client&producername=dedup-svc&branch=main&path=/missing&method=GET&timeout=0")
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
    let (app, admin_token, _) = setup_app_with_admin().await;

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
                        "producername": "nuke-svc",
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
                .uri("/require?consumername=nuke-client&producername=nuke-svc&branch=main&path=/items&method=GET&timeout=0")
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
                .uri("/admin/nuke/producers")
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
                .uri("/admin/nuke/producers")
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
                .uri("/admin/producers")
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
                .uri("/admin/nuke/consumers")
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
                .uri("/admin/consumers")
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
async fn test_feature_branch_accepts_breaking_changes_without_force() {
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

    // 0. Seed the service on the protected branch so this is not a brand-new service.
    let payload_seed =
        json!({ "producername": "alpha-svc", "branch": "main", "openapi_yaml": yaml_v1 });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload_seed).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // 1. Provide v1 on a feature branch.
    let payload_v1 =
        json!({ "producername": "alpha-svc", "branch": "feat-breaking", "openapi_yaml": yaml_v1 });
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

    // 2. A breaking change on the feature branch is accepted without `force`.
    let payload_breaking = json!({ "producername": "alpha-svc", "branch": "feat-breaking", "openapi_yaml": yaml_v2_breaking });
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
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // 3. The same breaking change on the protected branch is still rejected.
    let payload_breaking_main =
        json!({ "producername": "alpha-svc", "branch": "main", "openapi_yaml": yaml_v2_breaking });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&payload_breaking_main).unwrap(),
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
    assert!(body_str.contains("Breaking changes detected"));
}

#[tokio::test]
async fn test_problem_3_optimistic_concurrency_integration() {
    let app = setup_app_dev_mode().await;
    // These differ by an endpoint, not just by `info.title`. The subject here is
    // optimistic concurrency, which needs the version to actually move so that
    // step 3's stale `base_version` conflicts. Since issue #6 a document-level
    // edit with no endpoint effect deliberately does not bump the version, so
    // the original `paths: {}` pair could no longer drive this test.
    let yaml1 = "openapi: 3.0.0\ninfo:\n  title: T\n  version: 1.0.0\npaths:\n  /a:\n    get:\n      responses:\n        '200':\n          description: OK";
    let yaml2 = "openapi: 3.0.0\ninfo:\n  title: T\n  version: 1.0.0\npaths:\n  /a:\n    get:\n      responses:\n        '200':\n          description: Fine";

    // 1. First provide
    let payload1 = json!({ "producername": "svc", "branch": "main", "openapi_yaml": yaml1 });
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
    assert_eq!(res1.version, SemVer::new(1, 0, 0));

    // 2. Second provide with correct base_version
    let payload2 = json!({ "producername": "svc", "branch": "main", "openapi_yaml": yaml2, "base_version": "1.0.0" });
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
    assert_eq!(res2.version, SemVer::new(1, 0, 1));

    // 3. Third provide with OUTDATED base_version
    let payload3 = json!({ "producername": "svc", "branch": "main", "openapi_yaml": yaml1, "base_version": "1.0.0" });
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
    let (app, admin_token, _) = setup_app_with_admin().await;

    // --- Setup: create a staff user and get their token ---
    // Enable local users
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/admin/auth-config")
                .header("Authorization", format!("Bearer {}", admin_token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({"auth_mode": "local", "ldap_config": null}))
                        .unwrap(),
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
        ("DELETE", "/admin/producers/any-service", None),
        ("DELETE", "/admin/producers/any-service/branches/main", None),
        ("DELETE", "/admin/consumers/any-client", None),
        // Settings (POST = write)
        (
            "POST",
            "/admin/settings/dev-mode",
            Some(json!({"enabled": false})),
        ),
        (
            "PUT",
            "/admin/auth-config",
            Some(json!({"auth_mode": "disabled", "ldap_config": null})),
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
        ("POST", "/admin/nuke/producers", None),
        ("POST", "/admin/nuke/consumers", None),
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
        "/admin/producers",
        "/admin/consumers",
        "/admin/observability/stats",
        "/admin/observability/logs",
        "/admin/endpoint-yaml?producername=x&branch=main&path=/p&method=GET",
        "/admin/endpoint-versions?producername=x&branch=main&path=/p&method=GET",
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
        ("GET", "/admin/producers"),
        ("DELETE", "/admin/producers/x"),
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
                        "producername": "merge-svc",
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
                .uri("/require?consumername=merge-client&producername=merge-svc&branch=main&path=/users&method=GET")
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
                        "producername": "feature-svc",
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
                .uri("/require?consumername=feature-client&producername=feature-svc&branch=feature&path=/orders&method=POST")
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
async fn test_audit_logs_and_security() {
    let (app, token, _) = setup_app_with_admin().await;

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
                .method("PUT")
                .uri("/admin/auth-config")
                .header("Authorization", format!("Bearer {}", token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "auth_mode": "local",
                        "ldap_config": null
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
    assert!(reg_log["details"].as_str().unwrap().contains("user123"));
    assert!(
        !reg_log["details"]
            .as_str()
            .unwrap()
            .contains("super-secret-password")
    );

    // Verify dev mode setting log is registered with full actor username (admin)
    let dev_log = arr.iter().find(|l| l["action"] == "SET_DEV_MODE").unwrap();
    assert_eq!(dev_log["username"], "admin");
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
    assert!(
        csv_str.starts_with("id,timestamp,username,action,details,service,branch,action_type\n")
    );
    assert!(csv_str.contains("REGISTER_USER"));
    assert!(csv_str.contains("SET_DEV_MODE"));
}

#[tokio::test]
async fn test_endpoint_version_metadata() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();
    services::ensure_initial_admin(&repo).await.unwrap();
    services::set_auth_mode(&repo, &AuthMode::Dev)
        .await
        .unwrap();
    let dev_user = services::ensure_dev_user(&repo).await.ok();
    let mut state = test_app_state(repo.clone());
    state.dev_user = dev_user;
    let app = create_app(state);

    // 1. Protect the branch 'main'
    use sanshain_service::domain::ports::SpecRepository;
    repo.add_protected_branch("main").await.unwrap();

    // 2. Upload initial spec using /provide (unauthenticated because in DevMode/Anonymous)
    let spec1 = r#"
openapi: 3.0.0
info:
  title: Test Service
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
                        "producername": "UserService",
                        "branch": "main",
                        "openapi_yaml": spec1
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // 3. Setup with admin token to upload second spec under authenticated admin
    // Let's create an admin session
    let session = repo.create_session(1, "2099-12-31T23:59:59").await.unwrap();
    let token = session.token;

    // Change auth mode to Local to require auth
    services::set_auth_mode(&repo, &AuthMode::Local)
        .await
        .unwrap();

    // Upload an updated spec as 'admin'
    let spec2 = r#"
openapi: 3.0.0
info:
  title: Test Service
  version: 1.0.0
paths:
  /users:
    get:
      description: Get all users
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
                .header("Authorization", format!("Bearer {}", token))
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "producername": "UserService",
                        "branch": "main",
                        "openapi_yaml": spec2
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    // 4. Query /admin/endpoint-versions to verify metadata exists
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/endpoint-versions?producername=UserService&branch=main&api_type=openapi&path=/users&method=GET")
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
    let versions: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(versions.is_array());
    let arr = versions.as_array().unwrap();
    assert_eq!(arr.len(), 2);

    // Version 1 was created anonymously in Dev mode (should fall back to "DevMode/Anonymous" or "anonymous" username)
    assert_eq!(arr[0]["version"], 1);
    assert!(
        arr[0]["username"] == "DevMode/Anonymous"
            || arr[0]["username"] == "anonymous"
            || arr[0]["username"] == "dev_user"
            || arr[0]["username"].is_null()
    );
    assert_eq!(arr[0]["source_branch"], "main");

    // Version 2 was created by admin (full name root)
    assert_eq!(arr[1]["version"], 2);
    assert_eq!(arr[1]["username"], "root");
    assert_eq!(arr[1]["source_branch"], "main");
}

#[tokio::test]
async fn test_branches_metadata_endpoint() {
    let (app, token, repo) = setup_app_with_admin().await;

    // Populate branches with metadata using repo
    let s_id = repo.ensure_service("test-service").await.unwrap();
    repo.ensure_branch(s_id, "branch-1").await.unwrap();
    repo.ensure_branch(s_id, "branch-2").await.unwrap();

    // Query /branches/metadata (authenticated)
    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/branches/metadata")
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
    let res: Value = serde_json::from_slice(&body).unwrap();
    assert!(res.is_array());
    let arr = res.as_array().unwrap();
    assert_eq!(arr.len(), 2);

    let names: Vec<String> = arr
        .iter()
        .map(|item| item["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"branch-1".to_string()));
    assert!(names.contains(&"branch-2".to_string()));
}

/// Request bodies above the configured `max_body_bytes` limit must be rejected
/// with `413 Payload Too Large`, while payloads under the limit keep working.
#[tokio::test]
async fn test_request_body_over_limit_is_rejected_under_limit_accepted() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();
    services::ensure_initial_admin(&repo).await.unwrap();
    services::set_auth_mode(&repo, &AuthMode::Dev)
        .await
        .unwrap();
    let dev_user = services::ensure_dev_user(&repo).await.ok();

    let mut state = test_app_state(repo);
    state.dev_user = dev_user;
    state.max_body_bytes = 1024;
    let app = create_app(state);

    // Over the limit: the request is rejected before the spec is parsed.
    let oversized_payload = json!({
        "producername": "test-service",
        "branch": "main",
        "openapi_yaml": "x".repeat(2048),
    });
    let oversized_body = serde_json::to_vec(&oversized_payload).unwrap();
    assert!(oversized_body.len() > 1024);

    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(oversized_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);

    // Under the limit: a normal provide payload still succeeds.
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
        "producername": "test-service",
        "branch": "main",
        "openapi_yaml": openapi_yaml
    });
    let under_limit_body = serde_json::to_vec(&provide_payload).unwrap();
    assert!(under_limit_body.len() < 1024);

    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(under_limit_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
}

// --- Backward compat: legacy `servicename`/`clientname` field names (issue #4) ---
//
// 1.6.0 renamed servicename -> producername and clientname -> consumername on
// every /provide* and /require* payload, described as breaking with no aliases.
// Every known client (Go/JS/Rust/Conan/Maven) still sends the old names, so this
// restores acceptance of both, with no other behavior change.
//
// Each test sends the legacy name on one side of a round trip and the current
// name on the other, so a pass proves the alias resolved to the *same* producer
// or consumer — not merely that the payload deserialized.

/// Read a response body as UTF-8, for asserting on the returned snippet.
/// A helper outside a `#[test]` fn, so it must not `unwrap`: an unreadable body
/// becomes a marker string that fails the caller's `contains` assertion with a
/// readable message.
async fn body_text(response: Response) -> String {
    match axum::body::to_bytes(response.into_body(), 1_000_000).await {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(e) => format!("<unreadable body: {e}>"),
    }
}

#[tokio::test]
async fn legacy_servicename_on_provide_stores_under_producername() {
    let app = setup_app_dev_mode().await;

    let openapi_yaml = r#"
openapi: 3.0.0
info:
  title: Legacy Test API
  version: 1.0.0
paths:
  /legacy:
    get:
      responses:
        '200':
          description: OK
"#;
    let payload = json!({
        "servicename": "legacy-provide-svc",
        "branch": "main",
        "openapi_yaml": openapi_yaml
    });

    let response = app
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

    // Ask for it back under the *current* name: proves `servicename` landed in
    // `producername` rather than in some other field.
    let require_res = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require?consumername=legacy-consumer&producername=legacy-provide-svc&branch=main&path=/legacy&method=GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(require_res.status(), StatusCode::OK);
    let yaml = body_text(require_res).await;
    assert!(yaml.contains("/legacy"), "unexpected snippet: {yaml}");
}

#[tokio::test]
async fn legacy_servicename_on_provide_asyncapi_stores_under_producername() {
    let app = setup_app_dev_mode().await;

    let asyncapi_yaml = r#"
asyncapi: 2.6.0
info:
  title: Legacy Async API
  version: 1.0.0
channels:
  legacy/channel:
    publish:
      message:
        payload:
          type: object
"#;
    let payload = json!({
        "servicename": "legacy-async-svc",
        "branch": "main",
        "asyncapi_yaml": asyncapi_yaml
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide/asyncapi")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::ACCEPTED);

    let require_res = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require/asyncapi?consumername=legacy-consumer&producername=legacy-async-svc&branch=main&path=legacy/channel&method=PUB")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(require_res.status(), StatusCode::OK);
    let yaml = body_text(require_res).await;
    assert!(
        yaml.contains("legacy/channel"),
        "unexpected snippet: {yaml}"
    );
}

#[tokio::test]
async fn legacy_servicename_on_provide_proto_stores_under_producername() {
    let app = setup_app_dev_mode().await;

    let proto_content = r#"syntax = "proto3";
package legacy;

service LegacyService {
  rpc Ping (PingRequest) returns (PingResponse);
}

message PingRequest {}
message PingResponse {}
"#;
    let payload = json!({
        "servicename": "legacy-proto-svc",
        "branch": "main",
        "proto_content": proto_content
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide/grpc")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::ACCEPTED);

    let require_res = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require/grpc?consumername=legacy-consumer&producername=legacy-proto-svc&branch=main&path=LegacyService&method=Ping")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(require_res.status(), StatusCode::OK);
    let proto = body_text(require_res).await;
    assert!(proto.contains("Ping"), "unexpected snippet: {proto}");
}

#[tokio::test]
async fn legacy_clientname_and_servicename_accepted_on_require() {
    let app = setup_app_dev_mode().await;

    let openapi_yaml = r#"
openapi: 3.0.0
info:
  title: Legacy Require API
  version: 1.0.0
paths:
  /legacy-require:
    get:
      responses:
        '200':
          description: OK
"#;
    let provide_payload = json!({
        "producername": "legacy-require-svc",
        "branch": "main",
        "openapi_yaml": openapi_yaml
    });
    let provide_res = app
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
    assert_eq!(provide_res.status(), StatusCode::ACCEPTED);

    let require_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require?clientname=legacy-client&servicename=legacy-require-svc&branch=main&path=/legacy-require&method=GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(require_res.status(), StatusCode::OK);
    let yaml = body_text(require_res).await;
    assert!(
        yaml.contains("/legacy-require"),
        "unexpected snippet: {yaml}"
    );

    // The legacy `clientname` must record the dependency under that consumer,
    // not under an empty or defaulted name.
    let consumers_res = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/consumers")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(consumers_res.status(), StatusCode::OK);
    let consumers = body_text(consumers_res).await;
    assert!(
        consumers.contains("legacy-client"),
        "consumer not recorded: {consumers}"
    );
}

#[tokio::test]
async fn legacy_names_accepted_on_require_asyncapi() {
    let app = setup_app_dev_mode().await;

    let asyncapi_yaml = r#"
asyncapi: 2.6.0
info:
  title: Legacy Async Require API
  version: 1.0.0
channels:
  legacy/require-channel:
    publish:
      message:
        payload:
          type: object
"#;
    let provide_payload = json!({
        "producername": "legacy-async-require-svc",
        "branch": "main",
        "asyncapi_yaml": asyncapi_yaml
    });
    let provide_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide/asyncapi")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&provide_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(provide_res.status(), StatusCode::ACCEPTED);

    let require_res = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require/asyncapi?clientname=legacy-async-client&servicename=legacy-async-require-svc&branch=main&path=legacy/require-channel&method=PUB")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(require_res.status(), StatusCode::OK);
    let yaml = body_text(require_res).await;
    assert!(
        yaml.contains("legacy/require-channel"),
        "unexpected snippet: {yaml}"
    );
}

#[tokio::test]
async fn legacy_names_accepted_on_require_grpc() {
    let app = setup_app_dev_mode().await;

    let proto_content = r#"syntax = "proto3";
package legacy;

service LegacyRequireService {
  rpc Echo (EchoRequest) returns (EchoResponse);
}

message EchoRequest {}
message EchoResponse {}
"#;
    let provide_payload = json!({
        "producername": "legacy-proto-require-svc",
        "branch": "main",
        "proto_content": proto_content
    });
    let provide_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide/grpc")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&provide_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(provide_res.status(), StatusCode::ACCEPTED);

    let require_res = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require/grpc?clientname=legacy-proto-client&servicename=legacy-proto-require-svc&branch=main&path=LegacyRequireService&method=Echo")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(require_res.status(), StatusCode::OK);
    let proto = body_text(require_res).await;
    assert!(proto.contains("Echo"), "unexpected snippet: {proto}");
}

#[tokio::test]
async fn legacy_clientname_and_servicename_accepted_on_require_bundle() {
    let app = setup_app_dev_mode().await;

    let openapi_yaml = r#"
openapi: 3.0.0
info:
  title: Legacy Bundle API
  version: 1.0.0
paths:
  /legacy-bundle-a:
    get:
      responses:
        '200':
          description: OK
  /legacy-bundle-b:
    get:
      responses:
        '200':
          description: OK
"#;
    let provide_payload = json!({
        "producername": "legacy-bundle-svc",
        "branch": "main",
        "openapi_yaml": openapi_yaml
    });
    let provide_res = app
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
    assert_eq!(provide_res.status(), StatusCode::ACCEPTED);

    let bundle_payload = json!({
        "clientname": "legacy-bundle-client",
        "servicename": "legacy-bundle-svc",
        "branch": "main",
        "endpoints": [
            { "path": "/legacy-bundle-a", "method": "GET" },
            { "path": "/legacy-bundle-b", "method": "GET" }
        ]
    });
    let bundle_res = app
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

    assert_eq!(bundle_res.status(), StatusCode::OK);
    let yaml = body_text(bundle_res).await;
    assert!(
        yaml.contains("/legacy-bundle-a") && yaml.contains("/legacy-bundle-b"),
        "unexpected bundle: {yaml}"
    );
}

#[tokio::test]
async fn both_legacy_and_current_names_rejected_on_provide() {
    let app = setup_app_dev_mode().await;

    // serde's `alias` makes the two spellings the *same* field, so supplying
    // both is a duplicate field rather than a precedence question. Documented in
    // docs/api-usage.md: send one or the other.
    let payload = json!({
        "producername": "dup-svc",
        "servicename": "dup-svc-other",
        "branch": "main",
        "openapi_yaml": "openapi: 3.0.0\ninfo:\n  title: Dup\n  version: 1.0.0\npaths: {}\n"
    });

    let response = app
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

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let err = body_text(response).await;
    assert!(
        err.contains("duplicate field") && err.contains("producername"),
        "expected a duplicate-field error, got: {err}"
    );
}

#[tokio::test]
async fn both_legacy_and_current_names_rejected_on_require() {
    let app = setup_app_dev_mode().await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require?consumername=dup-client&clientname=dup-client-other&producername=dup-svc&branch=main&path=/x&method=GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let err = body_text(response).await;
    assert!(
        err.contains("duplicate field") && err.contains("consumername"),
        "expected a duplicate-field error, got: {err}"
    );
}

// --- Audit timeline is administrator-only (issue #5) ---------------------------
//
// The timeline records the Actor behind every state-changing request, across
// every Producer. Access is declared once, on the route, so these assert the
// status a given credential receives rather than reaching into the middleware.

/// Credentials for the audit-access tests.
///
/// Named fields rather than a tuple: with three same-typed token strings,
/// positional destructuring would let the admin and non-admin credentials be
/// swapped silently, and the test would still compile while asserting the
/// opposite of what it claims.
#[cfg(test)]
struct AuditAccessFixture {
    app: axum::Router,
    admin_session: String,
    user_session: String,
    /// API token owned by the non-admin user.
    api_token: String,
    /// The action of the single audit entry seeded below.
    seeded_action: &'static str,
}

#[cfg(test)]
async fn setup_audit_access_fixture() -> AuditAccessFixture {
    use sanshain_service::domain::ports::{NewAuditLog, SpecRepository};

    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    let hash = services::hash_password("admin-pass").unwrap();
    let admin = repo.create_user("admin", &hash, true).await.unwrap();
    repo.grant_user_role(admin.id, "admin")
        .await
        .expect("admin grant");
    let admin_session = repo
        .create_session(admin.id, "2099-12-31T23:59:59")
        .await
        .unwrap();

    let hash = services::hash_password("user-pass").unwrap();
    let user = repo.create_user("plain", &hash, true).await.unwrap();
    let user_session = repo
        .create_session(user.id, "2099-12-31T23:59:59")
        .await
        .unwrap();

    // Returns (id, raw_token) — the second element is the credential.
    let (_, api_token) = services::create_api_token(&repo, user.id, "ci", 30)
        .await
        .unwrap();

    // Seed one entry, so "the payload is unchanged" is asserted against actual
    // content rather than against an empty array that would satisfy any shape.
    let seeded_action = "PROVIDE_SPEC";
    repo.insert_audit_log(
        "admin",
        NewAuditLog {
            action: seeded_action,
            details: "seeded for the audit access tests",
            service: Some("orders"),
            branch: Some("main"),
            action_type: Some(seeded_action),
            diff: None,
        },
    )
    .await
    .unwrap();

    services::set_auth_mode(&repo, &AuthMode::Local)
        .await
        .unwrap();

    let app = create_app(test_app_state(repo));
    AuditAccessFixture {
        app,
        admin_session: admin_session.token,
        user_session: user_session.token,
        api_token,
        seeded_action,
    }
}

// `cfg(test)` is always true here; the attribute marks the helper as test code
// for clippy's `allow-unwrap-in-tests`, matching the idiom in middleware_test.rs.
#[cfg(test)]
async fn audit_timeline_status(app: axum::Router, authorization: Option<&str>) -> StatusCode {
    let mut builder = Request::builder().uri("/api/audit/timeline?limit=5");
    if let Some(value) = authorization {
        builder = builder.header("Authorization", value);
    }
    app.oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn audit_timeline_serves_an_admin_session_unchanged() {
    // Both that an administrator is allowed through, and that the payload they
    // get is the same one as before — asserted against a seeded entry, since an
    // empty array would satisfy any shape check.
    let f = setup_audit_access_fixture().await;

    let res = f
        .app
        .oneshot(
            Request::builder()
                .uri("/api/audit/timeline?limit=5")
                .header("Authorization", format!("Bearer {}", f.admin_session))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    let entries = json.as_array().expect("the timeline is a JSON array");
    assert_eq!(
        entries.len(),
        1,
        "the seeded entry is returned, got: {json}"
    );
    assert_eq!(entries[0]["action"], f.seeded_action);
    assert_eq!(entries[0]["service"], "orders");
    assert_eq!(entries[0]["branch"], "main");
    assert_eq!(entries[0]["username"], "admin");
}

#[tokio::test]
async fn audit_timeline_refuses_a_non_admin_session() {
    let f = setup_audit_access_fixture().await;

    let status = audit_timeline_status(f.app, Some(&format!("Bearer {}", f.user_session))).await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "an authenticated non-administrator must not read the audit trail"
    );
}

#[tokio::test]
async fn audit_timeline_refuses_an_api_token_that_is_otherwise_valid() {
    // Deliberate: the administrator middleware validates sessions only, so no
    // token reaches the timeline. Nothing outside api.yaml is consumed as a
    // REST API, and this route is outside it.
    //
    // The token is first shown to work on a route that still accepts tokens, so
    // this cannot pass merely because the credential was malformed — which is
    // exactly how an earlier version of this test passed for the wrong reason.
    let f = setup_audit_access_fixture().await;
    let bearer = format!("Bearer {}", f.api_token);

    let control = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/branches/protected")
                .header("Authorization", &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        control.status(),
        StatusCode::OK,
        "positive control: the API token must be a working credential elsewhere"
    );

    let status = audit_timeline_status(f.app, Some(&bearer)).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn audit_timeline_refuses_anonymous_callers() {
    let f = setup_audit_access_fixture().await;

    let status = audit_timeline_status(f.app, None).await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// --- Version moves only when the API moved (issue #6) --------------------------
//
// A Provide that changes no endpoint must leave the version alone: no bump, no
// record written, no update broadcast. Asserted through the same interface a
// Producer uses, on the version string a Consumer would observe.

#[cfg(test)]
const ISSUE6_OPENAPI: &str = r#"
openapi: 3.0.0
info:
  title: Orders API
  version: 1.0.0
paths:
  /orders:
    get:
      responses:
        '200':
          description: OK
"#;

#[cfg(test)]
const ISSUE6_ASYNCAPI: &str = r#"
asyncapi: 2.6.0
info:
  title: Orders Events
  version: 1.0.0
channels:
  order/created:
    publish:
      message:
        name: OrderCreated
        payload:
          type: object
          properties:
            id:
              type: string
"#;

/// POST a Provide and return (status, version string).
#[cfg(test)]
async fn provide_and_read_version(
    app: &axum::Router,
    uri: &str,
    payload: Value,
) -> (StatusCode, String) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let version = json
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    (status, version)
}

#[cfg(test)]
fn issue6_openapi_payload(yaml: &str) -> Value {
    json!({ "producername": "orders", "branch": "main", "openapi_yaml": yaml })
}

#[tokio::test]
async fn identical_reprovide_does_not_bump_the_version() {
    let app = setup_app_dev_mode().await;
    let payload = issue6_openapi_payload(ISSUE6_OPENAPI);

    let (status, first) = provide_and_read_version(&app, "/provide", payload.clone()).await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let (status, second) = provide_and_read_version(&app, "/provide", payload).await;
    assert_eq!(status, StatusCode::ACCEPTED);

    assert_eq!(
        second, first,
        "re-providing identical content must leave the version alone"
    );
}

#[tokio::test]
async fn reprovide_differing_only_in_formatting_does_not_bump_the_version() {
    let app = setup_app_dev_mode().await;

    let (_, first) =
        provide_and_read_version(&app, "/provide", issue6_openapi_payload(ISSUE6_OPENAPI)).await;

    // Same document, different bytes: reordered top-level keys, extra blank
    // lines, and a trailing comment. No endpoint is affected.
    let reformatted = r#"
paths:
  /orders:
    get:
      responses:
        '200':
          description: OK

info:
  version: 1.0.0
  title: Orders API

openapi: 3.0.0
"#;
    let (status, second) =
        provide_and_read_version(&app, "/provide", issue6_openapi_payload(reformatted)).await;
    assert_eq!(status, StatusCode::ACCEPTED);

    assert_eq!(
        second, first,
        "byte differences that leave every endpoint identical must not bump the version"
    );
}

#[tokio::test]
async fn alternating_openapi_and_asyncapi_provides_do_not_bump_each_other() {
    // The regression the shared version record caused: one row serves both API
    // types, so each Provide used to compare its fingerprint against the other
    // type's, never match, and bump. This is the case from the original report.
    let app = setup_app_dev_mode().await;

    let openapi = issue6_openapi_payload(ISSUE6_OPENAPI);
    let asyncapi = json!({
        "producername": "orders",
        "branch": "main",
        "asyncapi_yaml": ISSUE6_ASYNCAPI
    });

    // Establish both.
    provide_and_read_version(&app, "/provide", openapi.clone()).await;
    let (_, settled) = provide_and_read_version(&app, "/provide/asyncapi", asyncapi.clone()).await;

    // Now alternate several times with no content change at all.
    for _ in 0..3 {
        let (status, v) = provide_and_read_version(&app, "/provide", openapi.clone()).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(v, settled, "OpenAPI re-provide must not bump");

        let (status, v) =
            provide_and_read_version(&app, "/provide/asyncapi", asyncapi.clone()).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(v, settled, "AsyncAPI re-provide must not bump");
    }
}

#[tokio::test]
async fn adding_an_endpoint_still_bumps_the_minor_version() {
    let app = setup_app_dev_mode().await;

    let (_, first) =
        provide_and_read_version(&app, "/provide", issue6_openapi_payload(ISSUE6_OPENAPI)).await;
    assert_eq!(first, "1.0.0");

    let with_extra = r#"
openapi: 3.0.0
info:
  title: Orders API
  version: 1.0.0
paths:
  /orders:
    get:
      responses:
        '200':
          description: OK
  /invoices:
    get:
      responses:
        '200':
          description: OK
"#;
    let (status, second) =
        provide_and_read_version(&app, "/provide", issue6_openapi_payload(with_extra)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(second, "1.1.0", "an added endpoint is a minor change");
}

#[tokio::test]
async fn modifying_an_endpoint_still_bumps_the_patch_version() {
    let app = setup_app_dev_mode().await;

    let (_, first) =
        provide_and_read_version(&app, "/provide", issue6_openapi_payload(ISSUE6_OPENAPI)).await;
    assert_eq!(first, "1.0.0");

    // Same path and method, changed description: an endpoint modification.
    let modified = r#"
openapi: 3.0.0
info:
  title: Orders API
  version: 1.0.0
paths:
  /orders:
    get:
      responses:
        '200':
          description: All orders, newest first
"#;
    let (status, second) =
        provide_and_read_version(&app, "/provide", issue6_openapi_payload(modified)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(
        second, "1.0.1",
        "a modified endpoint is a patch change, got {second}"
    );
}

#[tokio::test]
async fn no_op_reprovide_writes_no_audit_entry_and_sends_no_update() {
    // Absence matters as much as the version: a build loop must not generate
    // write load or wake every connected browser.
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();
    services::ensure_initial_admin(&repo).await.unwrap();
    services::set_auth_mode(&repo, &AuthMode::Dev)
        .await
        .unwrap();
    let dev_user = services::ensure_dev_user(&repo).await.ok();

    let mut state = test_app_state(repo.clone());
    state.dev_user = dev_user;
    let mut updates = state.spec_updated_tx.subscribe();
    let app = create_app(state);

    let payload = issue6_openapi_payload(ISSUE6_OPENAPI);

    // First Provide: a real change, so it does notify.
    provide_and_read_version(&app, "/provide", payload.clone()).await;
    assert!(
        updates.try_recv().is_ok(),
        "a Provide that changes endpoints must notify listeners"
    );
    while updates.try_recv().is_ok() {}

    use sanshain_service::domain::ports::SpecRepository;
    let audit_before = repo
        .get_audit_logs(sanshain_service::domain::models::AuditLogFilter {
            from_date: None,
            to_date: None,
            action_type: None,
            service_wildcard: None,
            branch_wildcard: None,
            limit: 100,
        })
        .await
        .unwrap()
        .len();

    // Second Provide: identical, so nothing at all should happen.
    provide_and_read_version(&app, "/provide", payload).await;

    assert!(
        updates.try_recv().is_err(),
        "a no-op Provide must not broadcast a spec update"
    );

    let audit_after = repo
        .get_audit_logs(sanshain_service::domain::models::AuditLogFilter {
            from_date: None,
            to_date: None,
            action_type: None,
            service_wildcard: None,
            branch_wildcard: None,
            limit: 100,
        })
        .await
        .unwrap()
        .len();
    assert_eq!(
        audit_after, audit_before,
        "a no-op Provide must not write an audit entry"
    );
}

#[tokio::test]
async fn no_op_reprovide_does_not_rewrite_the_version_record() {
    // The returned SemVer staying put is not sufficient evidence. The version
    // row is upserted with `version = version + 1` and a fresh `updated_at`, so
    // a no-op that reached the write would bump that internal counter and touch
    // the timestamp while still reporting the same SemVer.
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool.clone());
    repo.run_migrations().await.unwrap();
    services::ensure_initial_admin(&repo).await.unwrap();
    services::set_auth_mode(&repo, &AuthMode::Dev)
        .await
        .unwrap();
    let dev_user = services::ensure_dev_user(&repo).await.ok();

    let mut state = test_app_state(repo.clone());
    state.dev_user = dev_user;
    let app = create_app(state);

    let payload = issue6_openapi_payload(ISSUE6_OPENAPI);
    provide_and_read_version(&app, "/provide", payload.clone()).await;

    let read_record = || async {
        sqlx::query_as::<_, (i64, String)>(
            "SELECT version, updated_at FROM service_spec_versions LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    };

    let before = read_record().await;

    // Re-provide the identical spec.
    provide_and_read_version(&app, "/provide", payload).await;

    let after = read_record().await;

    assert_eq!(
        after.0, before.0,
        "a no-op Provide must not bump the internal revision counter"
    );
    assert_eq!(
        after.1, before.1,
        "a no-op Provide must not touch updated_at"
    );
}

#[tokio::test]
async fn first_provide_establishes_a_version_even_with_no_endpoints() {
    // A spec with no paths produces no endpoint diff, so the no-op guard would
    // short-circuit it. The first Provide must still establish a version:
    // returning one that was never stored would break optimistic concurrency,
    // which sends the version back as `base_version`.
    let app = setup_app_dev_mode().await;
    let empty = "openapi: 3.0.0\ninfo:\n  title: Empty\n  version: 1.0.0\npaths: {}";

    let (status, version) =
        provide_and_read_version(&app, "/provide", issue6_openapi_payload(empty)).await;

    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(version, "1.0.0", "the first Provide establishes 1.0.0");

    // ...and re-providing it is still a no-op.
    let (_, again) =
        provide_and_read_version(&app, "/provide", issue6_openapi_payload(empty)).await;
    assert_eq!(again, "1.0.0", "re-providing an empty spec does not bump");
}

// --- Protected-branch rejections are audited (issue #8) ------------------------
//
// A refusal on a Protected branch is the most consequential thing the registry
// does, and it used to leave only a transient log line. These drive a real
// rejection through the API and then read it back from the audit timeline as an
// administrator, which is the only way to see it.

#[cfg(test)]
const ISSUE8_V1: &str = r#"
openapi: 3.0.0
info:
  title: Billing
  version: 1.0.0
paths:
  /invoices:
    get:
      responses:
        '200':
          description: OK
        '404':
          description: Not found
"#;

/// Removes the `404` response, which is a breaking change.
#[cfg(test)]
const ISSUE8_V2_BREAKING: &str = r#"
openapi: 3.0.0
info:
  title: Billing
  version: 1.0.0
paths:
  /invoices:
    get:
      responses:
        '200':
          description: OK
"#;

#[cfg(test)]
async fn provide_as(app: &axum::Router, token: &str, payload: Value) -> StatusCode {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

/// Read the audit timeline as an administrator. `query` is appended verbatim.
#[cfg(test)]
async fn timeline_as_admin(app: &axum::Router, token: &str, query: &str) -> Vec<Value> {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/audit/timeline?limit=100{query}"))
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "admin may read the timeline");
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice::<Value>(&body)
        .unwrap()
        .as_array()
        .expect("the timeline is an array")
        .clone()
}

#[tokio::test]
async fn a_rejected_provide_on_a_protected_branch_is_audited() {
    let (app, token, _repo) = setup_app_with_admin().await;

    let seed = json!({ "producername": "billing", "branch": "main", "openapi_yaml": ISSUE8_V1 });
    assert_eq!(provide_as(&app, &token, seed).await, StatusCode::ACCEPTED);

    let breaking =
        json!({ "producername": "billing", "branch": "main", "openapi_yaml": ISSUE8_V2_BREAKING });
    assert_eq!(
        provide_as(&app, &token, breaking).await,
        StatusCode::CONFLICT,
        "the caller's response is unchanged by this feature"
    );

    let rejections: Vec<Value> = timeline_as_admin(&app, &token, "")
        .await
        .into_iter()
        .filter(|e| e["action"] == "QUARANTINED_SPEC")
        .collect();

    assert_eq!(rejections.len(), 1, "exactly one held submission recorded");
    let entry = &rejections[0];
    assert_eq!(entry["service"], "billing");
    assert_eq!(entry["branch"], "main");
    assert_eq!(entry["username"], "admin", "the real Actor is recorded");
    let details = entry["details"].as_str().unwrap_or_default();
    assert!(
        details.contains("404") || details.to_lowercase().contains("breaking"),
        "the reason must be stored, got: {details}"
    );
}

#[tokio::test]
async fn rejections_are_filterable_by_action_type_and_producer() {
    let (app, token, _repo) = setup_app_with_admin().await;

    let seed = json!({ "producername": "billing", "branch": "main", "openapi_yaml": ISSUE8_V1 });
    provide_as(&app, &token, seed).await;
    let breaking =
        json!({ "producername": "billing", "branch": "main", "openapi_yaml": ISSUE8_V2_BREAKING });
    provide_as(&app, &token, breaking).await;

    let by_type = timeline_as_admin(&app, &token, "&action_type=REJECT").await;
    assert!(
        !by_type.is_empty() && by_type.iter().all(|e| e["action"] == "QUARANTINED_SPEC"),
        "filtering by the rejection action type returns only held Provides, got: {by_type:?}"
    );

    let by_producer = timeline_as_admin(&app, &token, "&action_type=REJECT&service=billing").await;
    assert_eq!(by_producer.len(), 1, "filterable by Producer as well");
}

#[tokio::test]
async fn a_successful_provide_records_no_rejection() {
    let (app, token, _repo) = setup_app_with_admin().await;

    let seed = json!({ "producername": "billing", "branch": "main", "openapi_yaml": ISSUE8_V1 });
    assert_eq!(provide_as(&app, &token, seed).await, StatusCode::ACCEPTED);

    let entries = timeline_as_admin(&app, &token, "").await;
    assert!(
        entries.iter().all(|e| e["action"] != "QUARANTINED_SPEC"),
        "a successful Provide must not look like a rejection"
    );
}

#[tokio::test]
async fn a_breaking_change_on_a_feature_branch_records_no_rejection() {
    // Feature branches accept breaking changes, so nothing was refused and
    // nothing should be recorded.
    let (app, token, _repo) = setup_app_with_admin().await;

    let seed = json!({ "producername": "billing", "branch": "feat-x", "openapi_yaml": ISSUE8_V1 });
    assert_eq!(provide_as(&app, &token, seed).await, StatusCode::ACCEPTED);

    let breaking = json!({ "producername": "billing", "branch": "feat-x", "openapi_yaml": ISSUE8_V2_BREAKING });
    assert_eq!(
        provide_as(&app, &token, breaking).await,
        StatusCode::ACCEPTED
    );

    let entries = timeline_as_admin(&app, &token, "").await;
    assert!(
        entries.iter().all(|e| e["action"] != "QUARANTINED_SPEC"),
        "an accepted change on a feature branch is not a rejection"
    );
}

#[tokio::test]
async fn a_dry_run_rejection_is_not_audited() {
    // A dry run asks "would this be refused?". Recording it would fill the
    // timeline with refusals that never happened.
    let (app, token, _repo) = setup_app_with_admin().await;

    let seed = json!({ "producername": "billing", "branch": "main", "openapi_yaml": ISSUE8_V1 });
    provide_as(&app, &token, seed).await;

    let dry = json!({
        "producername": "billing",
        "branch": "main",
        "openapi_yaml": ISSUE8_V2_BREAKING,
        "dry_run": true
    });
    provide_as(&app, &token, dry).await;

    let entries = timeline_as_admin(&app, &token, "").await;
    assert!(
        entries.iter().all(|e| e["action"] != "QUARANTINED_SPEC"),
        "a dry run must not record a rejection"
    );

    // Positive control: the same content, provided for real, IS recorded — so
    // the assertion above is about the dry run and not about rejection
    // recording being broken or absent altogether.
    let real =
        json!({ "producername": "billing", "branch": "main", "openapi_yaml": ISSUE8_V2_BREAKING });
    assert_eq!(provide_as(&app, &token, real).await, StatusCode::CONFLICT);

    let after = timeline_as_admin(&app, &token, "").await;
    assert_eq!(
        after
            .iter()
            .filter(|e| e["action"] == "QUARANTINED_SPEC")
            .count(),
        1,
        "the real refusal is recorded, so the dry run genuinely added nothing"
    );
}

/// Two AsyncAPI channels; removing one from a Protected branch is refused by the
/// endpoint-removal path, which is the site that used to be entirely silent.
#[cfg(test)]
const ISSUE8_ASYNC_TWO: &str = r#"
asyncapi: '2.6.0'
info:
  title: Billing Events
  version: '1.0'
channels:
  invoice/created:
    publish:
      message:
        name: InvoiceCreated
  invoice/paid:
    publish:
      message:
        name: InvoicePaid
"#;

#[cfg(test)]
const ISSUE8_ASYNC_ONE: &str = r#"
asyncapi: '2.6.0'
info:
  title: Billing Events
  version: '1.0'
channels:
  invoice/created:
    publish:
      message:
        name: InvoiceCreated
"#;

#[cfg(test)]
async fn provide_asyncapi_as(app: &axum::Router, token: &str, payload: Value) -> StatusCode {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide/asyncapi")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn refusing_to_remove_a_non_deprecated_endpoint_is_audited() {
    // The third rejection site. It is unreachable for OpenAPI — the whole-spec
    // compatibility check refuses a removed path first — so it is exercised
    // through AsyncAPI, and it is the path that previously produced no record
    // and no log line at all.
    let (app, token, _repo) = setup_app_with_admin().await;

    let seed = json!({
        "producername": "billing-events",
        "branch": "main",
        "asyncapi_yaml": ISSUE8_ASYNC_TWO
    });
    assert_eq!(
        provide_asyncapi_as(&app, &token, seed).await,
        StatusCode::ACCEPTED
    );

    let removal = json!({
        "producername": "billing-events",
        "branch": "main",
        "asyncapi_yaml": ISSUE8_ASYNC_ONE
    });
    assert_eq!(
        provide_asyncapi_as(&app, &token, removal).await,
        StatusCode::CONFLICT,
        "removing a non-deprecated endpoint from a Protected branch is refused"
    );

    let rejections: Vec<Value> = timeline_as_admin(&app, &token, "&action_type=REJECT")
        .await
        .into_iter()
        .filter(|e| e["action"] == "QUARANTINED_SPEC")
        .collect();

    assert_eq!(rejections.len(), 1, "the removal refusal is recorded");
    let entry = &rejections[0];
    assert_eq!(entry["service"], "billing-events");
    assert_eq!(entry["branch"], "main");
    let details = entry["details"].as_str().unwrap_or_default();
    assert!(
        details.contains("deprecated"),
        "the reason must name the deprecation requirement, got: {details}"
    );
}
