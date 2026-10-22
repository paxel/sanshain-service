//! Who signs in under each auth mode, through the real router: a fresh
//! instance starts in Off, Off admits only holders of `manage_auth_config`
//! (their sessions too) and closes the API, and the login page learns what
//! to offer from `/auth/options`.

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

/// A fresh instance with its bootstrap admin `root` / `root_password`.
#[cfg(test)]
async fn fresh_instance() -> (axum::Router, SqliteSpecRepository) {
    let repo = fresh_repo().await;
    unsafe {
        std::env::set_var("INITIAL_ADMIN_USERNAME", "root");
        std::env::set_var("INITIAL_ADMIN_PASSWORD", "root_password");
        std::env::remove_var("INITIAL_AUTH_MODE");
    }
    services::ensure_initial_admin(&repo, "http://127.0.0.1:3000/admin.html")
        .await
        .unwrap();
    (create_app(test_state(repo.clone())), repo)
}

#[tokio::test]
async fn a_fresh_instance_starts_in_off_and_only_the_admin_gets_in() {
    let (app, repo) = fresh_instance().await;
    assert_eq!(
        services::get_auth_mode(&repo).await.unwrap(),
        AuthMode::Disabled
    );

    let (status, options) = call(&app, "GET", "/auth/options", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(options["auth_mode"], "disabled");
    assert_eq!(options["admins_only"], true);
    assert_eq!(options["registration"], false);
    assert_eq!(options["sso"], false);

    let (status, body) = login(&app, "root", "root_password").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token = body["token"].as_str().unwrap().to_string();
    let (status, _) = call(&app, "GET", "/auth/me", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);

    // The API is closed while in maintenance, for the admin too.
    let (status, _) = call(
        &app,
        "POST",
        "/provide",
        Some(&token),
        Some(serde_json::json!({ "producername": "p", "stability": "snapshot", "openapi_yaml": "openapi: 3.0.0" })),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    // Nobody can register.
    let (status, _) = call(
        &app,
        "POST",
        "/auth/register",
        None,
        Some(serde_json::json!({ "username": "eve", "password": "secret-pass" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn off_refuses_everyone_else_and_ends_their_open_sessions() {
    let (app, repo) = fresh_instance().await;
    services::set_auth_mode(&repo, &AuthMode::Local)
        .await
        .unwrap();
    plain_user(&repo, "bob", "bob-pass").await;
    let (status, body) = login(&app, "bob", "bob-pass").await;
    assert_eq!(status, StatusCode::OK);
    let bob_token = body["token"].as_str().unwrap().to_string();
    let (_, body) = login(&app, "root", "root_password").await;
    let root_token = body["token"].as_str().unwrap().to_string();

    let (status, _) = call(
        &app,
        "PUT",
        "/admin/auth-config",
        Some(&root_token),
        Some(serde_json::json!({ "auth_mode": "off" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = login(&app, "bob", "bob-pass").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        body["error"].as_str().unwrap().contains("maintenance"),
        "{body}"
    );
    let (status, _) = call(&app, "GET", "/auth/me", Some(&bob_token), None).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the open session stops working"
    );
    let (status, _) = call(&app, "GET", "/admin/producers", Some(&bob_token), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = call(&app, "GET", "/auth/me", Some(&root_token), None).await;
    assert_eq!(status, StatusCode::OK);

    // Off remembers Local; switching back lets bob in again.
    let (status, _) = call(
        &app,
        "PUT",
        "/admin/auth-config",
        Some(&root_token),
        Some(serde_json::json!({ "auth_mode": "local" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&app, "GET", "/auth/me", Some(&bob_token), None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn registration_follows_its_switch_and_says_whether_to_wait() {
    let (app, _repo) = fresh_instance().await;
    let (_, body) = login(&app, "root", "root_password").await;
    let root = body["token"].as_str().unwrap().to_string();
    for (uri, json) in [(
        "/admin/auth-config",
        serde_json::json!({ "auth_mode": "local" }),
    )] {
        let (status, _) = call(&app, "PUT", uri, Some(&root), Some(json)).await;
        assert_eq!(status, StatusCode::OK);
    }
    let register = |name: &'static str| {
        let app = app.clone();
        async move {
            call(
                &app,
                "POST",
                "/auth/register",
                None,
                Some(serde_json::json!({ "username": name, "password": "secret-pass" })),
            )
            .await
        }
    };
    let (status, _) = register("carol").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "registration starts closed");

    let (status, _) = call(
        &app,
        "POST",
        "/admin/settings/registration",
        Some(&root),
        Some(serde_json::json!({ "enabled": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, options) = call(&app, "GET", "/auth/options", None, None).await;
    assert_eq!(options["registration"], true);
    assert_eq!(options["auto_approve"], false);
    let (status, body) = register("carol").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["approved"], false);

    let (status, _) = call(
        &app,
        "POST",
        "/admin/settings/auto-approve",
        Some(&root),
        Some(serde_json::json!({ "enabled": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = register("dave").await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["approved"], true);
    let (status, _) = login(&app, "dave", "secret-pass").await;
    assert_eq!(status, StatusCode::OK);

    // Closing registration keeps the accounts it created.
    let (status, _) = call(
        &app,
        "POST",
        "/admin/settings/registration",
        Some(&root),
        Some(serde_json::json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, options) = call(&app, "GET", "/auth/options", None, None).await;
    assert_eq!(options["registration"], false);
    let (status, _) = login(&app, "dave", "secret-pass").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn dev_mode_is_refused_without_its_gate() {
    let (app, repo) = fresh_instance().await;
    let (_, body) = login(&app, "root", "root_password").await;
    let root = body["token"].as_str().unwrap().to_string();
    let (status, body) = call(
        &app,
        "PUT",
        "/admin/auth-config",
        Some(&root),
        Some(serde_json::json!({ "auth_mode": "dev" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("ALLOW_INSECURE_DEV_MODE")
    );
    assert_eq!(
        services::get_auth_mode(&repo).await.unwrap(),
        AuthMode::Disabled
    );
}

#[tokio::test]
async fn an_admin_runs_a_service_account_end_to_end() {
    let (app, repo) = fresh_instance().await;
    services::set_auth_mode(&repo, &AuthMode::Local)
        .await
        .unwrap();
    plain_user(&repo, "bob", "bob-pass").await;
    let (_, body) = login(&app, "root", "root_password").await;
    let root = body["token"].as_str().unwrap().to_string();
    let (_, body) = login(&app, "bob", "bob-pass").await;
    let bob = body["token"].as_str().unwrap().to_string();

    let (status, body) = call(
        &app,
        "POST",
        "/admin/users",
        Some(&root),
        Some(serde_json::json!({ "username": "jenkins" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let (_, users) = call(&app, "GET", "/admin/users", Some(&root), None).await;
    let jenkins = users
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "jenkins")
        .unwrap();
    assert_eq!(jenkins["service_account"], true);
    assert_eq!(jenkins["approved"], true);

    // It never signs in.
    let (status, _) = login(&app, "jenkins", "!service-account").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Only a user manager issues its tokens.
    let issue = serde_json::json!({ "name": "ci", "expires_in_days": 30, "for_user": "jenkins" });
    let (status, _) = call(
        &app,
        "POST",
        "/auth/tokens",
        Some(&bob),
        Some(issue.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = call(&app, "POST", "/auth/tokens", Some(&root), Some(issue)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let raw = body["token"].as_str().unwrap().to_string();
    let token_id = body["id"].as_str().unwrap().to_string();

    // The token works as jenkins on the API.
    let spec = "openapi: 3.0.0\ninfo:\n  title: p\n  version: 1.0.0\npaths:\n  /a:\n    get:\n      responses:\n        '200': { description: ok }\n";
    let provide = serde_json::json!({ "producername": "jenkins-built", "stability": "snapshot", "openapi_yaml": spec });
    let (status, body) = call(&app, "POST", "/provide", Some(&raw), Some(provide.clone())).await;
    assert!(status.is_success(), "{status} {body}");

    // The admin's token page lists it, for jenkins, issued by root.
    let (_, tokens) = call(&app, "GET", "/auth/tokens", Some(&root), None).await;
    let listed = tokens
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == token_id.as_str())
        .unwrap();
    assert_eq!(listed["for_user"], "jenkins");
    assert_eq!(listed["issued_by"], "root");
    let (_, tokens) = call(&app, "GET", "/auth/tokens", Some(&bob), None).await;
    assert!(tokens.as_array().unwrap().is_empty(), "not bob's to see");

    let uri = format!("/auth/tokens/{token_id}");
    let (status, _) = call(&app, "DELETE", &uri, Some(&bob), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(&app, "DELETE", &uri, Some(&root), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(&app, "POST", "/provide", Some(&raw), Some(provide)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "revoked");
}
