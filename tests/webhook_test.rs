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

// --- Delivery: what a receiver sees ---

use axum::extract::State;
use axum::routing::post;
use hmac::{Hmac, KeyInit, Mac};
use sanshain_service::application::webhook_delivery::{WebhookDeliveryConfig, deliver_due};
use sanshain_service::domain::models::{DeliveryPassSummary as PassSummary, WebhookDeliveryStatus};
use sanshain_service::infrastructure::webhook_sender::ReqwestWebhookSender;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU16, Ordering};

/// What the receiver records per request: the headers and the raw body.
type Recorded = Arc<Mutex<Vec<(HeaderMap, String)>>>;

/// A throwaway receiver on an ephemeral port: records every request's
/// headers and body, answers the configured status.
#[cfg(test)]
struct Receiver {
    url: String,
    requests: Recorded,
    status: Arc<AtomicU16>,
}

#[cfg(test)]
impl Receiver {
    async fn start() -> Receiver {
        let requests: Recorded = Arc::new(Mutex::new(Vec::new()));
        let status = Arc::new(AtomicU16::new(200));
        async fn record(
            State((requests, status)): State<(Recorded, Arc<AtomicU16>)>,
            headers: HeaderMap,
            body: String,
        ) -> StatusCode {
            requests.lock().unwrap().push((headers, body));
            StatusCode::from_u16(status.load(Ordering::SeqCst)).unwrap()
        }
        let app = axum::Router::new()
            .route("/hook", post(record))
            .with_state((requests.clone(), status.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/hook", listener.local_addr().unwrap());
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Receiver {
            url,
            requests,
            status,
        }
    }

    fn answer_with(&self, status: u16) {
        self.status.store(status, Ordering::SeqCst);
    }

    fn requests(&self) -> Vec<(HeaderMap, String)> {
        self.requests.lock().unwrap().clone()
    }

    fn last(&self) -> (HeaderMap, String) {
        self.requests()
            .last()
            .cloned()
            .expect("a request was received")
    }
}

#[cfg(test)]
fn header<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .unwrap_or_else(|| panic!("no {name} header"))
        .to_str()
        .unwrap()
}

/// Independent verification of `Sanshain-Signature`: HMAC-SHA256 of
/// `<t>.<body>` under the secret, hex, as a receiver would compute it.
#[cfg(test)]
fn verify_signature(headers: &HeaderMap, body: &str, secret: &str) -> i64 {
    let value = header(headers, "Sanshain-Signature");
    let (t, v1) = value.split_once(',').unwrap();
    let t: i64 = t.strip_prefix("t=").unwrap().parse().unwrap();
    let v1 = v1.strip_prefix("v1=").unwrap();
    let mut mac = <Hmac<sha2::Sha256> as KeyInit>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("{t}.{body}").as_bytes());
    assert_eq!(
        hex::encode(mac.finalize().into_bytes()),
        v1,
        "signature mismatch"
    );
    t
}

#[cfg(test)]
fn config(base_url: Option<&str>) -> WebhookDeliveryConfig {
    WebhookDeliveryConfig {
        public_base_url: base_url.map(str::to_string),
        ..Default::default()
    }
}

/// One worker pass at `now` — the single-pass function the worker loops over.
#[cfg(test)]
async fn pass(
    ctx: &TestContext,
    config: &WebhookDeliveryConfig,
    now: chrono::DateTime<Utc>,
) -> PassSummary {
    let sender = ReqwestWebhookSender::new(std::time::Duration::from_secs(5)).unwrap();
    deliver_due(&ctx.repo, &sender, config, now).await.unwrap()
}

#[cfg(test)]
async fn add_format_hook(
    ctx: &TestContext,
    scope: WebhookScope,
    url: &str,
    format: WebhookFormat,
    exempt: bool,
) -> i64 {
    ctx.repo
        .create_webhook(NewWebhook {
            scope: &scope,
            url,
            format,
            event_types: &[],
            producer_glob: None,
            secret: "whsec_test",
            secret_hint: "deadbeef",
            allow_list_exempt: exempt,
            created_by: "root",
            created_at: "2026-01-01T00:00:00Z",
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn the_json_format_posts_the_signed_envelope_with_event_and_delivery_headers() {
    let ctx = setup().await;
    let receiver = Receiver::start().await;
    let hook = add_format_hook(
        &ctx,
        producer("orders"),
        &receiver.url,
        WebhookFormat::Json,
        true,
    )
    .await;
    provide(
        &ctx,
        "orders",
        "ga",
        &spec("1.0.0", &["/a"], &[]),
        json!({}),
    )
    .await;

    let now = Utc::now();
    let summary = pass(&ctx, &config(Some("https://sanshain.example/")), now).await;
    assert_eq!(
        summary,
        PassSummary {
            delivered: 1,
            failed: 0,
            dead: 0
        }
    );

    let (headers, body) = receiver.last();
    assert_eq!(header(&headers, "content-type"), "application/json");
    assert_eq!(header(&headers, "sanshain-event"), "version.published");
    assert!(header(&headers, "user-agent").starts_with("Sanshain/"));
    let rows = deliveries(&ctx, hook).await;
    assert_eq!(
        header(&headers, "x-sanshain-delivery"),
        rows[0].id.to_string()
    );
    let t = verify_signature(&headers, &body, "whsec_test");
    assert_eq!(t, now.timestamp());

    let envelope: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(envelope["id"], rows[0].event_id);
    assert_eq!(envelope["type"], "version.published");
    assert_eq!(envelope["occurred_at"], rows[0].occurred_at);
    assert_eq!(envelope["sanshain_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(envelope["data"]["producer"], "orders");
    assert_eq!(envelope["data"]["version"], "1.0.0");
    assert_eq!(
        envelope["links"],
        json!({
            "producer": "https://sanshain.example/producers.html?service=orders",
            "version": "https://sanshain.example/producers.html?service=orders&api_type=openapi&version=1.0.0",
            "graph": "https://sanshain.example/graph.html",
        })
    );
    assert_eq!(
        envelope.as_object().unwrap().len(),
        6,
        "no undocumented envelope fields"
    );

    assert_eq!(rows[0].status, WebhookDeliveryStatus::Delivered);
    assert_eq!(rows[0].attempts, 1);
    assert_eq!(rows[0].last_response_code, Some(200));
    assert!(rows[0].delivered_at.is_some());
    assert_eq!(
        pass(&ctx, &config(None), now).await,
        PassSummary::default(),
        "nothing left"
    );
    assert_eq!(receiver.requests().len(), 1);
}

#[tokio::test]
async fn links_fall_back_to_the_oidc_redirect_origin_or_are_omitted() {
    let ctx = setup().await;
    let receiver = Receiver::start().await;
    add_format_hook(&ctx, global(), &receiver.url, WebhookFormat::Json, true).await;
    send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "rel"})),
    )
    .await;
    pass(&ctx, &config(None), Utc::now()).await;
    let envelope: Value = serde_json::from_str(&receiver.last().1).unwrap();
    assert!(
        envelope.get("links").is_none(),
        "no base URL known: {envelope}"
    );

    services::set_oidc_config(
        &ctx.repo,
        &sanshain_service::domain::models::OidcConfig {
            issuer_url: "https://idp.example".to_string(),
            client_id: "sanshain".to_string(),
            client_secret: None,
            redirect_url: "https://sanshain.corp:8443/auth/oidc/callback".to_string(),
            scopes: vec!["openid".to_string()],
            username_claim: "preferred_username".to_string(),
            groups_claim: "groups".to_string(),
            admin_group: String::new(),
        },
    )
    .await
    .unwrap();
    send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "rel2"})),
    )
    .await;
    pass(&ctx, &config(None), Utc::now()).await;
    let envelope: Value = serde_json::from_str(&receiver.last().1).unwrap();
    assert_eq!(
        envelope["links"],
        json!({"graph": "https://sanshain.corp:8443/graph.html"})
    );
}

#[tokio::test]
async fn the_slack_and_teams_formats_carry_the_facts_and_the_deep_link() {
    let ctx = setup().await;
    let slack = Receiver::start().await;
    let teams = Receiver::start().await;
    add_format_hook(
        &ctx,
        consumer("web"),
        &slack.url,
        WebhookFormat::Slack,
        true,
    )
    .await;
    add_format_hook(
        &ctx,
        consumer("web"),
        &teams.url,
        WebhookFormat::Teams,
        true,
    )
    .await;
    provide(
        &ctx,
        "orders",
        "ga",
        &spec("1.0.0", &["/a"], &[]),
        json!({}),
    )
    .await;
    assert_eq!(
        require(&ctx, "web", "orders", "1.0.0", "/a", "").await,
        StatusCode::OK
    );
    provide(
        &ctx,
        "orders",
        "ga",
        &spec("2.0.0", &["/b"], &[]),
        json!({}),
    )
    .await;

    let summary = pass(&ctx, &config(Some("https://sanshain.example")), Utc::now()).await;
    assert_eq!(summary.delivered, 4);

    let (headers, body) = slack.last();
    verify_signature(&headers, &body, "whsec_test");
    assert_eq!(header(&headers, "sanshain-event"), "version.published");
    let message: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        message.as_object().unwrap().keys().collect::<Vec<_>>(),
        vec!["text"]
    );
    let text = message["text"].as_str().unwrap();
    assert!(text.contains("Producer orders published as GA: openapi 2.0.0 on dev (major change against 1.0.0), by root"), "{text}");
    assert!(text.contains("<https://sanshain.example/producers.html?service=orders&api_type=openapi&version=2.0.0|Open in Sanshain>"), "{text}");

    let (headers, body) = teams.last();
    verify_signature(&headers, &body, "whsec_test");
    let message: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(message["type"], "message");
    let attachment = &message["attachments"][0];
    assert_eq!(
        attachment["contentType"],
        "application/vnd.microsoft.card.adaptive"
    );
    let card = &attachment["content"];
    assert_eq!(card["type"], "AdaptiveCard");
    assert_eq!(
        card["$schema"],
        "http://adaptivecards.io/schemas/adaptive-card.json"
    );
    assert_eq!(card["version"], "1.4");
    assert_eq!(card["body"][0]["text"], "version.published");
    assert!(
        card["body"][1]["text"]
            .as_str()
            .unwrap()
            .contains("openapi 2.0.0")
    );
    assert_eq!(card["actions"][0]["type"], "Action.OpenUrl");
    assert_eq!(
        card["actions"][0]["url"],
        "https://sanshain.example/producers.html?service=orders&api_type=openapi&version=2.0.0"
    );

    // The dependency.recorded message names the Consumer and links its page.
    let dependency = slack
        .requests()
        .into_iter()
        .find(|(h, _)| header(h, "sanshain-event") == "dependency.recorded")
        .expect("web's own require reached its hook");
    let text: Value = serde_json::from_str(&dependency.1).unwrap();
    assert!(
        text["text"]
            .as_str()
            .unwrap()
            .contains("Consumer web now requires orders openapi 1.0.0 on dev: 1 endpoint(s)"),
        "{text}"
    );
}

#[tokio::test]
async fn a_rotated_secret_signs_subsequent_deliveries() {
    let ctx = setup().await;
    let receiver = Receiver::start().await;
    let hook = add_format_hook(&ctx, global(), &receiver.url, WebhookFormat::Json, true).await;
    send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "one"})),
    )
    .await;
    pass(&ctx, &config(None), Utc::now()).await;
    let (headers, body) = receiver.last();
    verify_signature(&headers, &body, "whsec_test");

    assert!(
        ctx.repo
            .rotate_webhook_secret(hook, "whsec_rotated", "cafebabe")
            .await
            .unwrap()
    );
    send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "two"})),
    )
    .await;
    pass(&ctx, &config(None), Utc::now()).await;
    let (headers, body) = receiver.last();
    verify_signature(&headers, &body, "whsec_rotated");
    assert_eq!(receiver.requests().len(), 2);
}

#[tokio::test]
async fn a_failing_receiver_is_retried_after_the_backoff_and_delivered_on_success() {
    let ctx = setup().await;
    let receiver = Receiver::start().await;
    receiver.answer_with(500);
    let hook = add_format_hook(&ctx, global(), &receiver.url, WebhookFormat::Json, true).await;
    send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "rel"})),
    )
    .await;

    let t0 = Utc::now();
    assert_eq!(
        pass(&ctx, &config(None), t0).await,
        PassSummary {
            delivered: 0,
            failed: 1,
            dead: 0
        }
    );
    let row = &deliveries(&ctx, hook).await[0];
    assert_eq!(row.status, WebhookDeliveryStatus::Failed);
    assert_eq!(row.attempts, 1);
    assert_eq!(row.last_response_code, Some(500));
    assert_eq!(
        row.last_error.as_deref(),
        Some("the receiver answered HTTP 500")
    );
    assert_eq!(
        row.next_attempt_at,
        (t0 + chrono::Duration::seconds(30)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    );

    // Too early: no attempt.
    receiver.answer_with(200);
    assert_eq!(
        pass(&ctx, &config(None), t0 + chrono::Duration::seconds(10)).await,
        PassSummary::default()
    );
    assert_eq!(receiver.requests().len(), 1);

    // After the backoff: delivered.
    assert_eq!(
        pass(&ctx, &config(None), t0 + chrono::Duration::seconds(31)).await,
        PassSummary {
            delivered: 1,
            failed: 0,
            dead: 0
        }
    );
    let row = &deliveries(&ctx, hook).await[0];
    assert_eq!(row.status, WebhookDeliveryStatus::Delivered);
    assert_eq!(row.attempts, 2);
    assert_eq!(receiver.requests().len(), 2);

    // The backoff doubles: a second failure waits 60 s, a third 120 s.
    receiver.answer_with(503);
    send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "rel2"})),
    )
    .await;
    // A fresh clock: the new delivery is due from its real creation time, which
    // on a slow runner is seconds after `t0` and would make the first pass skip it.
    let t1 = Utc::now();
    pass(&ctx, &config(None), t1).await;
    pass(&ctx, &config(None), t1 + chrono::Duration::seconds(30)).await;
    let row = &deliveries(&ctx, hook).await[0];
    assert_eq!(row.attempts, 2);
    assert_eq!(
        row.next_attempt_at,
        (t1 + chrono::Duration::seconds(90)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    );
}

#[tokio::test]
async fn a_receiver_down_for_a_day_yields_a_dead_delivery_that_a_redeliver_sends_again() {
    let ctx = setup().await;
    let receiver = Receiver::start().await;
    receiver.answer_with(500);
    let hook = add_format_hook(&ctx, global(), &receiver.url, WebhookFormat::Json, true).await;
    send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "rel"})),
    )
    .await;
    let t0 = Utc::now();
    pass(&ctx, &config(None), t0).await;
    // Still down, still inside the window: retried, not dead.
    assert_eq!(
        pass(&ctx, &config(None), t0 + chrono::Duration::hours(2)).await,
        PassSummary {
            delivered: 0,
            failed: 1,
            dead: 0
        }
    );
    // A retry that lands past the cutoff is marked dead.
    let later = t0
        + chrono::Duration::hours(23)
        + chrono::Duration::minutes(59)
        + chrono::Duration::seconds(50);
    assert_eq!(
        pass(&ctx, &config(None), later).await,
        PassSummary {
            delivered: 0,
            failed: 0,
            dead: 1
        }
    );
    let row = &deliveries(&ctx, hook).await[0];
    assert_eq!(row.status, WebhookDeliveryStatus::Dead);
    assert_eq!(row.attempts, 3);
    assert_eq!(row.last_response_code, Some(500));

    // A day later, nothing is attempted for it any more …
    receiver.answer_with(200);
    assert_eq!(
        pass(&ctx, &config(None), t0 + chrono::Duration::hours(26)).await,
        PassSummary::default()
    );
    assert_eq!(receiver.requests().len(), 3);

    // … until it is redelivered by hand: one more attempt, attempts kept.
    let redeliver_at = t0 + chrono::Duration::hours(27);
    assert!(
        ctx.repo
            .reset_webhook_delivery(
                row.id,
                &redeliver_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
            )
            .await
            .unwrap()
    );
    assert_eq!(
        deliveries(&ctx, hook).await[0].status,
        WebhookDeliveryStatus::Pending
    );
    assert_eq!(
        pass(&ctx, &config(None), redeliver_at).await,
        PassSummary {
            delivered: 1,
            failed: 0,
            dead: 0
        }
    );
    let row = &deliveries(&ctx, hook).await[0];
    assert_eq!(row.status, WebhookDeliveryStatus::Delivered);
    assert_eq!(row.attempts, 4);
    assert_eq!(receiver.requests().len(), 4);
}

#[tokio::test]
async fn a_delivery_never_attempted_within_a_day_is_dead_without_an_attempt() {
    let ctx = setup().await;
    let receiver = Receiver::start().await;
    let hook = add_format_hook(&ctx, global(), &receiver.url, WebhookFormat::Json, true).await;
    send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "rel"})),
    )
    .await;
    // The worker only looks 25 hours later (a paused hook resumed, a long outage).
    assert_eq!(
        pass(
            &ctx,
            &config(None),
            Utc::now() + chrono::Duration::hours(25)
        )
        .await,
        PassSummary {
            delivered: 0,
            failed: 0,
            dead: 1
        }
    );
    let row = &deliveries(&ctx, hook).await[0];
    assert_eq!(row.status, WebhookDeliveryStatus::Dead);
    assert_eq!(row.attempts, 0);
    assert!(row.last_error.as_deref().unwrap().contains("24 hours"));
    assert!(receiver.requests().is_empty());
}

#[tokio::test]
async fn a_paused_hook_makes_no_attempt_and_resumes_on_the_backoff_schedule() {
    let ctx = setup().await;
    let receiver = Receiver::start().await;
    receiver.answer_with(500);
    let hook = add_format_hook(&ctx, global(), &receiver.url, WebhookFormat::Json, true).await;
    send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "rel"})),
    )
    .await;
    let t0 = Utc::now();
    pass(&ctx, &config(None), t0).await;
    assert_eq!(receiver.requests().len(), 1);

    ctx.repo
        .update_webhook(
            hook,
            &sanshain_service::domain::models::WebhookPatch {
                paused: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    receiver.answer_with(200);
    send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "rel2"})),
    )
    .await;
    assert_eq!(
        pass(&ctx, &config(None), t0 + chrono::Duration::minutes(5)).await,
        PassSummary::default()
    );
    assert_eq!(
        receiver.requests().len(),
        1,
        "a paused hook is never attempted"
    );
    let rows = deliveries(&ctx, hook).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].status,
        WebhookDeliveryStatus::Pending,
        "kept, not dropped"
    );
    assert_eq!(rows[1].status, WebhookDeliveryStatus::Failed);

    ctx.repo
        .update_webhook(
            hook,
            &sanshain_service::domain::models::WebhookPatch {
                paused: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        pass(&ctx, &config(None), t0 + chrono::Duration::minutes(6)).await,
        PassSummary {
            delivered: 2,
            failed: 0,
            dead: 0
        }
    );
    assert_eq!(receiver.requests().len(), 3);
}

#[tokio::test]
async fn a_host_removed_from_the_allow_list_stops_delivery_with_a_reason() {
    let ctx = setup().await;
    let receiver = Receiver::start().await;
    sanshain_service::application::webhook_service::set_webhook_allowed_hosts(
        &ctx.repo,
        &["http://127.0.0.1".to_string()],
    )
    .await
    .unwrap();
    let hook = add_format_hook(&ctx, global(), &receiver.url, WebhookFormat::Json, false).await;
    send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "rel"})),
    )
    .await;
    assert_eq!(pass(&ctx, &config(None), Utc::now()).await.delivered, 1);

    sanshain_service::application::webhook_service::set_webhook_allowed_hosts(&ctx.repo, &[])
        .await
        .unwrap();
    send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "rel2"})),
    )
    .await;
    assert_eq!(
        pass(&ctx, &config(None), Utc::now()).await,
        PassSummary {
            delivered: 0,
            failed: 1,
            dead: 0
        }
    );
    assert_eq!(
        receiver.requests().len(),
        1,
        "no POST to a host no longer allowed"
    );
    let row = &deliveries(&ctx, hook).await[0];
    assert_eq!(row.status, WebhookDeliveryStatus::Failed);
    assert_eq!(row.last_response_code, None);
    assert!(
        row.last_error
            .as_deref()
            .unwrap()
            .contains("not in the allow-list"),
        "{row:?}"
    );

    // An admin-registered hook is exempt, at delivery as at registration.
    let exempt = add_format_hook(&ctx, global(), &receiver.url, WebhookFormat::Json, true).await;
    send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "rel3"})),
    )
    .await;
    let summary = pass(
        &ctx,
        &config(None),
        Utc::now() + chrono::Duration::minutes(2),
    )
    .await;
    assert_eq!(summary.delivered, 1);
    assert_eq!(
        deliveries(&ctx, exempt).await[0].status,
        WebhookDeliveryStatus::Delivered
    );
}

#[tokio::test]
async fn deliveries_are_counted_by_outcome_and_the_backlog_is_a_gauge() {
    let ctx = setup().await;
    let receiver = Receiver::start().await;
    receiver.answer_with(500);
    add_format_hook(&ctx, global(), &receiver.url, WebhookFormat::Json, true).await;
    send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "rel"})),
    )
    .await;
    pass(&ctx, &config(None), Utc::now()).await;
    let (status, _, body) = send(&ctx, "GET", "/metrics", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("sanshain_webhook_deliveries_total{outcome=\"failed\"}"),
        "{body}"
    );
    assert!(body.contains("sanshain_webhook_outbox_pending 1"), "{body}");
    receiver.answer_with(200);
    pass(
        &ctx,
        &config(None),
        Utc::now() + chrono::Duration::minutes(1),
    )
    .await;
    let (_, _, body) = send(&ctx, "GET", "/metrics", None).await;
    assert!(
        body.contains("sanshain_webhook_deliveries_total{outcome=\"delivered\"}"),
        "{body}"
    );
    assert!(body.contains("sanshain_webhook_outbox_pending 0"), "{body}");
}

// --- Hook management over the routes ---

#[cfg(test)]
fn as_json(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or_else(|e| panic!("expected JSON, got {e}: {body}"))
}

/// A Maintainer of `orders` and nothing else, with a session token.
#[cfg(test)]
async fn maintainer_token(ctx: &TestContext) -> String {
    let hash = services::hash_password("maint-pass").unwrap();
    let user = ctx.repo.create_user("maint", &hash, true).await.unwrap();
    let orders = ctx
        .repo
        .find_service("orders")
        .await
        .unwrap()
        .expect("orders exists");
    ctx.repo.add_user_maintainer(orders, user.id).await.unwrap();
    ctx.repo
        .create_session(user.id, "2099-12-31T23:59:59")
        .await
        .unwrap()
        .token
}

#[cfg(test)]
async fn set_allowed(ctx: &TestContext, hosts: &[&str]) {
    let (status, _, body) = send(
        ctx,
        "POST",
        "/admin/settings/webhook-allowed-hosts",
        Some(json!({"hosts": hosts})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[cfg(test)]
async fn audit_actions(ctx: &TestContext) -> Vec<String> {
    ctx.repo
        .get_recent_audit_logs(50)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.action)
        .collect()
}

#[tokio::test]
async fn creating_a_hook_shows_the_secret_once_and_lists_without_it() {
    let ctx = setup().await;
    let (status, _, body) = send(
        &ctx,
        "POST",
        "/admin/webhooks",
        Some(json!({
            "url": "https://hooks.slack.com/services/T/B/x",
            "format": "slack",
            "event_types": ["version.published", "provide.rejected"],
            "producer_glob": "pay*",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let created = as_json(&body);
    let secret = created["secret"].as_str().unwrap();
    assert!(
        secret.starts_with("whsec_") && secret.len() == 70,
        "{secret}"
    );
    let hook = &created["webhook"];
    assert_eq!(hook["scope"], json!({"kind": "global"}));
    assert_eq!(hook["format"], "slack");
    assert_eq!(
        hook["event_types"],
        json!(["provide.rejected", "version.published"])
    );
    assert_eq!(hook["producer_glob"], "pay*");
    assert_eq!(hook["paused"], false);
    assert_eq!(
        hook["allow_list_exempt"], true,
        "registered with manage_settings"
    );
    assert_eq!(hook["created_by"], "root");
    assert_eq!(hook["secret_hint"].as_str().unwrap().len(), 8);
    assert!(
        hook.get("secret").is_none(),
        "the hook never carries its secret"
    );
    let id = hook["id"].as_i64().unwrap();

    let (status, _, body) = send(&ctx, "GET", "/admin/webhooks", None).await;
    assert_eq!(status, StatusCode::OK);
    let listed = as_json(&body);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert!(listed[0].get("secret").is_none());
    assert!(
        !body.contains(secret),
        "the listing never reveals the secret"
    );
    let (status, _, body) = send(&ctx, "GET", &format!("/admin/webhooks/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.contains(secret));

    // The stored secret is the one shown: it signs the deliveries.
    let stored = ctx.repo.get_webhook(id).await.unwrap().unwrap();
    assert_eq!(stored.secret, secret);
    assert_eq!(
        audit_actions(&ctx).await,
        vec!["WEBHOOK_CREATED".to_string()]
    );
}

#[tokio::test]
async fn a_hook_is_refused_when_the_url_or_filter_is_junk() {
    let ctx = setup().await;
    for (body, why) in [
        (json!({"url": "ftp://x.example/y"}), "scheme"),
        (json!({"url": "nope"}), "not a URL"),
        (
            json!({"url": "https://x.example/y", "format": "carrier-pigeon"}),
            "format",
        ),
        (
            json!({"url": "https://x.example/y", "event_types": ["webhook.test"]}),
            "test type",
        ),
        (
            json!({"url": "https://x.example/y", "event_types": ["nope"]}),
            "unknown type",
        ),
        (
            json!({"url": "https://x.example/y", "headers": {"X": "y"}}),
            "no custom headers",
        ),
    ] {
        let (status, _, text) = send(&ctx, "POST", "/admin/webhooks", Some(body)).await;
        assert!(
            status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
            "{why}: {status} {text}"
        );
    }
    assert!(ctx.repo.list_webhooks(None).await.unwrap().is_empty());
}

#[tokio::test]
async fn system_hooks_obey_the_allow_list_for_maintainers_and_not_for_admins() {
    let ctx = setup().await;
    provide(
        &ctx,
        "orders",
        "ga",
        &spec("1.0.0", &["/a"], &[]),
        json!({}),
    )
    .await;
    let maint = maintainer_token(&ctx).await;
    let hook = json!({"url": "https://hooks.slack.com/services/T/B/x"});

    // An empty list: the Maintainer is told the admin has to allow hosts first.
    let (status, _, body) = send_as(
        &ctx,
        &maint,
        "POST",
        "/admin/producers/orders/webhooks",
        Some(hook.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(
        body.contains("add the receiver's host to the allow-list"),
        "{body}"
    );

    // The admin may register any host regardless.
    let (status, _, body) = send(
        &ctx,
        "POST",
        "/admin/producers/orders/webhooks",
        Some(hook.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(as_json(&body)["webhook"]["allow_list_exempt"], true);

    set_allowed(&ctx, &["*.webhook.office.com", "http://n8n.corp.internal"]).await;
    let (status, _, body) = send_as(
        &ctx,
        &maint,
        "POST",
        "/admin/producers/orders/webhooks",
        Some(hook.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(
        body.contains("'hooks.slack.com' is not in the webhook allow-list"),
        "{body}"
    );
    assert!(
        body.contains("*.webhook.office.com"),
        "the refusal names the allowed hosts: {body}"
    );

    let (status, _, body) = send_as(
        &ctx,
        &maint,
        "POST",
        "/admin/producers/orders/webhooks",
        Some(json!({"url": "http://corp.webhook.office.com/x"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "plain http needs an http pattern: {body}"
    );
    assert!(body.contains("plain http://"), "{body}");

    let (status, _, body) = send_as(
        &ctx,
        &maint,
        "POST",
        "/admin/producers/orders/webhooks",
        Some(json!({"url": "https://corp.webhook.office.com/webhookb2/x", "format": "teams"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let created = as_json(&body);
    assert_eq!(created["webhook"]["allow_list_exempt"], false);
    assert_eq!(
        created["webhook"]["scope"],
        json!({"kind": "producer", "name": "orders"})
    );
    assert_eq!(created["webhook"]["created_by"], "maint");

    // The same Maintainer manages the Consumer of the same name, and nobody else's.
    let (status, _, body) = send_as(
        &ctx,
        &maint,
        "POST",
        "/admin/consumers/orders/webhooks",
        Some(json!({"url": "http://n8n.corp.internal:5678/hook"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        as_json(&body)["webhook"]["scope"],
        json!({"kind": "consumer", "name": "orders"})
    );
    let (status, _, _) = send_as(
        &ctx,
        &maint,
        "GET",
        "/admin/producers/billing/webhooks",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = send_as(&ctx, &maint, "GET", "/admin/webhooks", None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // An unknown system is a 404, for the admin too.
    let (status, _, _) = send(&ctx, "POST", "/admin/producers/nope/webhooks", Some(hook)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_system_route_never_reaches_a_hook_of_another_scope() {
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
    let (_, _, body) = send(
        &ctx,
        "POST",
        "/admin/webhooks",
        Some(json!({"url": "https://g.example/x"})),
    )
    .await;
    let global_id = as_json(&body)["webhook"]["id"].as_i64().unwrap();
    let (_, _, body) = send(
        &ctx,
        "POST",
        "/admin/producers/billing/webhooks",
        Some(json!({"url": "https://b.example/x"})),
    )
    .await;
    let billing_id = as_json(&body)["webhook"]["id"].as_i64().unwrap();

    let maint = maintainer_token(&ctx).await;
    for id in [global_id, billing_id] {
        for (method, path) in [
            ("GET", String::new()),
            ("PATCH", String::new()),
            ("DELETE", String::new()),
            ("POST", "/test".to_string()),
            ("POST", "/rotate-secret".to_string()),
            ("GET", "/deliveries".to_string()),
            ("POST", "/deliveries/1/redeliver".to_string()),
        ] {
            let uri = format!("/admin/producers/orders/webhooks/{id}{path}");
            let body = (method == "PATCH").then(|| json!({"paused": true}));
            let (status, _, text) = send_as(&ctx, &maint, method, &uri, body).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}: {text}");
        }
    }
    assert_eq!(
        ctx.repo.list_webhooks(None).await.unwrap().len(),
        2,
        "nothing was touched"
    );
    let (status, _, _) = send(
        &ctx,
        "GET",
        &format!("/admin/producers/orders/webhooks/{global_id}"),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the admin is scoped by the route too"
    );
}

#[tokio::test]
async fn pause_resume_test_rotate_redeliver_and_delete_work_over_the_routes_and_are_audited() {
    let ctx = setup().await;
    let receiver = Receiver::start().await;
    let (_, _, body) = send(
        &ctx,
        "POST",
        "/admin/webhooks",
        Some(json!({"url": receiver.url, "format": "json"})),
    )
    .await;
    let created = as_json(&body);
    let id = created["webhook"]["id"].as_i64().unwrap();
    let secret = created["secret"].as_str().unwrap().to_string();

    // Test event: queued, delivered by the worker, signed with the secret shown.
    let (status, _, body) = send(&ctx, "POST", &format!("/admin/webhooks/{id}/test"), None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let event_id = as_json(&body)["event_id"].as_str().unwrap().to_string();
    assert_eq!(pass(&ctx, &config(None), Utc::now()).await.delivered, 1);
    let (headers, body) = receiver.last();
    verify_signature(&headers, &body, &secret);
    assert_eq!(header(&headers, "sanshain-event"), "webhook.test");
    let envelope = as_json(&body);
    assert_eq!(envelope["id"], event_id);
    assert_eq!(envelope["type"], "webhook.test");
    assert_eq!(envelope["data"]["hook_id"], id);
    assert!(
        envelope["data"]["message"]
            .as_str()
            .unwrap()
            .contains("test event")
    );

    // The delivery log over the route.
    let (status, _, body) = send(
        &ctx,
        "GET",
        &format!("/admin/webhooks/{id}/deliveries?limit=10"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let log = as_json(&body);
    assert_eq!(log.as_array().unwrap().len(), 1);
    assert_eq!(log[0]["status"], "delivered");
    assert_eq!(log[0]["event_type"], "webhook.test");
    assert_eq!(log[0]["attempts"], 1);
    assert_eq!(log[0]["last_response_code"], 200);
    let delivery_id = log[0]["id"].as_i64().unwrap();

    // Pause: no attempt; resume: delivered.
    let (status, _, body) = send(
        &ctx,
        "PATCH",
        &format!("/admin/webhooks/{id}"),
        Some(json!({"paused": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(as_json(&body)["paused"], true);
    send(
        &ctx,
        "POST",
        "/admin/branches",
        Some(json!({"name": "rel"})),
    )
    .await;
    assert_eq!(
        pass(&ctx, &config(None), Utc::now()).await,
        PassSummary::default()
    );
    let (_, _, body) = send(
        &ctx,
        "PATCH",
        &format!("/admin/webhooks/{id}"),
        Some(json!({"paused": false, "event_types": [], "producer_glob": null})),
    )
    .await;
    assert_eq!(as_json(&body)["paused"], false);
    assert_eq!(pass(&ctx, &config(None), Utc::now()).await.delivered, 1);

    // Rotate: the new secret signs the next delivery, the old one no longer does.
    let (status, _, body) = send(
        &ctx,
        "POST",
        &format!("/admin/webhooks/{id}/rotate-secret"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rotated = as_json(&body);
    let new_secret = rotated["secret"].as_str().unwrap().to_string();
    assert_ne!(new_secret, secret);
    assert_ne!(
        rotated["webhook"]["secret_hint"],
        created["webhook"]["secret_hint"]
    );

    // Redeliver the test delivery: sent again, signed with the rotated secret.
    let (status, _, body) = send(
        &ctx,
        "POST",
        &format!("/admin/webhooks/{id}/deliveries/{delivery_id}/redeliver"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(as_json(&body)["status"], "pending");
    assert_eq!(as_json(&body)["attempts"], 1, "the attempt count is kept");
    assert_eq!(pass(&ctx, &config(None), Utc::now()).await.delivered, 1);
    let (headers, body) = receiver.last();
    verify_signature(&headers, &body, &new_secret);
    assert_eq!(as_json(&body)["id"], event_id);
    let (status, _, _) = send(
        &ctx,
        "POST",
        &format!("/admin/webhooks/{id}/deliveries/999999/redeliver"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Delete: gone, with its log.
    let (status, _, _) = send(&ctx, "DELETE", &format!("/admin/webhooks/{id}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = send(&ctx, "GET", &format!("/admin/webhooks/{id}"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        ctx.repo.count_pending_webhook_deliveries().await.unwrap(),
        0
    );

    let actions = audit_actions(&ctx).await;
    for expected in [
        "WEBHOOK_CREATED",
        "WEBHOOK_PAUSED",
        "WEBHOOK_RESUMED",
        "WEBHOOK_SECRET_ROTATED",
        "WEBHOOK_REDELIVERED",
        "WEBHOOK_DELETED",
    ] {
        assert!(
            actions.iter().any(|a| a == expected),
            "{expected} missing from {actions:?}"
        );
    }
}

#[tokio::test]
async fn the_allow_list_setting_is_validated_stored_and_audited() {
    let ctx = setup().await;
    let (status, _, body) = send(&ctx, "GET", "/admin/settings/webhook-allowed-hosts", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(as_json(&body), json!({"hosts": []}));
    let (status, _, body) = send(
        &ctx,
        "POST",
        "/admin/settings/webhook-allowed-hosts",
        Some(json!({"hosts": [" Hooks.Slack.com", "*.webhook.office.com", "http://n8n.corp.internal", "hooks.slack.com"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        as_json(&body),
        json!({"hosts": ["hooks.slack.com", "*.webhook.office.com", "http://n8n.corp.internal"]})
    );
    let (status, _, body) = send(
        &ctx,
        "POST",
        "/admin/settings/webhook-allowed-hosts",
        Some(json!({"hosts": ["https://hooks.slack.com/x"]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("not a host pattern"), "{body}");
    let (_, _, body) = send(&ctx, "GET", "/admin/settings/webhook-allowed-hosts", None).await;
    assert_eq!(
        as_json(&body)["hosts"].as_array().unwrap().len(),
        3,
        "a refused list changes nothing"
    );
    assert!(
        audit_actions(&ctx)
            .await
            .contains(&"SET_WEBHOOK_ALLOWED_HOSTS".to_string())
    );
}

// --- Review follow-ups ---

#[tokio::test]
async fn a_catalog_push_does_not_reach_the_systems_consumers() {
    let ctx = setup().await;
    provide(
        &ctx,
        "orders",
        "ga",
        &spec("1.0.0", &["/a"], &[]),
        json!({}),
    )
    .await;
    assert_eq!(
        require(&ctx, "web", "orders", "1.0.0", "/a", "").await,
        StatusCode::OK
    );
    let web = add_hook(&ctx, consumer("web"), "https://hooks.example/web").await;
    let own = add_hook(&ctx, producer("orders"), "https://hooks.example/orders").await;
    let good = "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: orders\nspec:\n  dependsOn:\n    - resource:default/orders-db\n";
    let (status, _, _) = send(
        &ctx,
        "PUT",
        "/catalog/orders",
        Some(json!({"catalog_info_yaml": good})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        event_types(&deliveries(&ctx, own).await),
        vec![WebhookEventType::CatalogRecorded]
    );
    assert!(
        deliveries(&ctx, web).await.is_empty(),
        "a catalog push is about the system, not about what it provides"
    );
}

#[tokio::test]
async fn a_consumer_hook_follows_a_trunk_only_edge() {
    let ctx = setup().await;
    provide(
        &ctx,
        "orders",
        "ga",
        &spec("1.0.0", &["/a"], &[]),
        json!({}),
    )
    .await;
    // cli requires orders on trunk; its dev-graph edge is then pruned, so
    // only the open trunk pin remains.
    assert_eq!(
        require(&ctx, "cli", "orders", "1.0.0", "/a", "&trunk=true").await,
        StatusCode::OK
    );
    ctx.repo
        .delete_stale_dependencies("2999-01-01T00:00:00Z")
        .await
        .unwrap();
    let cli = add_hook(&ctx, consumer("cli"), "https://hooks.example/cli").await;
    provide(
        &ctx,
        "orders",
        "ga",
        &spec("1.1.0", &["/a"], &[]),
        json!({}),
    )
    .await;
    assert_eq!(
        event_types(&deliveries(&ctx, cli).await),
        vec![WebhookEventType::VersionPublished]
    );
}

#[cfg(test)]
async fn provide_asyncapi(ctx: &TestContext, producer: &str, yaml: &str) -> (StatusCode, String) {
    let (status, _, body) = send(
        ctx,
        "POST",
        "/provide/asyncapi",
        Some(json!({"producername": producer, "asyncapi_yaml": yaml, "stability": "ga"})),
    )
    .await;
    (status, body)
}

#[cfg(test)]
fn asyncapi_user_created(producer: &str, id_type: &str) -> String {
    format!(
        "asyncapi: 2.6.0\ninfo:\n  title: {producer}\n  version: 1.0.0\nchannels:\n  user-created:\n    publish:\n      message:\n        name: UserCreated\n        payload:\n          type: object\n          properties:\n            id: {{ type: {id_type} }}\n"
    )
}

#[tokio::test]
async fn a_message_contract_conflict_is_a_rejected_provide() {
    let ctx = setup().await;
    let (status, body) =
        provide_asyncapi(&ctx, "svc-a", &asyncapi_user_created("svc-a", "string")).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    // svc-b must exist as a Producer for its rejection to be recorded (the
    // telemetry rule), so it publishes something unrelated first.
    let (status, _) = provide_asyncapi(
        &ctx,
        "svc-b",
        "asyncapi: 2.6.0\ninfo:\n  title: b\n  version: 0.1.0\nchannels:\n  other:\n    publish:\n      message:\n        name: Other\n        payload:\n          type: string\n",
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let hook = add_hook(&ctx, producer("svc-b"), "https://hooks.example/b").await;

    let (status, body) =
        provide_asyncapi(&ctx, "svc-b", &asyncapi_user_created("svc-b", "integer")).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let rows = deliveries(&ctx, hook).await;
    assert_eq!(event_types(&rows), vec![WebhookEventType::ProvideRejected]);
    let data = event_data(&ctx, &rows[0]).await;
    assert_eq!(data["rule"], "message_contract_conflict");
    assert_eq!(data["producer"], "svc-b");
    assert_eq!(data["api_type"], "asyncapi");
    assert_eq!(data["version"], "1.0.0");
    assert_eq!(data["proposed_version"], Value::Null);
    assert!(
        data["message"]
            .as_str()
            .unwrap()
            .contains("owned by service 'svc-a'"),
        "{data}"
    );
    let actions = audit_actions(&ctx).await;
    assert!(
        actions.iter().any(|a| a == "VERSION_REJECTED"),
        "{actions:?}"
    );
}

#[tokio::test]
async fn audit_entries_name_the_host_and_never_the_hook_url() {
    let ctx = setup().await;
    let url = "https://hooks.slack.com/services/T000/B000/secretpathpart";
    let (status, _, body) = send(&ctx, "POST", "/admin/webhooks", Some(json!({"url": url}))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = as_json(&body)["webhook"]["id"].as_i64().unwrap();
    let (status, _, _) = send(&ctx, "DELETE", &format!("/admin/webhooks/{id}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let entries = ctx.repo.get_recent_audit_logs(10).await.unwrap();
    let hook_entries: Vec<_> = entries
        .iter()
        .filter(|e| e.action.starts_with("WEBHOOK_"))
        .collect();
    assert_eq!(hook_entries.len(), 2);
    for entry in hook_entries {
        assert!(
            entry.details.contains("host hooks.slack.com"),
            "{}",
            entry.details
        );
        assert!(
            !entry.details.contains("secretpathpart"),
            "{}",
            entry.details
        );
        assert!(!entry.details.contains("/services/"), "{}", entry.details);
    }
}
