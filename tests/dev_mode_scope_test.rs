//! Dev mode with its gate open: the API takes requests without a token,
//! while the administration still needs a signed-in administrator, and only
//! administrators sign in. Its own test binary, because the gate is a
//! process-wide environment variable.

#![expect(
    unsafe_code,
    reason = "edition 2024: std::env::set_var/remove_var are unsafe"
)]

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::Utc;
use sanshain_service::application::services;
use sanshain_service::domain::models::AuthMode;
use sanshain_service::domain::ports::SpecRepository;
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
        dev_user: None,
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
        max_body_bytes: sanshain_service::DEFAULT_MAX_BODY_BYTES,
        snapshot_in_progress: Arc::new(AtomicBool::new(false)),
        mcp: Default::default(),
        directory_roles: sanshain_service::application::directory_roles::DirectoryRoleCache::new(
            std::time::Duration::from_secs(300),
        ),
        root_users: std::sync::Arc::new(sanshain_service::domain::permissions::RootUsers::resolve(
            Some("root"),
            None,
        )),
    }
}

#[cfg(test)]
async fn fresh_repo() -> SqliteSpecRepository {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();
    repo
}

#[cfg(test)]
async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header("Authorization", format!("Bearer {token}"));
    }
    let request = match body {
        Some(json) => builder
            .header("Content-Type", "application/json")
            .body(Body::from(json.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let res = app.clone().oneshot(request).await.unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

#[cfg(test)]
async fn login(
    app: &axum::Router,
    username: &str,
    password: &str,
) -> (StatusCode, serde_json::Value) {
    call(
        app,
        "POST",
        "/auth/login",
        None,
        Some(serde_json::json!({ "username": username, "password": password })),
    )
    .await
}

/// An approved account with a local password and no roles.
#[cfg(test)]
async fn plain_user(repo: &SqliteSpecRepository, username: &str, password: &str) {
    let hash = services::hash_password(password).unwrap();
    repo.create_user(username, &hash, true).await.unwrap();
}

#[tokio::test]
async fn dev_mode_opens_the_api_but_not_the_administration() {
    unsafe {
        std::env::set_var("ALLOW_INSECURE_DEV_MODE", "true");
        std::env::set_var("INITIAL_ADMIN_USERNAME", "root");
        std::env::set_var("INITIAL_ADMIN_PASSWORD", "root_password");
        std::env::set_var("INITIAL_AUTH_MODE", "dev");
    }
    let repo = fresh_repo().await;
    services::ensure_initial_admin(&repo, "http://127.0.0.1:3000/admin.html")
        .await
        .unwrap();
    assert_eq!(services::get_auth_mode(&repo).await.unwrap(), AuthMode::Dev);
    plain_user(&repo, "bob", "bob-pass").await;
    let app = create_app(test_state(repo.clone()));

    let spec = "openapi: 3.0.0\ninfo:\n  title: p\n  version: 1.0.0\npaths:\n  /a:\n    get:\n      responses:\n        '200': { description: ok }\n";
    let (status, body) = call(
        &app,
        "POST",
        "/provide",
        None,
        Some(serde_json::json!({ "producername": "p", "stability": "snapshot", "openapi_yaml": spec })),
    )
    .await;
    assert!(
        status.is_success(),
        "a tokenless provide works in dev mode: {status} {body}"
    );

    let (status, _) = call(&app, "GET", "/admin/users", None, None).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "no dev user on the administration"
    );
    let (status, _) = call(&app, "GET", "/auth/me", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, body) = login(&app, "bob", "bob-pass").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        body["error"].as_str().unwrap().contains("dev mode"),
        "{body}"
    );

    let (status, body) = login(&app, "root", "root_password").await;
    assert_eq!(status, StatusCode::OK);
    let root = body["token"].as_str().unwrap().to_string();
    let (status, _) = call(&app, "GET", "/admin/users", Some(&root), None).await;
    assert_eq!(status, StatusCode::OK);
}
