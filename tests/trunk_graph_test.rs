//! Trunk-flag behavior at the router seam (ADR-0004): trunk provides mark the
//! version entry as trunk's current version; trunk requires maintain the
//! append-only trunk pin set. Absent flags leave 2.2.0 behavior untouched.

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use chrono::Utc;
use sanshain_service::application::services;
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

fn test_app_state(repo: SqliteSpecRepository) -> AppState {
    let (spec_updated_tx, _) = tokio::sync::broadcast::channel(100);
    AppState {
        repo: CachedSpecRepository::new(DatabaseRepo::Sqlite(repo), 256),
        db_url: "sqlite::memory:".to_string(),
        dev_user: None,
        csrf_tokens: Arc::new(RwLock::new(HashMap::new())),
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

#[cfg(test)]
struct TestContext {
    app: axum::Router,
    /// Session token of a fully privileged user, for both API and admin calls.
    token: String,
}

// `cfg(test)` is always true in this crate; the attribute marks the helpers as
// test code for clippy's `allow-unwrap-in-tests`.
#[cfg(test)]
async fn setup() -> TestContext {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool.clone());
    repo.run_migrations().await.unwrap();

    // A real seeded user with a real session token — no auth bypasses.
    let hash = services::hash_password("root-pass").unwrap();
    let user = repo.create_user("root", &hash, true).await.unwrap();
    repo.grant_user_role(user.id, "admin").await.unwrap();
    let session = repo
        .create_session(user.id, "2099-12-31T23:59:59")
        .await
        .unwrap();

    let app = create_app(test_app_state(repo));
    TestContext {
        app,
        token: session.token,
    }
}

#[cfg(test)]
async fn send(
    ctx: &TestContext,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, HeaderMap, String) {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("Authorization", format!("Bearer {}", ctx.token));
    let request = match body {
        Some(json) => builder
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&json).unwrap()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let response = ctx.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, headers, String::from_utf8_lossy(&bytes).to_string())
}

#[cfg(test)]
fn as_json(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or_else(|e| panic!("expected JSON, got {e}: {body}"))
}

/// A minimal one-endpoint spec.
#[cfg(test)]
fn spec(version: &str) -> String {
    format!(
        "openapi: 3.0.3\ninfo:\n  title: T\n  version: {version}\npaths:\n  /users:\n    get:\n      responses:\n        '200':\n          description: OK\n"
    )
}

#[cfg(test)]
async fn provide_with(
    ctx: &TestContext,
    producer: &str,
    stability: &str,
    yaml: &str,
    extra: &[(&str, Value)],
) -> (StatusCode, Value) {
    let mut payload = json!({
        "producername": producer,
        "openapi_yaml": yaml,
        "stability": stability,
    });
    for (k, v) in extra {
        payload[k] = v.clone();
    }
    let (status, _, body) = send(ctx, "POST", "/provide", Some(payload)).await;
    (status, as_json(&body))
}

/// The versions listing of one producer, as the UI reads it.
#[cfg(test)]
async fn versions_of(ctx: &TestContext, producer: &str) -> Vec<Value> {
    let (status, _, body) = send(
        ctx,
        "GET",
        &format!("/admin/producers/{producer}/versions"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    as_json(&body).as_array().unwrap().clone()
}

// ---------------------------------------------------------------------------
// #31 — trunk provide marks the version entry
// ---------------------------------------------------------------------------

#[tokio::test]
async fn trunk_provide_marks_the_version_and_plain_provide_does_not() {
    let ctx = setup().await;

    let (status, _) = provide_with(
        &ctx,
        "svc",
        "snapshot",
        &spec("1.0.0"),
        &[("trunk", json!(true))],
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let (status, _) = provide_with(&ctx, "svc", "snapshot", &spec("1.1.0"), &[]).await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let versions = versions_of(&ctx, "svc").await;
    let v100 = versions.iter().find(|v| v["version"] == "1.0.0").unwrap();
    let v110 = versions.iter().find(|v| v["version"] == "1.1.0").unwrap();
    assert!(
        v100["trunk_provided_at"].is_string(),
        "trunk provide must stamp the marker, got: {v100}"
    );
    assert!(
        v110["trunk_provided_at"].is_null(),
        "plain provide must not stamp, got: {v110}"
    );
}

#[tokio::test]
async fn trunk_dry_run_does_not_mark_but_noop_reprovide_refreshes() {
    let ctx = setup().await;

    // Dry-run trunk provide: nothing may be written.
    let (status, _) = provide_with(
        &ctx,
        "svc",
        "snapshot",
        &spec("1.0.0"),
        &[("trunk", json!(true)), ("dry_run", json!(true))],
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    // Nothing was written — the producer does not even exist.
    let (status, _, body) = send(&ctx, "GET", "/admin/producers/svc/versions", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "got: {body}");

    // Plain provide first, then an identical trunk re-provide (the idempotent
    // no-op): the no-op must still stamp the trunk marker — a nightly trunk
    // CI whose content didn't change is exactly what keeps trunk fresh.
    let (status, _) = provide_with(&ctx, "svc", "snapshot", &spec("1.0.0"), &[]).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let (status, _) = provide_with(
        &ctx,
        "svc",
        "snapshot",
        &spec("1.0.0"),
        &[("trunk", json!(true))],
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let versions = versions_of(&ctx, "svc").await;
    assert!(
        versions[0]["trunk_provided_at"].is_string(),
        "no-op trunk re-provide must stamp the marker, got: {}",
        versions[0]
    );
}
