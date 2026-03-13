use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;
use std::sync::Arc;
use std::collections::HashSet;
use tokio::sync::RwLock;
use tower::ServiceExt; // for `oneshot`
use sanshain_service::{create_app, AppState};
use sanshain_service::infrastructure::sqlite_repository::SqliteSpecRepository;
use sanshain_service::application::services;

const TEST_CSRF_TOKEN: &str = "test-csrf-token";

async fn setup_app() -> (axum::Router, SqliteSpecRepository) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();

    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    let mut tokens = HashSet::new();
    tokens.insert(TEST_CSRF_TOKEN.to_string());
    let state = AppState { repo: repo.clone(), csrf_tokens: Arc::new(RwLock::new(tokens)) };
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

    let mut tokens = HashSet::new();
    tokens.insert(TEST_CSRF_TOKEN.to_string());
    let state = AppState { repo, csrf_tokens: Arc::new(RwLock::new(tokens)) };
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
    assert!(md_report.contains("| client-a | test-service | `/users` | `GET` |"));
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

    // 3. Third provide (changed DTO on same path) -> should be CONFLICT
    let openapi_v1_modified = r#"
openapi: 3.0.0
info:
  title: Test API
  version: 1.0.0
paths:
  /users:
    get:
      description: Modified DTO
      responses:
        '200':
          description: OK
"#;
    let payload_modified = json!({
        "servicename": "test-service",
        "branch": "main",
        "openapi_yaml": openapi_v1_modified
    });

    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&payload_modified).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);

    // 4. Fourth provide (new path) -> should be ACCEPTED
    let openapi_v2 = r#"
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
  /v2/users:
    get:
      responses:
        '200':
          description: OK
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
    let svcs: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert!(svcs.contains(&"svc1".to_string()));

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
    let svcs: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert!(!svcs.contains(&"svc1".to_string()));
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
                .uri(&format!("/admin/users/{}/approve", new_user_id))
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
                .uri(&format!("/admin/users/{}", new_user_id))
                .header("Authorization", format!("Bearer {}", token))
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
