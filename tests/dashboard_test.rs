//! The landing page's dashboard at the router seam: numbers for everyone,
//! names only for a signed-in caller.

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
    let repo = SqliteSpecRepository::new(pool);
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
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, HeaderMap, String) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header("Authorization", format!("Bearer {token}"));
    }
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

const SPEC: &str = "openapi: 3.0.0\ninfo:\n  title: penguin\n  version: 1.0.0\npaths:\n  /fish:\n    get:\n      responses:\n        '200': { description: ok }\n";

#[tokio::test]
async fn the_numbers_are_public_and_the_names_need_a_sign_in() {
    let ctx = setup().await;
    let (status, _, body) = send(
        &ctx,
        "POST",
        "/provide",
        Some(&ctx.token),
        Some(json!({ "producername": "penguin", "stability": "snapshot", "openapi_yaml": SPEC })),
    )
    .await;
    assert!(status.is_success(), "{status} {body}");

    let (status, _, body) = send(&ctx, "GET", "/api/dashboard", None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let stats = as_json(&body);
    assert_eq!(stats["producers"], 1);
    assert_eq!(stats["consumers"], 0);
    assert_eq!(stats["versions"], 1);
    assert_eq!(stats["provides_last_7_days"], 1);
    let days = stats["provides_per_day"].as_array().unwrap();
    assert_eq!(days.len(), 30);
    assert_eq!(days[29]["day"], Utc::now().format("%Y-%m-%d").to_string());
    assert_eq!(days[29]["provides"], 1);
    assert!(
        !body.contains("penguin"),
        "no names without a sign-in: {body}"
    );

    let (status, _, _) = send(&ctx, "GET", "/api/dashboard/recent", None, None).await;
    assert!(
        matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN),
        "the names need a sign-in: {status}"
    );

    let (status, _, body) =
        send(&ctx, "GET", "/api/dashboard/recent", Some(&ctx.token), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let recent = as_json(&body);
    let recent = recent.as_array().unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0]["producer"], "penguin");
    assert_eq!(recent[0]["version"], "1.0.0");
    assert_eq!(recent[0]["username"], "root");
    assert!(recent[0]["display_name"].is_null());
    assert_eq!(recent[0]["external"], false);

    let (status, _, body) = send(
        &ctx,
        "PUT",
        "/auth/display-name",
        Some(&ctx.token),
        Some(json!({ "display_name": "Root Admin" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, _, body) = send(&ctx, "GET", "/api/dashboard/recent", Some(&ctx.token), None).await;
    assert_eq!(as_json(&body)[0]["display_name"], "Root Admin");
}

#[tokio::test]
async fn an_external_producer_is_marked_in_the_lists() {
    let ctx = setup().await;
    let (status, _, body) = send(
        &ctx,
        "POST",
        "/provide",
        Some(&ctx.token),
        Some(json!({ "producername": "penguin", "stability": "snapshot", "openapi_yaml": SPEC })),
    )
    .await;
    assert!(status.is_success(), "{status} {body}");

    let (status, _, body) = send(
        &ctx,
        "PUT",
        "/admin/producers/penguin/external",
        Some(&ctx.token),
        Some(json!({ "external": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, _, body) = send(&ctx, "GET", "/admin/producers", Some(&ctx.token), None).await;
    let producers = as_json(&body);
    assert_eq!(producers[0]["name"], "penguin");
    assert_eq!(producers[0]["external"], true);
    let (_, _, body) = send(&ctx, "GET", "/api/dashboard/recent", Some(&ctx.token), None).await;
    assert_eq!(as_json(&body)[0]["external"], true);

    let (status, _, _) = send(
        &ctx,
        "PUT",
        "/admin/producers/nobody/external",
        Some(&ctx.token),
        Some(json!({ "external": true })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_provide_on_the_admin_page_marks_only_the_producer_it_creates() {
    let ctx = setup().await;
    let (status, _, body) = send(
        &ctx,
        "POST",
        "/provide",
        Some(&ctx.token),
        Some(json!({ "producername": "ours", "stability": "snapshot", "openapi_yaml": SPEC })),
    )
    .await;
    assert!(status.is_success(), "{status} {body}");

    // A dry run creates nothing, so it marks nothing.
    let admin_provide = |name: &str, dry_run: bool| {
        json!({
            "producername": name,
            "api_type": "openapi",
            "content": SPEC,
            "stability": "snapshot",
            "dry_run": dry_run,
        })
    };
    for (name, dry_run) in [("penguin", true), ("penguin", false), ("ours", false)] {
        let (status, _, body) = send(
            &ctx,
            "POST",
            "/admin/producers/provide",
            Some(&ctx.token),
            Some(admin_provide(name, dry_run)),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{name} {dry_run}: {body}");
    }

    let (_, _, body) = send(&ctx, "GET", "/admin/producers", Some(&ctx.token), None).await;
    let external: HashMap<String, bool> = as_json(&body)
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["name"].as_str().unwrap().to_string(),
                p["external"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        external,
        HashMap::from([("ours".to_string(), false), ("penguin".to_string(), true)])
    );

    // A refused spec creates nothing and marks nothing.
    let (status, _, _) = send(
        &ctx,
        "POST",
        "/admin/producers/provide",
        Some(&ctx.token),
        Some(json!({
            "producername": "broken",
            "api_type": "openapi",
            "content": "not a spec",
            "stability": "snapshot",
        })),
    )
    .await;
    assert!(status.is_client_error(), "{status}");
    let (_, _, body) = send(&ctx, "GET", "/admin/producers", Some(&ctx.token), None).await;
    assert_eq!(as_json(&body).as_array().unwrap().len(), 2);
}
