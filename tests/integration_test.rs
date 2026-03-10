use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;
use tower::ServiceExt; // for `oneshot`
use sanshain_service::{create_app, AppState};
use sanshain_service::infrastructure::sqlite_repository::SqliteSpecRepository;

#[tokio::test]
async fn test_full_flow() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();

    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    let state = AppState { repo };
    let app = create_app(state);

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
    assert!(md_report.contains("# SanShain Dependency Report: Branch `main`"));
    assert!(md_report.contains("| client-a | test-service | `/users` | `GET` |"));
}

#[tokio::test]
async fn test_idempotency_and_conflict() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();

    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    let state = AppState { repo };
    let app = create_app(state);

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
                .body(Body::from(serde_json::to_vec(&payload_v2).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn test_protected_branches_api() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();

    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    let state = AppState { repo };
    let app = create_app(state);

    // 1. List default protected branches
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
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_feature_branch_allows_dto_update() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();

    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    let state = AppState { repo };
    let app = create_app(state);

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
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();

    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    let state = AppState { repo };
    let app = create_app(state);

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
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000).await.unwrap();
    let services: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert!(services.contains(&"svc1".to_string()));

    // List branches
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/services/svc1/branches")
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
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000).await.unwrap();
    let services: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert!(!services.contains(&"svc1".to_string()));
}
