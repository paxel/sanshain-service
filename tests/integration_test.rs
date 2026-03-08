use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;
use tower::ServiceExt; // for `oneshot`
use sanshain_service::{create_app, AppState};

#[tokio::test]
async fn test_full_flow() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .unwrap();

    let state = AppState { db: pool };
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
}
