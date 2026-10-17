//! Webhooks (#47) at the router seam: which requests write which events to
//! the outbox, who they fan out to, and what a receiver sees when the
//! worker delivers them — asserted through the HTTP surface and a throwaway
//! receiver, never through the worker's internals.

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use chrono::Utc;
use sanshain_service::application::services;
use sanshain_service::domain::models::{
    WebhookDelivery, WebhookEventType, WebhookFormat, WebhookScope,
};
use sanshain_service::domain::ports::{NewWebhook, SpecRepository};
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

const TEST_CSRF_TOKEN: &str = "webhook-test-csrf-token";

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
    let mut tokens = HashMap::new();
    tokens.insert(
        TEST_CSRF_TOKEN.to_string(),
        Utc::now() + chrono::Duration::hours(1),
    );
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
    repo: SqliteSpecRepository,
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

    let app = create_app(test_app_state(repo.clone()));
    TestContext {
        app,
        repo,
        token: session.token,
    }
}

#[cfg(test)]
async fn send_as(
    ctx: &TestContext,
    token: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, HeaderMap, String) {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("Authorization", format!("Bearer {token}"))
        .header("X-CSRF-Token", TEST_CSRF_TOKEN);
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
async fn send(
    ctx: &TestContext,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, HeaderMap, String) {
    send_as(ctx, &ctx.token, method, uri, body).await
}

/// An OpenAPI document with one GET operation per path; `deprecated` paths
/// carry `deprecated: true`.
#[cfg(test)]
fn spec(version: &str, paths: &[&str], deprecated: &[&str]) -> String {
    let mut yaml = format!("openapi: 3.0.0\ninfo:\n  title: T\n  version: {version}\npaths:\n");
    for path in paths {
        yaml.push_str(&format!("  {path}:\n    get:\n"));
        if deprecated.contains(path) {
            yaml.push_str("      deprecated: true\n");
        }
        yaml.push_str("      responses:\n        \"200\":\n          description: ok\n");
    }
    yaml
}

#[cfg(test)]
async fn provide(
    ctx: &TestContext,
    producer: &str,
    stability: &str,
    yaml: &str,
    extra: Value,
) -> (StatusCode, Value) {
    let mut body = json!({"producername": producer, "openapi_yaml": yaml, "stability": stability});
    if let Value::Object(extra) = extra {
        for (k, v) in extra {
            body[k] = v;
        }
    }
    let (status, _, text) = send(ctx, "POST", "/provide", Some(body)).await;
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

#[cfg(test)]
async fn require(
    ctx: &TestContext,
    consumer: &str,
    producer: &str,
    version: &str,
    path: &str,
    extra: &str,
) -> StatusCode {
    let uri = format!(
        "/require?consumername={consumer}&producername={producer}&version={version}&path={path}&method=GET{extra}"
    );
    send(ctx, "GET", &uri, None).await.0
}

#[cfg(test)]
async fn add_hook(ctx: &TestContext, scope: WebhookScope, url: &str) -> i64 {
    add_hook_with(ctx, scope, url, &[], None).await
}

#[cfg(test)]
async fn add_hook_with(
    ctx: &TestContext,
    scope: WebhookScope,
    url: &str,
    event_types: &[WebhookEventType],
    producer_glob: Option<&str>,
) -> i64 {
    ctx.repo
        .create_webhook(NewWebhook {
            scope: &scope,
            url,
            format: WebhookFormat::Json,
            event_types,
            producer_glob,
            secret: "whsec_test",
            secret_hint: "deadbeef",
            allow_list_exempt: true,
            created_by: "root",
            created_at: "2026-01-01T00:00:00Z",
        })
        .await
        .unwrap()
}

#[cfg(test)]
async fn deliveries(ctx: &TestContext, hook_id: i64) -> Vec<WebhookDelivery> {
    ctx.repo
        .list_webhook_deliveries(hook_id, 100)
        .await
        .unwrap()
}

#[cfg(test)]
fn event_types(rows: &[WebhookDelivery]) -> Vec<WebhookEventType> {
    rows.iter().map(|d| d.event_type).collect()
}

/// The stored `data` of a delivery's event, read through the claim so the
/// test never touches the schema.
#[cfg(test)]
async fn event_data(ctx: &TestContext, delivery: &WebhookDelivery) -> Value {
    let claimed = ctx
        .repo
        .claim_due_webhook_deliveries("2999-01-01T00:00:00Z", "2999-01-01T00:00:00Z", 1000)
        .await
        .unwrap();
    claimed
        .into_iter()
        .find(|d| d.delivery.id == delivery.id)
        .map(|d| d.data)
        .unwrap_or_else(|| panic!("delivery {} is not claimable", delivery.id))
}

fn global() -> WebhookScope {
    WebhookScope::Global
}

fn producer(name: &str) -> WebhookScope {
    WebhookScope::Producer {
        name: name.to_string(),
    }
}

fn consumer(name: &str) -> WebhookScope {
    WebhookScope::Consumer {
        name: name.to_string(),
    }
}

// --- Events, per type ---

#[tokio::test]
async fn a_provide_emits_version_published_and_a_dry_run_or_no_op_does_not() {
    let ctx = setup().await;
    let hook = add_hook(&ctx, producer("orders"), "https://hooks.example/orders").await;

    let (status, _) = provide(
        &ctx,
        "orders",
        "snapshot",
        &spec("1.0.0", &["/a"], &[]),
        json!({"dry_run": true}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(
        deliveries(&ctx, hook).await.is_empty(),
        "a dry-run emits nothing"
    );

    let (status, _) = provide(
        &ctx,
        "orders",
        "ga",
        &spec("1.0.0", &["/a"], &[]),
        json!({"trunk": true}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "first GA provide");
    let rows = deliveries(&ctx, hook).await;
    assert_eq!(event_types(&rows), vec![WebhookEventType::VersionPublished]);
    let data = event_data(&ctx, &rows[0]).await;
    assert_eq!(
        data,
        json!({
            "producer": "orders", "api_type": "openapi", "version": "1.0.0", "stability": "ga",
            "promoted": false, "stream": "trunk", "actor": "root", "compatibility": null,
        })
    );

    // Byte-identical re-provide: the idempotent no-op publishes nothing new.
    let (status, _) = provide(
        &ctx,
        "orders",
        "ga",
        &spec("1.0.0", &["/a"], &[]),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(deliveries(&ctx, hook).await.len(), 1);

    // The next GA carries its classification against the previous one.
    let (status, _) = provide(
        &ctx,
        "orders",
        "ga",
        &spec("1.1.0", &["/a", "/b"], &[]),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let rows = deliveries(&ctx, hook).await;
    assert_eq!(rows.len(), 2);
    let data = event_data(&ctx, &rows[0]).await;
    assert_eq!(data["version"], "1.1.0");
    assert_eq!(data["stream"], "dev");
    assert_eq!(
        data["compatibility"],
        json!({"against": "1.0.0", "impact": "minor"})
    );

    // A promotion says so.
    let (status, _) = provide(
        &ctx,
        "orders",
        "snapshot",
        &spec("1.2.0", &["/a", "/b"], &[]),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let (status, _) = provide(
        &ctx,
        "orders",
        "ga",
        &spec("1.2.0", &["/a", "/b"], &[]),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let rows = deliveries(&ctx, hook).await;
    assert_eq!(rows.len(), 4);
    let data = event_data(&ctx, &rows[0]).await;
    assert_eq!(data["promoted"], true);
    assert_eq!(data["stability"], "ga");
}

#[tokio::test]
async fn a_rejected_provide_emits_provide_rejected_with_rule_and_proposal() {
    let ctx = setup().await;
    let hook = add_hook(&ctx, producer("orders"), "https://hooks.example/orders").await;
    provide(
        &ctx,
        "orders",
        "ga",
        &spec("1.0.0", &["/a"], &[]),
        json!({}),
    )
    .await;

    // Same number, different content: the GA is immutable.
    let (status, body) = provide(
        &ctx,
        "orders",
        "ga",
        &spec("1.0.0", &["/a", "/b"], &[]),
        json!({"dry_run": true}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        deliveries(&ctx, hook).await.len(),
        1,
        "a dry-run rejection emits nothing"
    );

    let (status, body) = provide(
        &ctx,
        "orders",
        "ga",
        &spec("1.0.0", &["/a", "/b"], &[]),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let rows = deliveries(&ctx, hook).await;
    assert_eq!(
        event_types(&rows),
        vec![
            WebhookEventType::ProvideRejected,
            WebhookEventType::VersionPublished
        ]
    );
    let data = event_data(&ctx, &rows[0]).await;
    assert_eq!(data["producer"], "orders");
    assert_eq!(data["api_type"], "openapi");
    assert_eq!(data["version"], "1.0.0");
    assert_eq!(data["rule"], "immutable_ga");
    assert_eq!(data["proposed_version"], "1.1.0");
    assert_eq!(data["actor"], "root");
    assert!(data["message"].as_str().unwrap().contains("immutable"));

    // Breaking without a major bump.
    let (status, _) = provide(
        &ctx,
        "orders",
        "ga",
        &spec("1.1.0", &["/b"], &[]),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let rows = deliveries(&ctx, hook).await;
    let data = event_data(&ctx, &rows[0]).await;
    assert_eq!(data["rule"], "breaking_without_major");
    assert_eq!(data["proposed_version"], "2.0.0");
}

#[tokio::test]
async fn a_provide_that_newly_deprecates_an_element_emits_version_deprecated() {
    let ctx = setup().await;
    let hook = add_hook(&ctx, producer("orders"), "https://hooks.example/orders").await;
    provide(
        &ctx,
        "orders",
        "ga",
        &spec("1.0.0", &["/a", "/b"], &["/b"]),
        json!({}),
    )
    .await;
    let rows = deliveries(&ctx, hook).await;
    assert_eq!(
        event_types(&rows),
        vec![
            WebhookEventType::VersionDeprecated,
            WebhookEventType::VersionPublished
        ],
        "with no previous GA, every deprecated element is new"
    );
    let data = event_data(&ctx, &rows[0]).await;
    assert_eq!(
        data,
        json!({
            "producer": "orders", "api_type": "openapi", "version": "1.0.0", "stability": "ga",
            "elements": [{"path": "/b", "method": "GET"}], "document_deprecated": false,
        })
    );

    // /b was already deprecated in GA 1.0.0 — no event for it; /a is new.
    provide(
        &ctx,
        "orders",
        "snapshot",
        &spec("1.1.0", &["/a", "/b"], &["/a", "/b"]),
        json!({}),
    )
    .await;
    let rows = deliveries(&ctx, hook).await;
    assert_eq!(rows.len(), 4);
    let data = event_data(&ctx, &rows[0]).await;
    assert_eq!(data["version"], "1.1.0");
    assert_eq!(data["stability"], "snapshot");
    assert_eq!(data["elements"], json!([{"path": "/a", "method": "GET"}]));
    assert_eq!(
        data["document_deprecated"], true,
        "every element is deprecated"
    );

    // Nothing newly deprecated: only version.published.
    provide(
        &ctx,
        "orders",
        "snapshot",
        &spec("1.2.0", &["/a", "/b"], &["/b"]),
        json!({}),
    )
    .await;
    let rows = deliveries(&ctx, hook).await;
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0].event_type, WebhookEventType::VersionPublished);
}

#[tokio::test]
async fn a_require_emits_dependency_recorded_for_a_new_edge_only() {
    let ctx = setup().await;
    let hook = add_hook(&ctx, producer("orders"), "https://hooks.example/orders").await;
    provide(
        &ctx,
        "orders",
        "snapshot",
        &spec("1.0.0", &["/a", "/b"], &[]),
        json!({}),
    )
    .await;
    assert_eq!(deliveries(&ctx, hook).await.len(), 1);

    assert_eq!(
        require(&ctx, "web", "orders", "1.0.0", "/a", "&dry_run=true").await,
        StatusCode::OK
    );
    assert_eq!(
        deliveries(&ctx, hook).await.len(),
        1,
        "a dry-run records no edge"
    );

    assert_eq!(
        require(&ctx, "web", "orders", "1.0.0", "/a", "").await,
        StatusCode::OK
    );
    let rows = deliveries(&ctx, hook).await;
    assert_eq!(rows.len(), 2);
    let data = event_data(&ctx, &rows[0]).await;
    assert_eq!(
        data,
        json!({
            "consumer": "web", "producer": "orders", "api_type": "openapi", "version": "1.0.0",
            "stability": "snapshot", "snapshot_pinned": true, "stream": "dev",
            "endpoints": [{"path": "/a", "method": "GET"}],
        })
    );

    assert_eq!(
        require(&ctx, "web", "orders", "1.0.0", "/a", "").await,
        StatusCode::OK
    );
    assert_eq!(
        deliveries(&ctx, hook).await.len(),
        2,
        "re-recording an edge emits nothing"
    );

    // The same endpoint in the trunk stream is a different edge.
    assert_eq!(
        require(&ctx, "web", "orders", "1.0.0", "/a", "&trunk=true").await,
        StatusCode::OK
    );
    let rows = deliveries(&ctx, hook).await;
    assert_eq!(rows.len(), 3);
    assert_eq!(event_data(&ctx, &rows[0]).await["stream"], "trunk");
    assert_eq!(
        require(&ctx, "web", "orders", "1.0.0", "/a", "&trunk=true").await,
        StatusCode::OK
    );
    assert_eq!(deliveries(&ctx, hook).await.len(), 3);

    // A bundle carries only the endpoints new to the edge.
    let (status, _, _) = send(
        &ctx,
        "POST",
        "/require-bundle",
        Some(json!({
            "consumername": "web", "producername": "orders", "version": "1.0.0",
            "endpoints": [{"path": "/a", "method": "GET"}, {"path": "/b", "method": "GET"}],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = deliveries(&ctx, hook).await;
    assert_eq!(rows.len(), 4);
    assert_eq!(
        event_data(&ctx, &rows[0]).await["endpoints"],
        json!([{"path": "/b", "method": "GET"}])
    );
}

#[tokio::test]
async fn a_branch_cut_reaches_global_hooks_only() {
    let ctx = setup().await;
    let global_hook = add_hook(&ctx, global(), "https://hooks.example/ops").await;
    let producer_hook = add_hook(&ctx, producer("orders"), "https://hooks.example/orders").await;
    let (status, _, body) = send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "Release Maribou", "as_of": "2026-01-01T00:00:00Z"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let rows = deliveries(&ctx, global_hook).await;
    assert_eq!(event_types(&rows), vec![WebhookEventType::BranchCut]);
    let data = event_data(&ctx, &rows[0]).await;
    assert_eq!(data["name"], "Release Maribou");
    assert_eq!(data["source"], "trunk");
    assert_eq!(data["as_of"], "2026-01-01T00:00:00Z");
    assert_eq!(data["actor"], "root");
    assert!(data["cut_at"].as_str().unwrap().ends_with('Z'));
    assert!(deliveries(&ctx, producer_hook).await.is_empty());
}

#[tokio::test]
async fn a_catalog_push_emits_catalog_recorded_either_way() {
    let ctx = setup().await;
    let hook = add_hook(&ctx, producer("orders"), "https://hooks.example/orders").await;
    let as_consumer = add_hook(&ctx, consumer("orders"), "https://hooks.example/orders-c").await;
    let other = add_hook(&ctx, producer("billing"), "https://hooks.example/billing").await;
    let good = "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: orders\nspec:\n  dependsOn:\n    - resource:default/orders-db\n    - resource:default/cache\n";
    let (status, _, _) = send(
        &ctx,
        "PUT",
        "/catalog/orders",
        Some(json!({"catalog_info_yaml": good, "trunk": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = deliveries(&ctx, hook).await;
    assert_eq!(event_types(&rows), vec![WebhookEventType::CatalogRecorded]);
    assert_eq!(
        event_data(&ctx, &rows[0]).await,
        json!({"system": "orders", "trunk": true, "recorded": true, "warning": null, "resources": 2})
    );
    assert_eq!(
        deliveries(&ctx, as_consumer).await.len(),
        1,
        "the system's Consumer hooks too"
    );
    assert!(deliveries(&ctx, other).await.is_empty());

    let (status, _, _) = send(
        &ctx,
        "PUT",
        "/catalog/orders",
        Some(json!({"catalog_info_yaml": "key: [unclosed"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rows = deliveries(&ctx, hook).await;
    assert_eq!(rows.len(), 2);
    let data = event_data(&ctx, &rows[0]).await;
    assert_eq!(data["recorded"], false);
    assert_eq!(data["trunk"], false);
    assert_eq!(data["resources"], 0);
    assert!(
        data["warning"].as_str().unwrap().contains("not valid YAML"),
        "{data}"
    );
}

// --- Fan-out ---

#[tokio::test]
async fn a_producers_event_reaches_its_hooks_its_current_consumers_and_global_hooks() {
    let ctx = setup().await;
    provide(
        &ctx,
        "orders",
        "ga",
        &spec("1.0.0", &["/a"], &[]),
        json!({}),
    )
    .await;
    provide(
        &ctx,
        "billing",
        "ga",
        &spec("1.0.0", &["/b"], &[]),
        json!({}),
    )
    .await;
    // cli required orders on trunk once; both its trunk pin and its dev-graph
    // edge have since been closed by the TTL cleanups.
    assert_eq!(
        require(&ctx, "cli", "orders", "1.0.0", "/a", "&trunk=true").await,
        StatusCode::OK
    );
    ctx.repo
        .close_expired_trunk_data("2999-01-01T00:00:00Z", "2999-01-01T00:00:00Z")
        .await
        .unwrap();
    ctx.repo
        .delete_stale_dependencies("2999-01-01T00:00:00Z")
        .await
        .unwrap();
    // web requires orders (dev); far requires billing.
    assert_eq!(
        require(&ctx, "web", "orders", "1.0.0", "/a", "").await,
        StatusCode::OK
    );
    assert_eq!(
        require(&ctx, "far", "billing", "1.0.0", "/b", "").await,
        StatusCode::OK
    );

    let own = add_hook(&ctx, producer("orders"), "https://hooks.example/orders").await;
    let web = add_hook(&ctx, consumer("web"), "https://hooks.example/web").await;
    let cli = add_hook(&ctx, consumer("cli"), "https://hooks.example/cli").await;
    let far = add_hook(&ctx, consumer("far"), "https://hooks.example/far").await;
    let billing = add_hook(&ctx, producer("billing"), "https://hooks.example/billing").await;
    let ops = add_hook(&ctx, global(), "https://hooks.example/ops").await;

    provide(
        &ctx,
        "orders",
        "ga",
        &spec("1.1.0", &["/a"], &[]),
        json!({}),
    )
    .await;
    assert_eq!(
        deliveries(&ctx, own).await.len(),
        1,
        "the Producer's own hook"
    );
    assert_eq!(
        deliveries(&ctx, web).await.len(),
        1,
        "a current Consumer's hook"
    );
    assert_eq!(deliveries(&ctx, ops).await.len(), 1, "a global hook");
    assert!(
        deliveries(&ctx, cli).await.is_empty(),
        "a Consumer whose edge is closed"
    );
    assert!(
        deliveries(&ctx, far).await.is_empty(),
        "a Consumer of another Producer"
    );
    assert!(
        deliveries(&ctx, billing).await.is_empty(),
        "another Producer's hook"
    );
}

#[tokio::test]
async fn global_hooks_filter_by_event_type_and_producer_glob() {
    let ctx = setup().await;
    let rejections = add_hook_with(
        &ctx,
        global(),
        "https://hooks.example/rejections",
        &[WebhookEventType::ProvideRejected],
        None,
    )
    .await;
    let payments = add_hook_with(
        &ctx,
        global(),
        "https://hooks.example/payments",
        &[],
        Some("pay*"),
    )
    .await;
    let everything = add_hook(&ctx, global(), "https://hooks.example/all").await;

    provide(
        &ctx,
        "orders",
        "ga",
        &spec("1.0.0", &["/a"], &[]),
        json!({}),
    )
    .await;
    provide(
        &ctx,
        "payments",
        "ga",
        &spec("1.0.0", &["/p"], &[]),
        json!({}),
    )
    .await;
    provide(
        &ctx,
        "orders",
        "ga",
        &spec("1.0.0", &["/a", "/x"], &[]),
        json!({}),
    )
    .await;
    send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "rel"})),
    )
    .await;

    assert_eq!(
        event_types(&deliveries(&ctx, rejections).await),
        vec![WebhookEventType::ProvideRejected]
    );
    let rows = deliveries(&ctx, payments).await;
    assert_eq!(event_types(&rows), vec![WebhookEventType::VersionPublished]);
    assert_eq!(event_data(&ctx, &rows[0]).await["producer"], "payments");
    assert_eq!(
        event_types(&deliveries(&ctx, everything).await),
        vec![
            WebhookEventType::BranchCut,
            WebhookEventType::ProvideRejected,
            WebhookEventType::VersionPublished,
            WebhookEventType::VersionPublished,
        ]
    );
}
