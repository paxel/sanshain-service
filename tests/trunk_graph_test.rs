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
    pool: sqlx::SqlitePool,
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
        pool,
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

// ---------------------------------------------------------------------------
// #32 — trunk require maintains the append-only trunk pin set
// ---------------------------------------------------------------------------

#[cfg(test)]
async fn require_with(
    ctx: &TestContext,
    consumer: &str,
    producer: &str,
    version: &str,
    path: &str,
    method: &str,
    trunk: bool,
) -> StatusCode {
    let trunk_param = if trunk { "&trunk=true" } else { "" };
    let uri = format!(
        "/require?consumername={consumer}&producername={producer}&version={version}&path={path}&method={method}{trunk_param}"
    );
    let (status, _, _) = send(ctx, "GET", &uri, None).await;
    status
}

/// The current trunk edges of the report payload.
#[cfg(test)]
async fn trunk_graph(ctx: &TestContext) -> Vec<Value> {
    let (status, _, body) = send(ctx, "GET", "/report", None).await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    as_json(&body)["trunk_graph"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

#[cfg(test)]
async fn trunk_dependency_rows(ctx: &TestContext) -> Vec<(Option<String>, i64, i64, i64)> {
    sqlx::query_as::<_, (Option<String>, i64, i64, i64)>(
        "SELECT valid_to, major, minor, patch FROM trunk_dependencies ORDER BY id",
    )
    .fetch_all(&ctx.pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn trunk_require_maintains_append_only_pin_set() {
    let ctx = setup().await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.0.0"), &[]).await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.1.0"), &[]).await;

    let status = require_with(&ctx, "webapp", "svc", "1.0.0", "/users", "GET", true).await;
    assert_eq!(status, StatusCode::OK);

    let edges = trunk_graph(&ctx).await;
    assert_eq!(edges.len(), 1, "got: {edges:?}");
    assert_eq!(edges[0]["client"], "webapp");
    assert_eq!(edges[0]["service"], "svc");
    assert_eq!(edges[0]["version"], "1.0.0");

    // Re-pin to a newer version: the trunk graph shows only the newer pin…
    let status = require_with(&ctx, "webapp", "svc", "1.1.0", "/users", "GET", true).await;
    assert_eq!(status, StatusCode::OK);
    let edges = trunk_graph(&ctx).await;
    assert_eq!(edges.len(), 1, "got: {edges:?}");
    assert_eq!(edges[0]["version"], "1.1.0");

    // …while the storage keeps history: one closed record, one open.
    let rows = trunk_dependency_rows(&ctx).await;
    assert_eq!(rows.len(), 2, "append-only: close + insert, got {rows:?}");
    assert!(rows[0].0.is_some(), "old pin must be closed: {rows:?}");
    assert_eq!((rows[0].1, rows[0].2, rows[0].3), (1, 0, 0));
    assert!(rows[1].0.is_none(), "new pin must be open: {rows:?}");
    assert_eq!((rows[1].1, rows[1].2, rows[1].3), (1, 1, 0));
}

#[tokio::test]
async fn identical_trunk_require_refreshes_the_open_record() {
    let ctx = setup().await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.0.0"), &[]).await;

    for _ in 0..2 {
        let status = require_with(&ctx, "webapp", "svc", "1.0.0", "/users", "GET", true).await;
        assert_eq!(status, StatusCode::OK);
    }

    let rows = trunk_dependency_rows(&ctx).await;
    assert_eq!(rows.len(), 1, "identical re-pin must not append: {rows:?}");
    assert!(
        rows[0].0.is_none(),
        "the single record stays open: {rows:?}"
    );
}

#[tokio::test]
async fn plain_require_and_dry_run_record_no_trunk_pin() {
    let ctx = setup().await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.0.0"), &[]).await;

    let status = require_with(&ctx, "webapp", "svc", "1.0.0", "/users", "GET", false).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = send(
        &ctx,
        "GET",
        "/require?consumername=webapp&producername=svc&version=1.0.0&path=/users&method=GET&trunk=true&dry_run=true",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert!(trunk_dependency_rows(&ctx).await.is_empty());
    // The normal dependency edge exists as before.
    let (_, _, body) = send(&ctx, "GET", "/report", None).await;
    let deps = as_json(&body)["dependency_graph"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(deps.len(), 1, "got: {deps:?}");
}

#[tokio::test]
async fn trunk_require_bundle_records_pins_for_all_endpoints() {
    let ctx = setup().await;
    let two = "openapi: 3.0.3\ninfo:\n  title: T\n  version: 1.0.0\npaths:\n  /users:\n    get:\n      responses:\n        '200':\n          description: OK\n  /orders:\n    get:\n      responses:\n        '200':\n          description: OK\n";
    provide_with(&ctx, "svc", "snapshot", two, &[]).await;

    let payload = json!({
        "consumername": "webapp",
        "producername": "svc",
        "version": "1.0.0",
        "trunk": true,
        "endpoints": [
            {"path": "/users", "method": "GET"},
            {"path": "/orders", "method": "GET"}
        ]
    });
    let (status, _, body) = send(&ctx, "POST", "/require-bundle", Some(payload)).await;
    assert_eq!(status, StatusCode::OK, "got: {body}");

    let edges = trunk_graph(&ctx).await;
    assert_eq!(edges.len(), 2, "got: {edges:?}");
}

// ---------------------------------------------------------------------------
// #34 — sanshain-branches: create and read, incl. retroactive at-date
// ---------------------------------------------------------------------------

/// A second, unprivileged user (no roles at all) with a real session.
#[cfg(test)]
async fn unprivileged_token(ctx: &TestContext) -> String {
    let repo = SqliteSpecRepository::new(ctx.pool.clone());
    let hash = services::hash_password("dev-pass").unwrap();
    let user = repo.create_user("dev", &hash, true).await.unwrap();
    repo.create_session(user.id, "2099-12-31T23:59:59")
        .await
        .unwrap()
        .token
}

#[cfg(test)]
async fn branch_graph(ctx: &TestContext, name: &str) -> Vec<Value> {
    let encoded = name.replace(' ', "%20");
    let (status, _, body) = send(
        ctx,
        "GET",
        &format!("/admin/branches/{encoded}/graph"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    as_json(&body).as_array().unwrap().clone()
}

#[tokio::test]
async fn branch_creation_is_releaser_gated_and_names_are_unique() {
    let ctx = setup().await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.0.0"), &[]).await;
    require_with(&ctx, "webapp", "svc", "1.0.0", "/users", "GET", true).await;

    // An unprivileged user may not create branches.
    let dev_token = unprivileged_token(&ctx).await;
    let request = Request::builder()
        .method("POST")
        .uri("/admin/branches")
        .header("Authorization", format!("Bearer {dev_token}"))
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({"name": "Release Maribou"})).unwrap(),
        ))
        .unwrap();
    let response = ctx.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // The admin (implicit releaser) creates it; a duplicate name answers 409.
    let (status, _, body) = send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "Release Maribou"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "got: {body}");
    let created = as_json(&body);
    assert_eq!(created["name"], "Release Maribou");
    assert_eq!(created["source"], "trunk");

    let (status, _, body) = send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "Release Maribou"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "got: {body}");

    // The listing names it, and its graph carries the copied trunk pin.
    let (status, _, body) = send(&ctx, "GET", "/admin/branches", None).await;
    assert_eq!(status, StatusCode::OK);
    let listing = as_json(&body);
    let names: Vec<&str> = listing
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["Release Maribou"]);

    let graph = branch_graph(&ctx, "Release Maribou").await;
    assert_eq!(graph.len(), 1, "got: {graph:?}");
    assert_eq!(graph[0]["client"], "webapp");
    assert_eq!(graph[0]["version"], "1.0.0");
}

#[tokio::test]
async fn retroactive_branch_equals_the_main_graph_as_it_was() {
    let ctx = setup().await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.0.0"), &[]).await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.1.0"), &[]).await;

    require_with(&ctx, "webapp", "svc", "1.0.0", "/users", "GET", true).await;
    // Give the cut date its own second, then move trunk on.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let cut = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    require_with(&ctx, "webapp", "svc", "1.1.0", "/users", "GET", true).await;

    // Retroactive: the branch equals the main graph as it was at the cut.
    let (status, _, body) = send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "Maribou", "as_of": cut})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "got: {body}");
    let graph = branch_graph(&ctx, "Maribou").await;
    assert_eq!(graph.len(), 1, "got: {graph:?}");
    assert_eq!(graph[0]["version"], "1.0.0");

    // A branch off the current state sees the newer pin; branching off an
    // existing branch copies that branch's graph.
    let (status, _, _) = send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "Nightjar"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(branch_graph(&ctx, "Nightjar").await[0]["version"], "1.1.0");

    let (status, _, body) = send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "Maribou-LTS", "source": "Maribou"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "got: {body}");
    assert_eq!(
        branch_graph(&ctx, "Maribou-LTS").await[0]["version"],
        "1.0.0"
    );

    // An unknown source answers 404.
    let (status, _, _) = send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "Broken", "source": "Ghost"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
