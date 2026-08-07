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

// ---------------------------------------------------------------------------
// #36 — tag on the wire: hotfix builds update their sanshain-branch
// ---------------------------------------------------------------------------

#[tokio::test]
async fn trunk_and_tag_together_are_rejected() {
    let ctx = setup().await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.0.0"), &[]).await;
    send(&ctx, "POST", "/admin/branches", Some(json!({"name": "R"}))).await;

    let (status, body) = provide_with(
        &ctx,
        "svc",
        "snapshot",
        &spec("1.0.0"),
        &[("trunk", json!(true)), ("tag", json!("R"))],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body}");
    let msg = body["error"].as_str().unwrap();
    assert!(msg.contains("trunk") && msg.contains("tag"), "got: {msg}");

    let (status, _, body) = send(
        &ctx,
        "GET",
        "/require?consumername=c&producername=svc&version=1.0.0&path=/users&method=GET&trunk=true&tag=R",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body}");
}

#[tokio::test]
async fn unknown_tag_is_an_instructive_404() {
    let ctx = setup().await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.0.0"), &[]).await;

    let (status, body) = provide_with(
        &ctx,
        "svc",
        "snapshot",
        &spec("1.0.0"),
        &[("tag", json!("Release Mariboo"))],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "got: {body}");
    let msg = body["error"].as_str().unwrap();
    assert!(msg.contains("releaser must create"), "got: {msg}");

    let (status, _, _) = send(
        &ctx,
        "GET",
        "/require?consumername=c&producername=svc&version=1.0.0&path=/users&method=GET&tag=Ghost",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn tagged_require_updates_only_its_branch() {
    let ctx = setup().await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.0.0"), &[]).await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.1.0"), &[]).await;
    require_with(&ctx, "webapp", "svc", "1.0.0", "/users", "GET", true).await;
    send(&ctx, "POST", "/admin/branches", Some(json!({"name": "R"}))).await;

    // Hotfix: the branch pin moves to 1.1.0; trunk stays at 1.0.0.
    let (status, _, body) = send(
        &ctx,
        "GET",
        "/require?consumername=webapp&producername=svc&version=1.1.0&path=/users&method=GET&tag=R",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");

    let branch = branch_graph(&ctx, "R").await;
    assert_eq!(branch.len(), 1, "got: {branch:?}");
    assert_eq!(branch[0]["version"], "1.1.0");
    let trunk = trunk_graph(&ctx).await;
    assert_eq!(trunk[0]["version"], "1.0.0", "trunk must be untouched");

    // The branch keeps its own history: close + insert.
    let rows: Vec<(Option<String>,)> =
        sqlx::query_as("SELECT valid_to FROM branch_dependencies ORDER BY id")
            .fetch_all(&ctx.pool)
            .await
            .unwrap();
    assert_eq!(rows.len(), 2, "got: {rows:?}");
    assert!(rows[0].0.is_some() && rows[1].0.is_none(), "got: {rows:?}");
}

#[tokio::test]
async fn tagged_provide_marks_the_member_version() {
    let ctx = setup().await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.0.0"), &[]).await;
    send(&ctx, "POST", "/admin/branches", Some(json!({"name": "R"}))).await;

    let (status, body) =
        provide_with(&ctx, "svc", "ga", &spec("1.0.1"), &[("tag", json!("R"))]).await;
    assert_eq!(status, StatusCode::ACCEPTED, "got: {body}");

    let rows: Vec<(i64, i64, i64, Option<String>)> = sqlx::query_as(
        "SELECT major, minor, patch, valid_to FROM branch_member_versions ORDER BY id",
    )
    .fetch_all(&ctx.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1, "got: {rows:?}");
    assert_eq!((rows[0].0, rows[0].1, rows[0].2), (1, 0, 1));
    assert!(rows[0].3.is_none());

    // The trunk marker is NOT set by a tagged provide.
    let versions = versions_of(&ctx, "svc").await;
    let v101 = versions.iter().find(|v| v["version"] == "1.0.1").unwrap();
    assert!(v101["trunk_provided_at"].is_null(), "got: {v101}");
}

// ---------------------------------------------------------------------------
// #38 — sanshain-branch rename and delete (admin bar)
// ---------------------------------------------------------------------------

/// A user holding exactly the `releaser` role — may create branches but is
/// below the admin bar for rename/delete.
#[cfg(test)]
async fn releaser_token(ctx: &TestContext) -> String {
    let repo = SqliteSpecRepository::new(ctx.pool.clone());
    let hash = services::hash_password("rel-pass").unwrap();
    let user = repo.create_user("rel", &hash, true).await.unwrap();
    repo.grant_user_role(user.id, "releaser").await.unwrap();
    repo.create_session(user.id, "2099-12-31T23:59:59")
        .await
        .unwrap()
        .token
}

#[cfg(test)]
async fn send_as(
    ctx: &TestContext,
    token: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, String) {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("Authorization", format!("Bearer {token}"));
    let request = match body {
        Some(json) => builder
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&json).unwrap()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let response = ctx.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

#[tokio::test]
async fn branch_rename_is_admin_gated_and_identity_survives() {
    let ctx = setup().await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.0.0"), &[]).await;
    require_with(&ctx, "webapp", "svc", "1.0.0", "/users", "GET", true).await;
    send(&ctx, "POST", "/admin/branches", Some(json!({"name": "R"}))).await;
    send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "Other"})),
    )
    .await;

    // A releaser may create, but not rename.
    let rel = releaser_token(&ctx).await;
    let (status, _) = send_as(
        &ctx,
        &rel,
        "PUT",
        "/admin/branches/R",
        Some(json!({"new_name": "Maribou"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Collision with a live name → 409; unknown branch → 404.
    let (status, _, _) = send(
        &ctx,
        "PUT",
        "/admin/branches/R",
        Some(json!({"new_name": "Other"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _, _) = send(
        &ctx,
        "PUT",
        "/admin/branches/Ghost",
        Some(json!({"new_name": "X"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The admin renames; membership survives (id-based), old name 404s.
    let (status, _, body) = send(
        &ctx,
        "PUT",
        "/admin/branches/R",
        Some(json!({"new_name": "Maribou"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    let graph = branch_graph(&ctx, "Maribou").await;
    assert_eq!(graph.len(), 1, "membership survives the rename: {graph:?}");
    let (status, _, _) = send(&ctx, "GET", "/admin/branches/R/graph", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Audited with old and new name.
    let audit: Vec<(String,)> =
        sqlx::query_as("SELECT details FROM audit_logs WHERE action = 'BRANCH_RENAMED'")
            .fetch_all(&ctx.pool)
            .await
            .unwrap();
    assert_eq!(audit.len(), 1, "got: {audit:?}");
    assert!(
        audit[0].0.contains("'R'") && audit[0].0.contains("'Maribou'"),
        "got: {}",
        audit[0].0
    );
}

#[tokio::test]
async fn branch_delete_is_admin_gated_and_frees_the_name() {
    let ctx = setup().await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.0.0"), &[]).await;
    require_with(&ctx, "webapp", "svc", "1.0.0", "/users", "GET", true).await;
    send(&ctx, "POST", "/admin/branches", Some(json!({"name": "R"}))).await;

    let rel = releaser_token(&ctx).await;
    let (status, _) = send_as(&ctx, &rel, "DELETE", "/admin/branches/R", None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _, _) = send(&ctx, "DELETE", "/admin/branches/R", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = send(&ctx, "GET", "/admin/branches/R/graph", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The branch's rows are gone with it, and the name is reusable.
    let rows: Vec<(i64,)> = sqlx::query_as("SELECT COUNT(*) FROM branch_dependencies")
        .fetch_all(&ctx.pool)
        .await
        .unwrap();
    assert_eq!(rows[0].0, 0);
    let (status, _, _) = send(&ctx, "POST", "/admin/branches", Some(json!({"name": "R"}))).await;
    assert_eq!(status, StatusCode::CREATED);

    let audit: Vec<(i64,)> =
        sqlx::query_as("SELECT COUNT(*) FROM audit_logs WHERE action = 'BRANCH_DELETED'")
            .fetch_all(&ctx.pool)
            .await
            .unwrap();
    assert_eq!(audit[0].0, 1);
}

// ---------------------------------------------------------------------------
// #35 — trunk TTL: stale trunk data is closed (never deleted)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn trunk_ttl_setting_defaults_to_90_and_roundtrips() {
    let ctx = setup().await;
    let (status, _, body) = send(&ctx, "GET", "/admin/settings/trunk-max-age", None).await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    assert_eq!(as_json(&body)["days"], 90);

    let (status, _, _) = send(
        &ctx,
        "POST",
        "/admin/settings/trunk-max-age",
        Some(json!({"days": 30})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, _, body) = send(&ctx, "GET", "/admin/settings/trunk-max-age", None).await;
    assert_eq!(as_json(&body)["days"], 30);
}

#[tokio::test]
async fn trunk_cleanup_closes_stale_rows_and_keeps_history() {
    let ctx = setup().await;
    provide_with(
        &ctx,
        "old-svc",
        "snapshot",
        &spec("1.0.0"),
        &[("trunk", json!(true))],
    )
    .await;
    provide_with(
        &ctx,
        "fresh-svc",
        "snapshot",
        &spec("1.0.0"),
        &[("trunk", json!(true))],
    )
    .await;
    require_with(&ctx, "webapp", "old-svc", "1.0.0", "/users", "GET", true).await;
    require_with(&ctx, "webapp", "fresh-svc", "1.0.0", "/users", "GET", true).await;

    // Age one producer's trunk data far past the TTL.
    sqlx::query("UPDATE trunk_dependencies SET last_required_at = '2020-01-01T00:00:00Z' WHERE service_id = (SELECT id FROM services WHERE name = 'old-svc')")
        .execute(&ctx.pool).await.unwrap();
    sqlx::query("UPDATE spec_versions SET trunk_provided_at = '2020-01-01T00:00:00Z' WHERE service_id = (SELECT id FROM services WHERE name = 'old-svc')")
        .execute(&ctx.pool).await.unwrap();

    let (status, _, body) = send(&ctx, "POST", "/admin/cleanup/trunk", None).await;
    assert_eq!(status, StatusCode::OK, "got: {body}");

    // The stale pin left the current view — closed, not deleted.
    let edges = trunk_graph(&ctx).await;
    assert_eq!(edges.len(), 1, "got: {edges:?}");
    assert_eq!(edges[0]["service"], "fresh-svc");
    let rows: Vec<(Option<String>,)> = sqlx::query_as(
        "SELECT valid_to FROM trunk_dependencies WHERE service_id = (SELECT id FROM services WHERE name = 'old-svc')",
    )
    .fetch_all(&ctx.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1, "history preserved: {rows:?}");
    assert!(rows[0].0.is_some(), "row closed, not deleted: {rows:?}");

    // The stale trunk marker is cleared; the fresh one survives.
    let old = versions_of(&ctx, "old-svc").await;
    assert!(old[0]["trunk_provided_at"].is_null(), "got: {}", old[0]);
    let fresh = versions_of(&ctx, "fresh-svc").await;
    assert!(
        fresh[0]["trunk_provided_at"].is_string(),
        "got: {}",
        fresh[0]
    );

    // The report names the staleness boundary for the UI.
    let (_, _, body) = send(&ctx, "GET", "/report", None).await;
    assert!(
        as_json(&body)["trunk_stale_before"].is_string(),
        "got: {body}"
    );
}

// ---------------------------------------------------------------------------
// #39 — report scopes: dev (default), main, branch[@date]
// ---------------------------------------------------------------------------

#[tokio::test]
async fn report_scope_selects_the_graph() {
    let ctx = setup().await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.0.0"), &[]).await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.1.0"), &[]).await;
    // Dev activity pins 1.0.0; trunk pins 1.1.0.
    require_with(&ctx, "webapp", "svc", "1.0.0", "/users", "GET", false).await;
    require_with(&ctx, "webapp", "svc", "1.1.0", "/users", "GET", true).await;
    send(&ctx, "POST", "/admin/branches", Some(json!({"name": "R"}))).await;
    // The branch moves on to 1.0.0 via a hotfix; main stays at 1.1.0.
    send(
        &ctx,
        "GET",
        "/require?consumername=webapp&producername=svc&version=1.0.0&path=/users&method=GET&tag=R",
        None,
    )
    .await;

    // Default scope: dev — the recorded dependency set (both pins).
    let (_, _, body) = send(&ctx, "GET", "/report", None).await;
    let dev = as_json(&body)["dependency_graph"].as_array().unwrap().len();
    assert_eq!(dev, 2, "dev scope keeps accumulated pins");

    // Main scope: the current trunk pin set.
    let (status, _, body) = send(&ctx, "GET", "/report?scope=main", None).await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    let main = as_json(&body)["dependency_graph"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(main.len(), 1, "got: {main:?}");
    assert_eq!(main[0]["version"], "1.1.0");

    // Branch scope: the branch's current pins.
    let (status, _, body) = send(&ctx, "GET", "/report?scope=R", None).await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    let branch = as_json(&body)["dependency_graph"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(branch.len(), 1, "got: {branch:?}");
    assert_eq!(branch[0]["version"], "1.0.0");

    // Unknown branch scope → 404; the markdown report is scoped too.
    let (status, _, _) = send(&ctx, "GET", "/report?scope=Ghost", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, _, md) = send(&ctx, "GET", "/report/markdown?scope=main", None).await;
    assert!(md.contains("1.1.0") && !md.contains("1.0.0"), "got: {md}");
}

// ---------------------------------------------------------------------------
// #37 — dangling branch references and delete-version warnings
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_version_warning_names_referencing_branches() {
    let ctx = setup().await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.0.0"), &[]).await;
    require_with(&ctx, "webapp", "svc", "1.0.0", "/users", "GET", true).await;
    send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "Maribou"})),
    )
    .await;

    let (status, _, body) = send(
        &ctx,
        "GET",
        "/admin/producers/svc/versions/openapi/1.0.0/dependents",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    let dependents = as_json(&body);
    let names: Vec<&str> = dependents
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d.as_str().unwrap())
        .collect();
    assert!(names.contains(&"webapp"), "got: {names:?}");
    assert!(
        names.iter().any(|n| n.contains("Maribou")),
        "branch reference must be named: {names:?}"
    );
}

#[tokio::test]
async fn dangling_branch_reference_is_marked_and_heals_on_reprovide() {
    let ctx = setup().await;
    let doc = spec("1.0.0");
    provide_with(&ctx, "svc", "snapshot", &doc, &[]).await;
    require_with(&ctx, "webapp", "svc", "1.0.0", "/users", "GET", true).await;
    send(&ctx, "POST", "/admin/branches", Some(json!({"name": "R"}))).await;

    let graph = branch_graph(&ctx, "R").await;
    assert_eq!(graph[0]["dangling"], Value::Null, "got: {}", graph[0]);

    // Delete the version: the branch keeps the reference, visibly dangling.
    let (status, _, _) = send(
        &ctx,
        "DELETE",
        "/admin/producers/svc/versions/openapi/1.0.0",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let graph = branch_graph(&ctx, "R").await;
    assert_eq!(graph.len(), 1, "reference survives: {graph:?}");
    assert_eq!(graph[0]["dangling"], true, "got: {}", graph[0]);

    // Re-providing the number heals the reference automatically.
    provide_with(&ctx, "svc", "snapshot", &doc, &[]).await;
    let graph = branch_graph(&ctx, "R").await;
    assert_eq!(graph[0]["dangling"], Value::Null, "got: {}", graph[0]);
}

// ---------------------------------------------------------------------------
// #41 — audit stream stamping and filter
// ---------------------------------------------------------------------------

#[tokio::test]
async fn audit_entries_carry_their_stream_and_filter_by_it() {
    let ctx = setup().await;
    provide_with(
        &ctx,
        "svc",
        "snapshot",
        &spec("1.0.0"),
        &[("trunk", json!(true))],
    )
    .await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.1.0"), &[]).await;
    send(&ctx, "POST", "/admin/branches", Some(json!({"name": "R"}))).await;
    provide_with(&ctx, "svc", "ga", &spec("1.0.1"), &[("tag", json!("R"))]).await;
    require_with(&ctx, "webapp", "svc", "1.0.0", "/users", "GET", true).await;

    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT action, stream FROM audit_logs WHERE action IN ('PROVIDE_SPEC', 'REQUIRE_SPEC') ORDER BY id",
    )
    .fetch_all(&ctx.pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 4, "got: {rows:?}");
    assert_eq!(rows[0].1.as_deref(), Some("trunk"));
    assert_eq!(rows[1].1, None, "plain provide carries no stream: {rows:?}");
    assert_eq!(rows[2].1.as_deref(), Some("R"));
    assert_eq!(rows[3].1.as_deref(), Some("trunk"));

    // The timeline filters by stream.
    let (status, _, body) = send(&ctx, "GET", "/api/audit/timeline?stream=R", None).await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    let entries = as_json(&body);
    let actions: Vec<&str> = entries
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    assert_eq!(actions, vec!["PROVIDE_SPEC"], "got: {entries}");
    assert_eq!(entries[0]["stream"], "R");
}

// ---------------------------------------------------------------------------
// #40 — timeline: the main graph at any date, with change markers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn trunk_graph_is_reconstructable_at_a_date_with_change_markers() {
    let ctx = setup().await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.0.0"), &[]).await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.1.0"), &[]).await;

    require_with(&ctx, "webapp", "svc", "1.0.0", "/users", "GET", true).await;
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let mid = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    require_with(&ctx, "webapp", "svc", "1.1.0", "/users", "GET", true).await;

    // At `mid`, the main graph pinned 1.0.0; now it pins 1.1.0.
    let (status, _, body) = send(
        &ctx,
        "GET",
        &format!("/admin/trunk/graph?at={}", mid.replace(':', "%3A")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    let at_mid = as_json(&body);
    assert_eq!(at_mid.as_array().unwrap().len(), 1, "got: {at_mid}");
    assert_eq!(at_mid[0]["version"], "1.0.0");
    let (_, _, body) = send(&ctx, "GET", "/admin/trunk/graph", None).await;
    assert_eq!(as_json(&body)[0]["version"], "1.1.0");

    // The timeline lists the change instants (open + close events).
    let (status, _, body) = send(&ctx, "GET", "/admin/trunk/timeline", None).await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    let dates = as_json(&body);
    let dates = dates.as_array().unwrap();
    assert!(dates.len() >= 2, "got: {dates:?}");
    // Sorted ascending, RFC 3339.
    let strs: Vec<&str> = dates.iter().map(|d| d.as_str().unwrap()).collect();
    let mut sorted = strs.clone();
    sorted.sort();
    assert_eq!(strs, sorted);

    // A branch has its own timeline endpoint.
    send(&ctx, "POST", "/admin/branches", Some(json!({"name": "R"}))).await;
    let (status, _, body) = send(&ctx, "GET", "/admin/branches/R/timeline", None).await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    assert!(!as_json(&body).as_array().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// #42 — reverse lookup and scoped-export stamping
// ---------------------------------------------------------------------------

#[tokio::test]
async fn producer_branch_memberships_answer_the_reverse_lookup() {
    let ctx = setup().await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.0.0"), &[]).await;
    require_with(&ctx, "webapp", "svc", "1.0.0", "/users", "GET", true).await;
    send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "Maribou"})),
    )
    .await;
    send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "Nightjar"})),
    )
    .await;

    let (status, _, body) =
        send(&ctx, "GET", "/admin/producers/svc/branch-memberships", None).await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    let memberships = as_json(&body);
    let rows = memberships.as_array().unwrap();
    assert_eq!(rows.len(), 2, "got: {rows:?}");
    let branches: Vec<&str> = rows.iter().map(|m| m["branch"].as_str().unwrap()).collect();
    assert!(branches.contains(&"Maribou") && branches.contains(&"Nightjar"));
    assert_eq!(rows[0]["version"], "1.0.0");
}

#[tokio::test]
async fn scoped_markdown_report_names_its_scope() {
    let ctx = setup().await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.0.0"), &[]).await;
    require_with(&ctx, "webapp", "svc", "1.0.0", "/users", "GET", true).await;

    let (_, _, md) = send(&ctx, "GET", "/report/markdown?scope=main", None).await;
    assert!(md.contains("Scope: main"), "got: {md}");
    let (_, _, md) = send(&ctx, "GET", "/report/markdown", None).await;
    assert!(
        !md.contains("Scope:"),
        "unscoped report stays unchanged: {md}"
    );
}

// ---------------------------------------------------------------------------
// #43 — diff between any two (graph, date) selections
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graph_diff_names_added_removed_and_changed_pins() {
    let ctx = setup().await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.0.0"), &[]).await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.1.0"), &[]).await;
    require_with(&ctx, "webapp", "svc", "1.0.0", "/users", "GET", true).await;
    send(&ctx, "POST", "/admin/branches", Some(json!({"name": "R"}))).await;
    // Trunk moves on; the branch keeps 1.0.0.
    require_with(&ctx, "webapp", "svc", "1.1.0", "/users", "GET", true).await;
    // And trunk gains a second consumer the branch never saw.
    require_with(&ctx, "mobile", "svc", "1.1.0", "/users", "GET", true).await;

    let (status, _, body) = send(&ctx, "GET", "/admin/graph/diff?left=R&right=main", None).await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    let diff = as_json(&body);
    assert_eq!(diff["left"], "R");
    assert_eq!(diff["right"], "main");

    let changed = diff["pins_changed"].as_array().unwrap();
    assert_eq!(changed.len(), 1, "got: {changed:?}");
    assert_eq!(changed[0]["client"], "webapp");
    assert_eq!(changed[0]["from"], "1.0.0");
    assert_eq!(changed[0]["to"], "1.1.0");

    let added = diff["pins_added"].as_array().unwrap();
    assert_eq!(added.len(), 1, "got: {added:?}");
    assert_eq!(added[0]["client"], "mobile");
    assert!(diff["pins_removed"].as_array().unwrap().is_empty());

    // Same selection twice: explicitly no differences.
    let (_, _, body) = send(&ctx, "GET", "/admin/graph/diff?left=main&right=main", None).await;
    let diff = as_json(&body);
    assert!(diff["pins_changed"].as_array().unwrap().is_empty());
    assert!(diff["pins_added"].as_array().unwrap().is_empty());
    assert!(diff["pins_removed"].as_array().unwrap().is_empty());

    // Unknown branch → 404.
    let (status, _, _) = send(&ctx, "GET", "/admin/graph/diff?left=Ghost&right=main", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Review fixes — reserved branch names, instant normalization, dangling pins
// in the main view, and the shared report selector grammar
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reserved_graph_names_cannot_name_a_branch() {
    let ctx = setup().await;
    for name in ["main", "dev"] {
        let (status, _, body) = send(
            &ctx,
            "POST",
            "/admin/branches",
            Some(json!({ "name": name })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body}");
        assert!(body.contains("reserved"), "got: {body}");
    }

    // Renaming an existing branch into a reserved name is refused the same way.
    let (status, _, body) = send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({ "name": "rel-1" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "got: {body}");
    let (status, _, body) = send(
        &ctx,
        "PUT",
        "/admin/branches/rel-1",
        Some(json!({ "new_name": "main" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body}");
}

#[tokio::test]
async fn timeline_instants_normalize_offset_and_precision() {
    let ctx = setup().await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.0.0"), &[]).await;
    assert_eq!(
        require_with(&ctx, "web", "svc", "1.0.0", "/users", "GET", true).await,
        StatusCode::OK
    );

    let (status, _, body) = send(&ctx, "GET", "/admin/trunk/graph", None).await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    let pins = as_json(&body);
    let valid_from = pins[0]["valid_from"].as_str().unwrap().to_string();
    assert!(valid_from.ends_with('Z'), "got: {valid_from}");

    // The exact pin instant, expressed with a -02:00 offset and with
    // millisecond precision: both name the same instant as the stored Z form,
    // so both must select the pin (valid_from <= at).
    let offset_form = chrono::DateTime::parse_from_rfc3339(&valid_from)
        .unwrap()
        .with_timezone(&chrono::FixedOffset::west_opt(2 * 3600).unwrap())
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, false);
    let millis_form = format!("{}.000Z", valid_from.trim_end_matches('Z'));
    for at in [offset_form.as_str(), millis_form.as_str()] {
        let (status, _, body) =
            send(&ctx, "GET", &format!("/admin/trunk/graph?at={at}"), None).await;
        assert_eq!(status, StatusCode::OK, "at={at}: {body}");
        let pins = as_json(&body);
        assert_eq!(pins.as_array().unwrap().len(), 1, "at={at}: {pins}");
        assert_eq!(pins[0]["version"], "1.0.0", "at={at}");
    }

    // A non-date instant is a 400, not a silently empty graph.
    let (status, _, body) = send(&ctx, "GET", "/admin/trunk/graph?at=banana", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body}");
}

#[tokio::test]
async fn deleted_version_is_dangling_in_the_main_graph_too() {
    let ctx = setup().await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.0.0"), &[]).await;
    assert_eq!(
        require_with(&ctx, "web", "svc", "1.0.0", "/users", "GET", true).await,
        StatusCode::OK
    );

    let (status, _, body) = send(
        &ctx,
        "DELETE",
        "/admin/producers/svc/versions/openapi/1.0.0",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");

    // The admin main-graph endpoint and the report's trunk graph both mark
    // the pin dangling — a deleted version must look broken in every view.
    let (status, _, body) = send(&ctx, "GET", "/admin/trunk/graph", None).await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    assert_eq!(as_json(&body)[0]["dangling"], true, "got: {body}");
    let report_pins = trunk_graph(&ctx).await;
    assert_eq!(report_pins[0]["dangling"], true, "got: {report_pins:?}");
}

#[tokio::test]
async fn report_scope_speaks_the_shared_selector_grammar() {
    let ctx = setup().await;
    provide_with(&ctx, "svc", "snapshot", &spec("1.0.0"), &[]).await;
    assert_eq!(
        require_with(&ctx, "web", "svc", "1.0.0", "/users", "GET", true).await,
        StatusCode::OK
    );

    // main@instant reads the trunk graph as it was — the same grammar the
    // diff endpoint speaks, offsets normalized into the stored UTC form.
    let (status, _, body) = send(
        &ctx,
        "GET",
        "/report?scope=main@2100-01-01T02:00:00%2B02:00",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    let report = as_json(&body);
    assert_eq!(report["scope_label"], "main@2100-01-01T00:00:00Z");
    assert_eq!(report["dependency_graph"].as_array().unwrap().len(), 1);

    // Before the pin existed: an empty graph, not an error.
    let (status, _, body) =
        send(&ctx, "GET", "/report?scope=main@2000-01-01T00:00:00Z", None).await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    assert!(
        as_json(&body)["dependency_graph"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // The dev scope has no timeline — refused, not misread as a branch name.
    let (status, _, body) = send(&ctx, "GET", "/report?scope=dev@2100-01-01T00:00:00Z", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body}");
}
