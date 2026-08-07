//! Router-seam coverage for the admin settings, auth, and API report/require
//! handlers that the behavioral suites do not reach: auth-config get/put/test,
//! observability endpoints, cache/database settings, dev-mode and auto-approve
//! toggles, the nuke endpoints, user administration, producer metadata, and
//! the report/bundle/timeline API surface.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::Utc;
use sanshain_service::application::services;
use sanshain_service::domain::models::{ApiType, AuthMode, Stability};
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
        directory_roles: sanshain_service::application::directory_roles::DirectoryRoleCache::new(
            std::time::Duration::from_secs(300),
        ),
        root_users: std::sync::Arc::new(sanshain_service::domain::permissions::RootUsers::resolve(
            Some("root"),
            None,
        )),
    }
}

const DEMO_SPEC_1_0_0: &str = r#"openapi: 3.0.0
info:
  title: demo
  version: 1.0.0
paths:
  /hello:
    get:
      responses:
        '200': { description: ok }
"#;

// `cfg(test)` is always true in this crate; the attribute marks the helpers as
// test code for clippy's `allow-unwrap-in-tests`.
#[cfg(test)]
async fn app_with_seed() -> (axum::Router, SqliteSpecRepository, String) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    unsafe {
        std::env::set_var("INITIAL_ADMIN_USERNAME", "root");
        std::env::set_var("INITIAL_ADMIN_PASSWORD", "root_password");
    }
    services::ensure_initial_admin(&repo).await.unwrap();
    let (session, _user) = services::login(&repo, "root", "root_password")
        .await
        .unwrap();

    services::provide_spec(
        &repo,
        services::ProvideSpecParams {
            producername: "demo-svc",
            api_type: ApiType::OpenApi,
            content: DEMO_SPEC_1_0_0,
            stability: Stability::Ga,
            dry_run: false,
            trunk: false,
            tag: None,
            caller: Some(sanshain_service::domain::permissions::Actor::test_releaser()),
            require_prior_content_match: false,
        },
    )
    .await
    .unwrap();

    let app = create_app(test_state(repo.clone()));
    (app, repo, session.token)
}

#[cfg(test)]
async fn get_json(app: &axum::Router, uri: &str, auth: &str) -> (StatusCode, Value) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header(axum::http::header::AUTHORIZATION, auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&body).unwrap_or(Value::Null);
    (status, json)
}

/// Send a JSON body with the given method; answers status and parsed body.
#[cfg(test)]
async fn send_json(
    app: &axum::Router,
    method: &str,
    uri: &str,
    auth: &str,
    body: &Value,
) -> (StatusCode, Value) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header(axum::http::header::AUTHORIZATION, auth)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

#[cfg(test)]
async fn send_empty(
    app: &axum::Router,
    method: &str,
    uri: &str,
    auth: &str,
) -> (StatusCode, Value) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header(axum::http::header::AUTHORIZATION, auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

// --- Admin settings -------------------------------------------------------

#[tokio::test]
async fn auto_approve_settings_roundtrip() {
    let (app, _repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    let (status, body) = get_json(&app, "/admin/settings/auto-approve", &auth).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "auto_approve_users": false }));

    let (status, _) = send_json(
        &app,
        "POST",
        "/admin/settings/auto-approve",
        &auth,
        &json!({ "enabled": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = get_json(&app, "/admin/settings/auto-approve", &auth).await;
    assert_eq!(body, json!({ "auto_approve_users": true }));
}

#[tokio::test]
async fn auth_config_get_put_and_password_masking() {
    let (app, repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    // Fresh instance: the bootstrap sets local mode; no LDAP config stored.
    let (status, body) = get_json(&app, "/admin/auth-config", &auth).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["auth_mode"], "local");
    assert_eq!(body["ldap_config"], Value::Null);

    // Unknown mode is a named 400.
    let (status, body) = send_json(
        &app,
        "PUT",
        "/admin/auth-config",
        &auth,
        &json!({ "auth_mode": "bogus" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "Invalid auth mode: bogus");

    // ldap mode without a config is refused.
    let (status, body) = send_json(
        &app,
        "PUT",
        "/admin/auth-config",
        &auth,
        &json!({ "auth_mode": "ldap" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "LDAP config required for ldap mode");

    // Store an LDAP config with a real bind password.
    let ldap = json!({
        "server_url": "ldap://127.0.0.1:9",
        "bind_dn": "cn=admin,dc=example,dc=com",
        "bind_password": "secret-bind-pw",
        "base_dn": "dc=example,dc=com",
    });
    let (status, _) = send_json(
        &app,
        "PUT",
        "/admin/auth-config",
        &auth,
        &json!({ "auth_mode": "ldap", "ldap_config": ldap }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Reading it back masks the password.
    let (status, body) = get_json(&app, "/admin/auth-config", &auth).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["auth_mode"], "ldap");
    assert_eq!(body["ldap_config"]["bind_password"], "****");
    assert_eq!(body["ldap_config"]["server_url"], "ldap://127.0.0.1:9");

    // Writing the masked placeholder back keeps the stored secret.
    let masked = json!({
        "server_url": "ldap://127.0.0.1:9",
        "bind_dn": "cn=admin,dc=example,dc=com",
        "bind_password": "****",
        "base_dn": "dc=example,dc=com",
    });
    let (status, _) = send_json(
        &app,
        "PUT",
        "/admin/auth-config",
        &auth,
        &json!({ "auth_mode": "ldap", "ldap_config": masked }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let stored = services::get_ldap_config(&repo)
        .await
        .unwrap()
        .expect("ldap config stored");
    assert_eq!(
        stored.bind_password.as_deref(),
        Some("secret-bind-pw"),
        "the masked placeholder must not overwrite the stored secret"
    );

    // Switching back to local mode works without a config.
    let (status, _) = send_json(
        &app,
        "PUT",
        "/admin/auth-config",
        &auth,
        &json!({ "auth_mode": "local" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = get_json(&app, "/admin/auth-config", &auth).await;
    assert_eq!(body["auth_mode"], "local");
}

#[tokio::test]
async fn auth_config_test_reports_unreachable_server() {
    let (app, _repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    // Port 9 (discard) is not an LDAP server; the connection test must fail
    // as a client error, not hang or 500.
    let (status, body) = send_json(
        &app,
        "POST",
        "/admin/auth-config/test",
        &auth,
        &json!({
            "server_url": "ldap://127.0.0.1:9",
            "bind_dn": "cn=admin,dc=example,dc=com",
            "bind_password": "pw",
            "base_dn": "dc=example,dc=com",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let error = body["error"].as_str().unwrap();
    assert!(
        error.starts_with("LDAP connection failed"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn snapshot_and_dependency_age_settings_and_cleanups() {
    let (app, _repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    // Defaults are 30 days each.
    let (status, body) = get_json(&app, "/admin/settings/snapshot-max-age", &auth).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "days": 30 }));
    let (status, body) = get_json(&app, "/admin/settings/dependency-max-age", &auth).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "days": 30 }));

    let (status, _) = send_json(
        &app,
        "POST",
        "/admin/settings/snapshot-max-age",
        &auth,
        &json!({ "days": 14 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = get_json(&app, "/admin/settings/snapshot-max-age", &auth).await;
    assert_eq!(body, json!({ "days": 14 }));

    // Zero dependency max-age is refused; a positive value sticks.
    let (status, body) = send_json(
        &app,
        "POST",
        "/admin/settings/dependency-max-age",
        &auth,
        &json!({ "days": 0 }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "days must be greater than 0");
    let (status, _) = send_json(
        &app,
        "POST",
        "/admin/settings/dependency-max-age",
        &auth,
        &json!({ "days": 45 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = get_json(&app, "/admin/settings/dependency-max-age", &auth).await;
    assert_eq!(body, json!({ "days": 45 }));

    // Nothing is old enough to cull in a fresh database.
    let (status, body) = send_empty(&app, "POST", "/admin/cleanup/snapshots", &auth).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "deleted": 0 }));
    let (status, body) = send_empty(&app, "POST", "/admin/cleanup/dependencies", &auth).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "deleted": 0 }));
}

#[tokio::test]
async fn cache_config_get_set_and_database_info() {
    let (app, _repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    let (status, body) = get_json(&app, "/admin/settings/cache", &auth).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["enabled"], true);
    assert_eq!(body["memory_limit_mb"], 64);

    let (status, body) = send_json(
        &app,
        "POST",
        "/admin/settings/cache",
        &auth,
        &json!({ "memory_mb": 16 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "memory_mb": 16 }));
    let (_, body) = get_json(&app, "/admin/settings/cache", &auth).await;
    assert_eq!(body["memory_limit_mb"], 16);

    let (status, body) = get_json(&app, "/admin/settings/database", &auth).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({ "backend": "sqlite", "url": "sqlite::memory:" })
    );
}

#[tokio::test]
async fn observability_stats_logs_and_debug_config() {
    let (app, _repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    let (status, stats) = get_json(&app, "/admin/observability/stats", &auth).await;
    assert_eq!(status, StatusCode::OK);
    assert!(stats["memory_total"].as_u64().unwrap() > 0);
    assert_eq!(stats["requests_total"], 0);
    assert_eq!(stats["failures_total"], 0);

    let (status, logs) = get_json(&app, "/admin/observability/logs", &auth).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        logs,
        json!({ "errors": [], "warnings": [], "infos": [], "debugs": [] })
    );

    let (status, config) = get_json(&app, "/admin/observability/debug-config", &auth).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        config,
        json!({ "business_logic_debug": false, "admin_user_debug": false })
    );

    let (status, _) = send_json(
        &app,
        "POST",
        "/admin/observability/debug-config-update",
        &auth,
        &json!({ "business_logic_debug": true, "admin_user_debug": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, config) = get_json(&app, "/admin/observability/debug-config", &auth).await;
    assert_eq!(
        config,
        json!({ "business_logic_debug": true, "admin_user_debug": true })
    );
}

#[tokio::test]
async fn audit_log_listing_and_csv_export_carry_the_recorded_action() {
    let (app, _repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    // Produce one audited admin action to observe.
    let (status, _) = send_empty(&app, "POST", "/admin/cache/clear", &auth).await;
    assert_eq!(status, StatusCode::OK);

    let (status, logs) = get_json(&app, "/admin/observability/audit-logs", &auth).await;
    assert_eq!(status, StatusCode::OK);
    let entry = logs
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["action"] == "CLEAR_CACHE")
        .expect("cache clear is audited");
    assert_eq!(entry["username"], "root");
    assert_eq!(entry["details"], "Cleared spec caches");

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/observability/audit-logs/export")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get(axum::http::header::CONTENT_TYPE).unwrap(),
        "text/csv"
    );
    assert_eq!(
        res.headers()
            .get(axum::http::header::CONTENT_DISPOSITION)
            .unwrap(),
        "attachment; filename=\"audit_logs.csv\""
    );
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let csv = std::str::from_utf8(&body).unwrap();
    let mut lines = csv.lines();
    assert_eq!(
        lines.next().unwrap(),
        "id,timestamp,username,action,details,service,version,action_type"
    );
    assert!(
        csv.contains("\"CLEAR_CACHE\""),
        "export must contain the audited action: {csv}"
    );
}

// --- User administration --------------------------------------------------

#[tokio::test]
async fn user_registration_approval_login_and_deletion_lifecycle() {
    let (app, repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    // Registration is only open in local auth mode.
    services::set_auth_mode(&repo, &AuthMode::Disabled)
        .await
        .unwrap();
    let creds = json!({ "username": "bob", "password": "hunter2secret" });
    let (status, _) = send_json(&app, "POST", "/auth/register", &auth, &creds).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    services::set_auth_mode(&repo, &AuthMode::Local)
        .await
        .unwrap();
    let (status, _) = send_json(&app, "POST", "/auth/register", &auth, &creds).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, body) = send_json(&app, "POST", "/auth/register", &auth, &creds).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "User already exists");

    // The users listing shows bob unapproved and without roles.
    let (status, users) = get_json(&app, "/admin/users", &auth).await;
    assert_eq!(status, StatusCode::OK);
    let bob = users
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "bob")
        .expect("bob listed")
        .clone();
    assert_eq!(bob["approved"], false);
    assert_eq!(bob["roles"], json!([]));
    let bob_id = bob["id"].as_i64().unwrap();

    // Unapproved users cannot log in.
    let (status, _) = send_json(&app, "POST", "/auth/login", &auth, &creds).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Approving an unknown ID is 404; approving bob lets him in.
    let (status, _) = send_empty(&app, "POST", "/admin/users/999999/approve", &auth).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let uri = format!("/admin/users/{bob_id}/approve");
    let (status, _) = send_empty(&app, "POST", &uri, &auth).await;
    assert_eq!(status, StatusCode::OK);
    let (status, login) = send_json(&app, "POST", "/auth/login", &auth, &creds).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!login["token"].as_str().unwrap().is_empty());

    // Deleting bob works once, then 404s; his login is gone.
    let uri = format!("/admin/users/{bob_id}");
    let (status, _) = send_empty(&app, "DELETE", &uri, &auth).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send_empty(&app, "DELETE", &uri, &auth).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send_json(&app, "POST", "/auth/login", &auth, &creds).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_logout_and_me_roundtrip() {
    let (app, repo, _token) = app_with_seed().await;

    // Login is refused while auth is disabled.
    services::set_auth_mode(&repo, &AuthMode::Disabled)
        .await
        .unwrap();
    let creds = json!({ "username": "root", "password": "root_password" });
    let (status, _) = send_json(&app, "POST", "/auth/login", "", &creds).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    services::set_auth_mode(&repo, &AuthMode::Local)
        .await
        .unwrap();
    let wrong = json!({ "username": "root", "password": "wrong" });
    let (status, _) = send_json(&app, "POST", "/auth/login", "", &wrong).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, login) = send_json(&app, "POST", "/auth/login", "", &creds).await;
    assert_eq!(status, StatusCode::OK);
    let auth = format!("Bearer {}", login["token"].as_str().unwrap());

    let (status, me) = get_json(&app, "/auth/me", &auth).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["username"], "root");
    assert_eq!(me["approved"], true);
    assert_eq!(me["is_root"], true);
    assert_eq!(me["maintains"], json!([]));

    // Logout kills the session.
    let (status, _) = send_empty(&app, "POST", "/auth/logout", &auth).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = get_json(&app, "/auth/me", &auth).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn change_password_rotates_the_session_token() {
    let (app, repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);
    services::set_auth_mode(&repo, &AuthMode::Local)
        .await
        .unwrap();

    // Wrong old password is refused.
    let (status, _) = send_json(
        &app,
        "POST",
        "/auth/change-password",
        &auth,
        &json!({ "old_password": "wrong", "new_password": "brand-new-pw" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, body) = send_json(
        &app,
        "POST",
        "/auth/change-password",
        &auth,
        &json!({ "old_password": "root_password", "new_password": "brand-new-pw" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let new_auth = format!("Bearer {}", body["token"].as_str().unwrap());
    assert_ne!(new_auth, auth, "a fresh session token is issued");

    // The old token is dead, the new one works, and only the new password
    // logs in.
    let (status, _) = get_json(&app, "/auth/me", &auth).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, me) = get_json(&app, "/auth/me", &new_auth).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["username"], "root");
    let (status, _) = send_json(
        &app,
        "POST",
        "/auth/login",
        "",
        &json!({ "username": "root", "password": "root_password" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send_json(
        &app,
        "POST",
        "/auth/login",
        "",
        &json!({ "username": "root", "password": "brand-new-pw" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn api_token_lifecycle_creates_lists_and_authenticates() {
    let (app, _repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    let (status, created) = send_json(
        &app,
        "POST",
        "/auth/tokens",
        &auth,
        &json!({ "name": "ci-token", "expires_in_days": 30 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(created["name"], "ci-token");
    let secret = created["token"].as_str().unwrap();
    assert!(!secret.is_empty());
    let id = created["id"].as_str().unwrap().to_string();

    let (status, listed) = get_json(&app, "/auth/tokens", &auth).await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<&str> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["ci-token"]);

    // The API token authenticates API routes.
    let api_auth = format!("Bearer {}", secret);
    let (status, report) = get_json(&app, "/report", &api_auth).await;
    assert_eq!(status, StatusCode::OK);
    assert!(report.is_object());

    let uri = format!("/auth/tokens/{id}");
    let (status, _) = send_empty(&app, "DELETE", &uri, &auth).await;
    assert_eq!(status, StatusCode::OK);
    let (_, listed) = get_json(&app, "/auth/tokens", &auth).await;
    assert_eq!(listed, json!([]));
}

// --- Destructive operations ----------------------------------------------

#[tokio::test]
async fn nuke_producers_and_consumers_require_exact_confirmation() {
    let (app, repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    // Seed one Consumer pin so the consumers nuke has something to delete.
    services::require_endpoint(
        &repo,
        services::RequireEndpointParams {
            consumername: "web-ui",
            producername: "demo-svc",
            version: "1.0.0".parse().unwrap(),
            api_type: ApiType::OpenApi,
            path: "/hello",
            method: "GET",
            trunk: false,
            tag: None,
        },
    )
    .await
    .unwrap();

    let (status, body) = send_json(
        &app,
        "POST",
        "/admin/nuke/consumers",
        &auth,
        &json!({ "confirmation": "nope" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "Invalid confirmation");
    let (status, body) = send_json(
        &app,
        "POST",
        "/admin/nuke/consumers",
        &auth,
        &json!({ "confirmation": "DELETE ALL CLIENTS" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "deleted": 1 }));
    let (_, consumers) = get_json(&app, "/admin/consumers", &auth).await;
    assert_eq!(consumers, json!([]));

    let (status, body) = send_json(
        &app,
        "POST",
        "/admin/nuke/producers",
        &auth,
        &json!({ "confirmation": "nope" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "Invalid confirmation");
    let (status, body) = send_json(
        &app,
        "POST",
        "/admin/nuke/producers",
        &auth,
        &json!({ "confirmation": "DELETE ALL SERVICES" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "deleted": 1 }));
    let (_, producers) = get_json(&app, "/admin/producers", &auth).await;
    assert_eq!(producers, json!([]));
}

#[tokio::test]
async fn nuke_users_deletes_only_non_admin_users() {
    let (app, repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    let hash = services::hash_password("some-password").unwrap();
    repo.create_user("plain-user", &hash, true).await.unwrap();

    let (status, body) = send_json(
        &app,
        "POST",
        "/admin/nuke/users",
        &auth,
        &json!({ "confirmation": "wrong" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "Invalid confirmation");

    let (status, body) = send_json(
        &app,
        "POST",
        "/admin/nuke/users",
        &auth,
        &json!({ "confirmation": "DELETE ALL USERS" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "deleted": 1 }));
    assert!(
        repo.find_user("plain-user").await.unwrap().is_none(),
        "the plain user is gone"
    );
    assert!(
        repo.find_user("root").await.unwrap().is_some(),
        "the admin survives the user nuke"
    );
}

#[tokio::test]
async fn nuke_database_wipes_specs_but_keeps_the_acting_admin() {
    let (app, repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    let (status, body) = send_json(
        &app,
        "POST",
        "/admin/nuke/database",
        &auth,
        &json!({ "confirmation": "wrong" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "Invalid confirmation");

    let (status, _) = send_json(
        &app,
        "POST",
        "/admin/nuke/database",
        &auth,
        &json!({ "confirmation": "NUKE DATABASE" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(repo.list_producers().await.unwrap(), Vec::<String>::new());
    assert!(
        repo.find_user("root").await.unwrap().is_some(),
        "the acting admin survives the database nuke"
    );
}

#[tokio::test]
async fn producer_metadata_update_is_reflected_in_the_listing() {
    let (app, _repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    let (status, _) = send_json(
        &app,
        "POST",
        "/admin/producers/metadata",
        &auth,
        &json!({ "name": "demo-svc", "icon": "sun", "domain": "core" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, producers) = get_json(&app, "/admin/producers", &auth).await;
    assert_eq!(status, StatusCode::OK);
    let demo = producers
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "demo-svc")
        .expect("demo-svc listed");
    assert_eq!(demo["icon"], "sun");
    assert_eq!(demo["domain"], "core");
}

// --- API surface: reports, bundles, timelines -----------------------------

#[tokio::test]
async fn report_markdown_and_isolation_render_the_seeded_producer() {
    let (app, repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    // Both reports render the dependency graph, so seed one Consumer pin.
    services::require_endpoint(
        &repo,
        services::RequireEndpointParams {
            consumername: "web-ui",
            producername: "demo-svc",
            version: "1.0.0".parse().unwrap(),
            api_type: ApiType::OpenApi,
            path: "/hello",
            method: "GET",
            trunk: false,
            tag: None,
        },
    )
    .await
    .unwrap();

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/report/markdown")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get(axum::http::header::CONTENT_TYPE).unwrap(),
        "text/markdown; charset=utf-8"
    );
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let markdown = std::str::from_utf8(&body).unwrap();
    assert!(
        markdown.contains("demo-svc") && markdown.contains("web-ui"),
        "markdown report names the dependency: {markdown}"
    );

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/report/isolation")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let isolation = std::str::from_utf8(&body).unwrap();
    assert!(
        isolation.starts_with("# Service Isolation Report"),
        "unexpected isolation report: {isolation}"
    );
    assert!(
        isolation.contains("demo-svc"),
        "isolation report names the connected producer: {isolation}"
    );
}

#[tokio::test]
async fn require_bundle_serves_merged_yaml_and_dry_run_records_nothing() {
    let (app, _repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    // An empty endpoint list is refused.
    let (status, body) = send_json(
        &app,
        "POST",
        "/require-bundle",
        &auth,
        &json!({
            "consumername": "web-ui",
            "producername": "demo-svc",
            "version": "1.0.0",
            "endpoints": [],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"].as_str().unwrap().contains("endpoint"),
        "unexpected error: {body}"
    );

    // Dry run answers the bundle but records no Consumer.
    let bundle = json!({
        "consumername": "web-ui",
        "producername": "demo-svc",
        "version": "1.0.0",
        "endpoints": [ { "path": "/hello", "method": "GET" } ],
        "dry_run": true,
    });
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/require-bundle")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(bundle.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get("X-Sanshain-Resolution").unwrap(),
        "served"
    );
    assert_eq!(res.headers().get("X-Sanshain-Version").unwrap(), "1.0.0");
    assert_eq!(res.headers().get("X-Sanshain-Stability").unwrap(), "ga");
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let yaml = std::str::from_utf8(&body).unwrap();
    assert!(yaml.contains("/hello"), "bundle carries the endpoint");
    let (_, consumers) = get_json(&app, "/admin/consumers", &auth).await;
    assert_eq!(consumers, json!([]), "dry run must not record a Consumer");

    // A real bundle records the pin.
    let mut real = bundle.clone();
    real["dry_run"] = json!(false);
    let (status, _) = send_json(&app, "POST", "/require-bundle", &auth, &real).await;
    assert_eq!(status, StatusCode::OK);
    let (_, consumers) = get_json(&app, "/admin/consumers", &auth).await;
    assert_eq!(consumers, json!(["web-ui"]));
}

#[tokio::test]
async fn require_answers_304_when_the_etag_matches() {
    let (app, _repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    let uri = "/require?consumername=web-ui&producername=demo-svc&version=1.0.0&path=/hello&method=GET&dry_run=true";
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let etag = res.headers().get("ETag").expect("etag present").clone();

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header(axum::http::header::AUTHORIZATION, &auth)
                .header(axum::http::header::IF_NONE_MATCH, &etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_MODIFIED);
}

#[tokio::test]
async fn producer_versions_endpoint_history_and_unknowns() {
    let (app, _repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    let (status, versions) = get_json(&app, "/producers/demo-svc/versions", &auth).await;
    assert_eq!(status, StatusCode::OK);
    let list = versions.as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["version"], "1.0.0");
    assert_eq!(list[0]["stability"], "ga");

    let (status, body) = get_json(&app, "/producers/no-such-svc/versions", &auth).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "Producer 'no-such-svc' not found");

    let (status, history) = get_json(
        &app,
        "/endpoint-versions?service=demo-svc&path=/hello&method=GET",
        &auth,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let entries = history.as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["version"], "1.0.0");

    let (status, _) = get_json(
        &app,
        "/endpoint-versions?service=no-such-svc&path=/hello&method=GET",
        &auth,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn audit_timeline_answers_filtered_entries() {
    let (app, _repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    // Provide through the router so a WRITE audit entry exists.
    let spec = DEMO_SPEC_1_0_0.replace("1.0.0", "1.1.0")
        + "  /extra:\n    get:\n      responses:\n        '200': { description: ok }\n";
    let (status, _) = send_json(
        &app,
        "POST",
        "/provide",
        &auth,
        &json!({
            "producername": "demo-svc",
            "openapi_yaml": spec,
            "stability": "snapshot",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let (status, timeline) = get_json(&app, "/api/audit/timeline?limit=10", &auth).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!timeline.as_array().unwrap().is_empty());

    // The service/version filters are SQL LIKE patterns ('%' wildcard,
    // percent-encoded in the query string).
    let (status, filtered) = get_json(
        &app,
        "/api/audit/timeline?limit=10&action_type=WRITE&service=demo%25&version=1.%25",
        &auth,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let entry = &filtered.as_array().unwrap()[0];
    assert_eq!(entry["action"], "PROVIDE_SPEC");
    assert_eq!(entry["service"], "demo-svc");
    assert_eq!(entry["version"], "1.1.0");
}

#[tokio::test]
async fn provide_rejects_invalid_specs_and_legacy_fields() {
    let (app, _repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    // A document without a parsable info.version is a 400.
    let (status, _) = send_json(
        &app,
        "POST",
        "/provide",
        &auth,
        &json!({
            "producername": "bad-svc",
            "openapi_yaml": "not a spec at all",
            "stability": "snapshot",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 1.x-shaped payloads (branch parameter) are a named 422, not silently
    // accepted.
    let (status, _) = send_json(
        &app,
        "POST",
        "/provide",
        &auth,
        &json!({
            "producername": "bad-svc",
            "openapi_yaml": DEMO_SPEC_1_0_0,
            "stability": "snapshot",
            "branch": "main",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Same for the require query string, which answers 400.
    let (status, _) = get_json(
        &app,
        "/require?consumername=c&producername=demo-svc&version=1.0.0&path=/hello&method=GET&branch=main",
        &auth,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A dry-run provide does not create a version.
    let spec_2 = DEMO_SPEC_1_0_0.replace("1.0.0", "2.0.0");
    let (status, body) = send_json(
        &app,
        "POST",
        "/provide",
        &auth,
        &json!({
            "producername": "demo-svc",
            "openapi_yaml": spec_2,
            "stability": "ga",
            "dry_run": true,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body["version"], "2.0.0");
    let (_, versions) = get_json(&app, "/admin/producers/demo-svc/versions", &auth).await;
    assert_eq!(
        versions.as_array().unwrap().len(),
        1,
        "dry run must not store a new version"
    );

    // Diffing against a version that does not exist is a 404.
    let (status, _) = get_json(
        &app,
        "/admin/producers/demo-svc/diff?from=1.0.0&to=9.9.9",
        &auth,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// --- CSRF middleware ------------------------------------------------------

#[tokio::test]
async fn csrf_validation_gates_cookieless_state_changes() {
    let (app, _repo, _token) = app_with_seed().await;

    // A state-changing request with neither a Bearer token nor a CSRF token
    // is refused outright.
    let (status, _) = send_empty(&app, "POST", "/auth/logout", "").await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // A bogus CSRF token is refused the same way.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/logout")
                .header("X-CSRF-Token", "bogus")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // A token minted by /csrf-token passes the gate.
    let (status, minted) = get_json(&app, "/csrf-token", "").await;
    assert_eq!(status, StatusCode::OK);
    let csrf = minted["csrf_token"].as_str().unwrap().to_string();
    assert_eq!(csrf.len(), 32);
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/logout")
                .header("X-CSRF-Token", &csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // /auth/login is exempt from CSRF: a wrong password answers 401 (the
    // handler spoke), not the middleware's 403.
    let (status, _) = send_json(
        &app,
        "POST",
        "/auth/login",
        "",
        &json!({ "username": "root", "password": "wrong" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
