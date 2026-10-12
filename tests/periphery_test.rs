//! Periphery from Backstage catalog-info.yaml (#46) at the router seam: the
//! push endpoint, and how the recorded Peripheral Resources reach the report,
//! the graph views and sanshain-branches.

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
fn catalog(name: &str, depends_on: &[&str]) -> String {
    let refs: String = depends_on.iter().map(|r| format!("    - {r}\n")).collect();
    format!(
        "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: {name}\nspec:\n  type: service\n  dependsOn:\n{refs}"
    )
}

#[cfg(test)]
async fn push(ctx: &TestContext, system: &str, yaml: &str, trunk: bool) -> (StatusCode, Value) {
    let (status, _, body) = send(
        ctx,
        "PUT",
        &format!("/catalog/{system}"),
        Some(json!({"catalog_info_yaml": yaml, "trunk": trunk})),
    )
    .await;
    let value = serde_json::from_str(&body).unwrap_or(Value::String(body));
    (status, value)
}

#[tokio::test]
async fn any_authenticated_caller_may_push_and_gets_the_recorded_set_back() {
    let ctx = setup().await;
    let dev_token = unprivileged_token(&ctx).await;
    let request = Request::builder()
        .method("PUT")
        .uri("/catalog/orders")
        .header("Authorization", format!("Bearer {dev_token}"))
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "catalog_info_yaml": catalog("orders", &["resource:default/Orders-DB", "component:billing", "cache"]),
            }))
            .unwrap(),
        ))
        .unwrap();
    let response = ctx.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        as_json(&String::from_utf8_lossy(&bytes)),
        json!({
            "system": "orders",
            "trunk": false,
            "resources": ["resource:default/cache", "resource:default/orders-db"],
        })
    );
}

#[tokio::test]
async fn an_unauthenticated_push_is_refused() {
    let ctx = setup().await;
    let request = Request::builder()
        .method("PUT")
        .uri("/catalog/orders")
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({"catalog_info_yaml": catalog("orders", &[])})).unwrap(),
        ))
        .unwrap();
    let response = ctx.app.clone().oneshot(request).await.unwrap();
    // Without a Bearer token a PUT is a browser-style request, and the CSRF
    // check refuses it before authentication is even consulted.
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_file_without_the_systems_component_is_a_400_and_records_nothing() {
    let ctx = setup().await;
    let (status, body) = push(&ctx, "orders", &catalog("billing", &["db"]), true).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body}");
    assert!(
        body.to_string().contains("no Component named 'orders'"),
        "got: {body}"
    );
    let rows: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM peripheral_dependencies")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    assert_eq!(rows.0, 0);
}

#[tokio::test]
async fn unknown_fields_are_rejected_by_name() {
    let ctx = setup().await;
    let (status, _, body) = send(
        &ctx,
        "PUT",
        "/catalog/orders",
        Some(json!({"catalog_info_yaml": catalog("orders", &[]), "tag": "rel-1"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "got: {body}");
}

#[tokio::test]
async fn a_trunk_push_writes_trunk_and_dev_and_a_dev_push_only_dev() {
    let ctx = setup().await;
    push(&ctx, "orders", &catalog("orders", &["orders-db"]), true).await;
    push(
        &ctx,
        "orders",
        &catalog("orders", &["orders-db", "scratch"]),
        false,
    )
    .await;
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT track, resource_ref FROM peripheral_dependencies WHERE valid_to IS NULL ORDER BY track, resource_ref",
    )
    .fetch_all(&ctx.pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            ("dev".to_string(), "resource:default/orders-db".to_string()),
            ("dev".to_string(), "resource:default/scratch".to_string()),
            (
                "trunk".to_string(),
                "resource:default/orders-db".to_string()
            ),
        ]
    );
}

#[cfg(test)]
async fn get_json(ctx: &TestContext, uri: &str) -> Value {
    let (status, _, body) = send(ctx, "GET", uri, None).await;
    assert_eq!(status, StatusCode::OK, "{uri} got: {body}");
    as_json(&body)
}

#[tokio::test]
async fn the_report_carries_the_periphery_of_its_scope() {
    let ctx = setup().await;
    push(&ctx, "orders", &catalog("orders", &["orders-db"]), true).await;
    push(
        &ctx,
        "billing",
        &catalog("billing", &["orders-db", "ledger"]),
        false,
    )
    .await;

    // Dev: all activity — the trunk push landed there too.
    let dev = get_json(&ctx, "/report").await;
    assert_eq!(
        dev["peripheral_dependencies"],
        json!([
            {"system": "billing", "resource": "resource:default/ledger"},
            {"system": "billing", "resource": "resource:default/orders-db"},
            {"system": "orders", "resource": "resource:default/orders-db"},
        ])
    );
    // Main: trunk only.
    let main = get_json(&ctx, "/report?scope=main").await;
    assert_eq!(
        main["peripheral_dependencies"],
        json!([{"system": "orders", "resource": "resource:default/orders-db"}])
    );
    // A past main carries the periphery as it was then — none yet.
    let past = get_json(&ctx, "/report?scope=main@2000-01-01T00:00:00Z").await;
    assert_eq!(past["peripheral_dependencies"], json!([]));
}

#[tokio::test]
async fn the_periphery_endpoint_answers_per_scope() {
    let ctx = setup().await;
    push(&ctx, "orders", &catalog("orders", &["orders-db"]), true).await;
    push(
        &ctx,
        "orders",
        &catalog("orders", &["orders-db", "scratch"]),
        false,
    )
    .await;

    assert_eq!(
        get_json(&ctx, "/admin/periphery").await,
        json!([
            {"system": "orders", "resource": "resource:default/orders-db"},
            {"system": "orders", "resource": "resource:default/scratch"},
        ])
    );
    assert_eq!(
        get_json(&ctx, "/admin/periphery?scope=main").await,
        json!([{"system": "orders", "resource": "resource:default/orders-db"}])
    );
    assert_eq!(
        get_json(&ctx, "/admin/periphery?scope=main@2000-01-01T00:00:00Z").await,
        json!([])
    );
    let (status, _, _) = send(&ctx, "GET", "/admin/periphery?scope=no-such-branch", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = send(&ctx, "GET", "/admin/periphery?scope=main@nope", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[cfg(test)]
async fn create_branch(ctx: &TestContext, body: Value) {
    let (status, _, text) = send(ctx, "POST", "/admin/branches", Some(body)).await;
    assert!(
        status.is_success(),
        "branch creation failed: {status} {text}"
    );
}

#[tokio::test]
async fn a_branch_cut_copies_the_trunk_periphery_and_keeps_it_frozen() {
    let ctx = setup().await;
    push(&ctx, "orders", &catalog("orders", &["orders-db"]), true).await;
    create_branch(&ctx, json!({"name": "rel-1"})).await;
    // Trunk moves on after the cut; the release keeps what it was cut with.
    push(&ctx, "orders", &catalog("orders", &["orders-db-v2"]), true).await;

    assert_eq!(
        get_json(&ctx, "/admin/periphery?scope=rel-1").await,
        json!([{"system": "orders", "resource": "resource:default/orders-db"}])
    );
    assert_eq!(
        get_json(&ctx, "/admin/periphery?scope=main").await,
        json!([{"system": "orders", "resource": "resource:default/orders-db-v2"}])
    );
    let report = get_json(&ctx, "/report?scope=rel-1").await;
    assert_eq!(
        report["peripheral_dependencies"],
        json!([{"system": "orders", "resource": "resource:default/orders-db"}])
    );

    // Branching off a branch copies the branch's periphery.
    create_branch(&ctx, json!({"name": "rel-1.1", "source": "rel-1"})).await;
    assert_eq!(
        get_json(&ctx, "/admin/periphery?scope=rel-1.1").await,
        json!([{"system": "orders", "resource": "resource:default/orders-db"}])
    );

    // The diff names the resource the release does not have yet.
    let diff = get_json(&ctx, "/admin/graph/diff?left=rel-1&right=main").await;
    assert_eq!(
        diff["periphery_added"],
        json!([{"system": "orders", "resource": "resource:default/orders-db-v2"}])
    );
    assert_eq!(
        diff["periphery_removed"],
        json!([{"system": "orders", "resource": "resource:default/orders-db"}])
    );
}

#[tokio::test]
async fn a_retroactive_cut_takes_the_periphery_as_it_was() {
    let ctx = setup().await;
    push(&ctx, "orders", &catalog("orders", &["orders-db"]), true).await;
    // Backdate the push so an instant before it exists without sleeping.
    sqlx::query("UPDATE peripheral_dependencies SET valid_from = '2026-01-01T00:00:00Z'")
        .execute(&ctx.pool)
        .await
        .unwrap();
    create_branch(
        &ctx,
        json!({"name": "old", "as_of": "2025-12-31T00:00:00Z"}),
    )
    .await;
    assert_eq!(
        get_json(&ctx, "/admin/periphery?scope=old").await,
        json!([])
    );
    create_branch(
        &ctx,
        json!({"name": "new", "as_of": "2026-01-02T00:00:00Z"}),
    )
    .await;
    assert_eq!(
        get_json(&ctx, "/admin/periphery?scope=new").await,
        json!([{"system": "orders", "resource": "resource:default/orders-db"}])
    );
}

#[tokio::test]
async fn periphery_changes_are_timeline_instants() {
    let ctx = setup().await;
    push(&ctx, "orders", &catalog("orders", &["orders-db"]), true).await;
    sqlx::query("UPDATE peripheral_dependencies SET valid_from = '2026-01-01T00:00:00Z'")
        .execute(&ctx.pool)
        .await
        .unwrap();
    let dates = get_json(&ctx, "/admin/trunk/timeline").await;
    assert_eq!(dates, json!(["2026-01-01T00:00:00Z"]));
}

#[tokio::test]
async fn deleting_a_branch_deletes_its_periphery_copy() {
    let ctx = setup().await;
    push(&ctx, "orders", &catalog("orders", &["orders-db"]), true).await;
    create_branch(&ctx, json!({"name": "rel-1"})).await;
    let (status, _, body) = send(&ctx, "DELETE", "/admin/branches/rel-1", None).await;
    assert!(status.is_success(), "got: {status} {body}");
    let rows: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM peripheral_dependencies WHERE track = 'branch'")
            .fetch_one(&ctx.pool)
            .await
            .unwrap();
    assert_eq!(rows.0, 0);
}

#[cfg(test)]
fn one_endpoint_spec() -> String {
    "openapi: 3.0.3\ninfo:\n  title: T\n  version: 1.0.0\npaths:\n  /users:\n    get:\n      responses:\n        '200':\n          description: OK\n".to_string()
}

#[tokio::test]
async fn deleting_the_last_role_of_a_system_closes_its_periphery() {
    let ctx = setup().await;
    let (status, _, body) = send(
        &ctx,
        "POST",
        "/provide",
        Some(json!({"producername": "users", "openapi_yaml": one_endpoint_spec(), "stability": "snapshot"})),
    )
    .await;
    assert!(status.is_success(), "provide failed: {status} {body}");
    for (consumer, producer) in [("webapp", "users"), ("users", "users")] {
        let (status, _, body) = send(
            &ctx,
            "GET",
            &format!("/require?consumername={consumer}&producername={producer}&version=1.0.0&path=/users&method=GET&trunk=true"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "require failed: {body}");
    }
    push(&ctx, "webapp", &catalog("webapp", &["sessions"]), true).await;
    push(&ctx, "users", &catalog("users", &["users-db"]), true).await;

    // `webapp` is only a Consumer: deleting it closes its periphery.
    let (status, _, body) = send(&ctx, "DELETE", "/admin/consumers/webapp", None).await;
    assert!(status.is_success(), "got: {status} {body}");
    // `users` is a Consumer and a Producer: dropping one role keeps it.
    let (status, _, body) = send(&ctx, "DELETE", "/admin/consumers/users", None).await;
    assert!(status.is_success(), "got: {status} {body}");

    let expected = json!([{"system": "users", "resource": "resource:default/users-db"}]);
    assert_eq!(get_json(&ctx, "/admin/periphery").await, expected);
    assert_eq!(
        get_json(&ctx, "/admin/periphery?scope=main").await,
        expected
    );
    // Closed, not deleted: the timeline keeps webapp's rows.
    let closed: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM peripheral_dependencies WHERE system_name = 'webapp' AND valid_to IS NOT NULL",
    )
    .fetch_one(&ctx.pool)
    .await
    .unwrap();
    assert_eq!(closed.0, 2);
}
