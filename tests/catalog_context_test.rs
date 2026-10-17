//! Owner, description, source location and links from a system's Backstage
//! catalog-info.yaml (#48) at the router seam: a push records the system's
//! context, and the Producer, Consumer, report and graph scope endpoints
//! answer it — without touching repository internals.

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
async fn get_json(ctx: &TestContext, uri: &str) -> Value {
    let (status, _, body) = send(ctx, "GET", uri, None).await;
    assert_eq!(status, StatusCode::OK, "{uri} got: {body}");
    as_json(&body)
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

/// A Component with the fields #48 reads: `metadata` lines and `spec` lines
/// are spliced in verbatim (already indented by two spaces).
#[cfg(test)]
fn catalog_with(name: &str, metadata: &str, spec: &str) -> String {
    format!(
        "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: {name}\n{metadata}spec:\n  type: service\n{spec}"
    )
}

#[cfg(test)]
const FULL_METADATA: &str = r#"  description: "  Takes orders and hands them to billing.  "
  annotations:
    backstage.io/source-location: url:https://github.com/acme/orders/tree/main/
    other.io/ignored: whatever
  links:
    - url: https://wiki.acme.corp/orders
      title: Wiki
      type: docs
      icon: docs
    - url: https://grafana.acme.corp/d/orders
      title: Dashboard
    - url: https://jira.acme.corp/browse/ORD
"#;

#[cfg(test)]
fn expected_full_context() -> Value {
    json!({
        "owner": {"kind": "group", "name": "team-orders"},
        "description": "Takes orders and hands them to billing.",
        "source_location": "https://github.com/acme/orders/tree/main/",
        "links": [
            {"url": "https://wiki.acme.corp/orders", "title": "Wiki", "type": "docs", "icon": "docs"},
            {"url": "https://grafana.acme.corp/d/orders", "title": "Dashboard", "type": null, "icon": null},
            {"url": "https://jira.acme.corp/browse/ORD", "title": "jira.acme.corp", "type": null, "icon": null},
        ],
    })
}

#[cfg(test)]
fn one_endpoint_spec() -> String {
    "openapi: 3.0.3\ninfo:\n  title: T\n  version: 1.0.0\npaths:\n  /users:\n    get:\n      responses:\n        '200':\n          description: OK\n".to_string()
}

/// `orders` provides and `webapp` requires it: one Producer, one Consumer.
#[cfg(test)]
async fn seed_roles(ctx: &TestContext) {
    let (status, _, body) = send(
        ctx,
        "POST",
        "/provide",
        Some(json!({"producername": "orders", "openapi_yaml": one_endpoint_spec(), "stability": "snapshot"})),
    )
    .await;
    assert!(status.is_success(), "provide failed: {status} {body}");
    let (status, _, body) = send(
        ctx,
        "GET",
        "/require?consumername=webapp&producername=orders&version=1.0.0&path=/users&method=GET&trunk=true",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "require failed: {body}");
}

#[tokio::test]
async fn a_push_makes_the_context_visible_on_every_endpoint() {
    let ctx = setup().await;
    seed_roles(&ctx).await;
    let spec = "  owner: team-orders\n  dependsOn:\n    - orders-db\n";
    let (status, body) = push(
        &ctx,
        "orders",
        &catalog_with("orders", FULL_METADATA, spec),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["recorded"], true);
    push(
        &ctx,
        "webapp",
        &catalog_with("webapp", FULL_METADATA, "  owner: user:default/Alice\n"),
        false,
    )
    .await;

    let producer = get_json(&ctx, "/admin/producers/orders").await;
    assert_eq!(
        producer,
        json!({"name": "orders", "context": expected_full_context()})
    );
    let consumer = get_json(&ctx, "/admin/consumers/webapp").await;
    assert_eq!(consumer["name"], "webapp");
    assert_eq!(
        consumer["context"]["owner"],
        json!({"kind": "user", "name": "alice"})
    );
    assert_eq!(consumer["context"]["links"].as_array().unwrap().len(), 3);

    let report = get_json(&ctx, "/report").await;
    assert_eq!(report["systems"]["orders"], expected_full_context());
    assert_eq!(report["systems"]["webapp"]["owner"]["name"], "alice");
    // The context is a present-day fact, not a graph fact: every scope carries it.
    let main = get_json(&ctx, "/report?scope=main").await;
    assert_eq!(main["systems"]["orders"], expected_full_context());

    let scope = get_json(&ctx, "/admin/graph/scope").await;
    assert_eq!(scope["systems"]["orders"], expected_full_context());
    let scope_main = get_json(&ctx, "/admin/graph/scope?scope=main").await;
    assert_eq!(scope_main["systems"]["webapp"]["owner"]["kind"], "user");
}

#[tokio::test]
async fn a_system_without_a_push_has_no_context_and_an_unknown_one_is_404() {
    let ctx = setup().await;
    seed_roles(&ctx).await;
    assert_eq!(
        get_json(&ctx, "/admin/producers/orders").await,
        json!({"name": "orders", "context": null})
    );
    assert_eq!(
        get_json(&ctx, "/admin/consumers/webapp").await,
        json!({"name": "webapp", "context": null})
    );
    assert_eq!(get_json(&ctx, "/report").await["systems"], json!({}));
    // webapp only consumes; orders only produces.
    let (status, _, _) = send(&ctx, "GET", "/admin/producers/webapp", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = send(&ctx, "GET", "/admin/consumers/orders", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[cfg(test)]
async fn audit_count(ctx: &TestContext) -> i64 {
    let audited: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM audit_logs WHERE action = 'CATALOG_PUSHED'")
            .fetch_one(&ctx.pool)
            .await
            .unwrap();
    audited.0
}

#[tokio::test]
async fn a_second_push_replaces_the_context_and_an_identical_one_changes_nothing() {
    let ctx = setup().await;
    seed_roles(&ctx).await;
    let full = catalog_with(
        "orders",
        FULL_METADATA,
        "  owner: team-orders\n  dependsOn: [orders-db]\n",
    );
    push(&ctx, "orders", &full, true).await;
    assert_eq!(audit_count(&ctx).await, 1);
    let timeline_before = get_json(&ctx, "/admin/trunk/timeline").await;

    // Identical content: no audit entry, no timeline instant, same answer.
    push(&ctx, "orders", &full, true).await;
    assert_eq!(audit_count(&ctx).await, 1);
    assert_eq!(
        get_json(&ctx, "/admin/trunk/timeline").await,
        timeline_before
    );
    assert_eq!(
        get_json(&ctx, "/admin/producers/orders").await["context"],
        expected_full_context()
    );

    // Without the fields: cleared, not kept — the push replaces the context.
    let bare = catalog_with("orders", "", "  dependsOn: [orders-db]\n");
    push(&ctx, "orders", &bare, true).await;
    assert_eq!(audit_count(&ctx).await, 2);
    assert_eq!(
        get_json(&ctx, "/admin/producers/orders").await["context"],
        json!({"owner": null, "description": null, "source_location": null, "links": []})
    );
}

#[tokio::test]
async fn an_empty_depends_on_still_records_the_context() {
    let ctx = setup().await;
    seed_roles(&ctx).await;
    let (status, body) = push(
        &ctx,
        "orders",
        &catalog_with(
            "orders",
            "  description: Orders.\n",
            "  owner: group:shop/team-orders\n",
        ),
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["resources"], json!([]));
    assert_eq!(body["recorded"], true);
    let context = get_json(&ctx, "/admin/producers/orders").await["context"].clone();
    assert_eq!(
        context["owner"],
        json!({"kind": "group", "name": "team-orders"})
    );
    assert_eq!(context["description"], "Orders.");
    assert_eq!(get_json(&ctx, "/admin/periphery").await, json!([]));
}

#[tokio::test]
async fn fields_are_normalised_validated_and_bounded() {
    let ctx = setup().await;
    seed_roles(&ctx).await;
    let long_description = "x".repeat(2000);
    let links: String = (1..=21)
        .map(|i| format!("    - url: https://link{i}.example\n      title: Link {i}\n"))
        .collect();
    let metadata = format!(
        "  description: {long_description}\n  annotations:\n    backstage.io/source-location: url:https://git.example/orders\n  links:\n    - url: javascript:alert(1)\n      title: Bad\n    - url: ftp://files.example/x\n    - title: no url at all\n{links}"
    );
    push(
        &ctx,
        "orders",
        &catalog_with("orders", &metadata, "  owner: Team-Orders\n"),
        false,
    )
    .await;
    let context = get_json(&ctx, "/admin/producers/orders").await["context"].clone();
    assert_eq!(
        context["owner"],
        json!({"kind": "group", "name": "team-orders"})
    );
    assert_eq!(context["source_location"], "https://git.example/orders");
    let description = context["description"].as_str().unwrap();
    assert_eq!(description.chars().count(), 1000);
    assert!(description.ends_with('…'), "{description}");
    let links = context["links"].as_array().unwrap();
    assert_eq!(links.len(), 20, "{links:?}");
    assert_eq!(links[0]["url"], "https://link1.example");
    assert_eq!(links[19]["url"], "https://link20.example");
    assert!(
        links
            .iter()
            .all(|l| l["url"].as_str().unwrap().starts_with("https://"))
    );

    // A source location that is no http(s) URL is dropped, as is a malformed owner.
    push(
        &ctx,
        "orders",
        &catalog_with(
            "orders",
            "  annotations:\n    backstage.io/source-location: file:///srv/orders\n",
            "  owner: 'not a ref'\n",
        ),
        false,
    )
    .await;
    let context = get_json(&ctx, "/admin/producers/orders").await["context"].clone();
    assert_eq!(
        context,
        json!({"owner": null, "description": null, "source_location": null, "links": []})
    );
}

#[tokio::test]
async fn a_file_that_is_not_the_systems_component_records_no_context() {
    let ctx = setup().await;
    seed_roles(&ctx).await;
    let (status, body) = push(
        &ctx,
        "orders",
        &catalog_with("billing", FULL_METADATA, "  owner: team-orders\n"),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["recorded"], false);
    assert_eq!(
        get_json(&ctx, "/admin/producers/orders").await["context"],
        Value::Null
    );
    assert_eq!(get_json(&ctx, "/report").await["systems"], json!({}));
    assert_eq!(get_json(&ctx, "/admin/periphery").await, json!([]));
}

#[tokio::test]
async fn deleting_the_last_role_of_a_system_removes_its_context() {
    let ctx = setup().await;
    seed_roles(&ctx).await;
    // `orders` requires itself too, so it is a Producer and a Consumer.
    let (status, _, body) = send(
        &ctx,
        "GET",
        "/require?consumername=orders&producername=orders&version=1.0.0&path=/users&method=GET",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "require failed: {body}");
    for system in ["orders", "webapp"] {
        push(
            &ctx,
            system,
            &catalog_with(system, FULL_METADATA, "  owner: team-orders\n"),
            true,
        )
        .await;
    }

    // `webapp` is only a Consumer: its context goes with it.
    let (status, _, body) = send(&ctx, "DELETE", "/admin/consumers/webapp", None).await;
    assert!(status.is_success(), "got: {status} {body}");
    // `orders` keeps its Producer role, so its context stays.
    let (status, _, body) = send(&ctx, "DELETE", "/admin/consumers/orders", None).await;
    assert!(status.is_success(), "got: {status} {body}");

    let systems = get_json(&ctx, "/report").await["systems"].clone();
    assert_eq!(
        systems.as_object().unwrap().keys().collect::<Vec<_>>(),
        vec!["orders"]
    );
    let rows: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM system_context")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    assert_eq!(rows.0, 1);
}
