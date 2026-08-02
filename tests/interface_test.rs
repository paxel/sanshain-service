use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::Utc;
use sanshain_service::application::services;
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
use tower::ServiceExt;

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

// `cfg(test)` is always true in this crate; the attribute marks the helper as
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

#[tokio::test]
async fn test_admin_settings_interface_consistency() {
    unsafe {
        std::env::set_var("INITIAL_ADMIN_USERNAME", "root");
        std::env::set_var("INITIAL_ADMIN_PASSWORD", "root_password");
    }
    let (app, repo) = setup_app().await;
    services::ensure_initial_admin(&repo).await.unwrap();

    // Login to get token
    let login_req = Request::builder()
        .method("POST")
        .uri("/auth/login")
        .header("Content-Type", "application/json")
        .body(Body::from(
            json!({
                "username": "root",
                "password": "root_password"
            })
            .to_string(),
        ))
        .unwrap();

    let login_res = app.clone().oneshot(login_req).await.unwrap();
    assert_eq!(login_res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(login_res.into_body(), usize::MAX)
        .await
        .unwrap();
    let login_data: Value = serde_json::from_slice(&body).unwrap();
    let token = login_data["token"].as_str().unwrap();

    let check_setting = |uri: &str, key: &str| {
        let app = app.clone();
        let token = token.to_string();
        let uri = uri.to_string();
        let key = key.to_string();
        async move {
            let req = Request::builder()
                .method("GET")
                .uri(&uri)
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap();
            let res = app.oneshot(req).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK, "Failed GET {}", uri);
            let body = axum::body::to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap();
            let data: Value = serde_json::from_slice(&body).unwrap();
            assert!(data.is_object(), "Response from {} is not an object", uri);
            assert!(
                data.get(&key).is_some(),
                "Key {} missing in response from {}",
                key,
                uri
            );
        }
    };

    check_setting("/admin/settings/dev-mode", "dev_mode").await;
    check_setting("/admin/settings/auto-approve", "auto_approve_users").await;
    check_setting("/admin/settings/snapshot-max-age", "days").await;
    check_setting("/admin/settings/dependency-max-age", "days").await;
}

#[tokio::test]
async fn test_admin_list_interface_consistency() {
    unsafe {
        std::env::set_var("INITIAL_ADMIN_USERNAME", "root");
        std::env::set_var("INITIAL_ADMIN_PASSWORD", "root_password");
    }
    let (app, repo) = setup_app().await;
    services::ensure_initial_admin(&repo).await.unwrap();

    let login_req = Request::builder()
        .method("POST")
        .uri("/auth/login")
        .header("Content-Type", "application/json")
        .body(Body::from(
            json!({
                "username": "root",
                "password": "root_password"
            })
            .to_string(),
        ))
        .unwrap();

    let login_res = app.clone().oneshot(login_req).await.unwrap();
    let body = axum::body::to_bytes(login_res.into_body(), usize::MAX)
        .await
        .unwrap();
    let login_data: Value = serde_json::from_slice(&body).unwrap();
    let token = login_data["token"].as_str().unwrap();

    let check_list = |uri: &str| {
        let app = app.clone();
        let token = token.to_string();
        let uri = uri.to_string();
        async move {
            let req = Request::builder()
                .method("GET")
                .uri(&uri)
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap();
            let res = app.oneshot(req).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK, "Failed GET {}", uri);
            let body = axum::body::to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap();
            let data: Value = serde_json::from_slice(&body).unwrap();
            assert!(data.is_array(), "Response from {} is not an array", uri);
        }
    };

    check_list("/admin/users").await;
    check_list("/admin/producers").await;
    check_list("/admin/consumers").await;
}
