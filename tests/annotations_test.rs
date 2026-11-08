//! Annotations (#91) at the router seam: project-declared entries the build
//! fills in, `scope: version` stored with the provided version and
//! `scope: system` once per system, on every call naming the system.

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
fn spec(version: &str, path: &str) -> String {
    format!(
        "openapi: 3.0.3\ninfo:\n  title: T\n  version: {version}\npaths:\n  /{path}:\n    get:\n      responses:\n        '200':\n          description: OK\n"
    )
}

#[cfg(test)]
fn ann(scope: &str, name: &str, value: &str) -> Value {
    json!({"scope": scope, "name": name, "value": value})
}

#[cfg(test)]
async fn provide(ctx: &TestContext, yaml: &str, annotations: Value) -> (StatusCode, String) {
    let (status, _, body) = send(
        ctx,
        "POST",
        "/provide",
        Some(json!({
            "producername": "orders",
            "openapi_yaml": yaml,
            "stability": "snapshot",
            "annotations": annotations,
        })),
    )
    .await;
    (status, body)
}

/// `(name, value)` of a version's stored annotations, by name.
#[cfg(test)]
async fn version_annotations(ctx: &TestContext, version: &str) -> Vec<(String, String)> {
    let (major, minor, patch): (i64, i64, i64) = {
        let mut parts = version.split('.').map(|p| p.parse::<i64>().unwrap());
        (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
        )
    };
    sqlx::query_as(
        "SELECT a.name, a.value FROM spec_version_annotation a \
         JOIN spec_versions v ON v.id = a.spec_version_id \
         JOIN services s ON s.id = v.service_id \
         WHERE s.name = 'orders' AND v.major = ? AND v.minor = ? AND v.patch = ? ORDER BY a.name",
    )
    .bind(major)
    .bind(minor)
    .bind(patch)
    .fetch_all(&ctx.pool)
    .await
    .unwrap()
}

#[cfg(test)]
async fn system_annotations(ctx: &TestContext, system: &str) -> Vec<(String, String)> {
    sqlx::query_as("SELECT name, value FROM system_annotation WHERE system_name = ? ORDER BY name")
        .bind(system)
        .fetch_all(&ctx.pool)
        .await
        .unwrap()
}

#[cfg(test)]
fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(n, v)| (n.to_string(), v.to_string()))
        .collect()
}

#[tokio::test]
async fn a_provide_stores_version_and_system_annotations_apart() {
    let ctx = setup().await;
    let (status, body) = provide(
        &ctx,
        &spec("1.0.0", "users"),
        json!([
            ann(
                "version",
                "requirements",
                "http://solar.system/id-7/requirements/42"
            ),
            ann("system", "jenkins", "https://jenkins.corp/job/orders"),
        ]),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "got: {body}");
    assert_eq!(
        version_annotations(&ctx, "1.0.0").await,
        pairs(&[("requirements", "http://solar.system/id-7/requirements/42")])
    );
    assert_eq!(
        system_annotations(&ctx, "orders").await,
        pairs(&[("jenkins", "https://jenkins.corp/job/orders")])
    );
}

#[tokio::test]
async fn a_real_write_replaces_the_names_it_sends_and_a_no_op_only_fills() {
    let ctx = setup().await;
    provide(
        &ctx,
        &spec("1.0.0", "users"),
        json!([ann("version", "req", "one"), ann("version", "doc", "d1")]),
    )
    .await;
    // Identical bytes: a no-op that may only fill names not set yet.
    let (status, body) = provide(
        &ctx,
        &spec("1.0.0", "users"),
        json!([ann("version", "req", "two"), ann("version", "extra", "e")]),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "got: {body}");
    assert_eq!(
        version_annotations(&ctx, "1.0.0").await,
        pairs(&[("doc", "d1"), ("extra", "e"), ("req", "one")])
    );
    // New snapshot bytes for the same version: a real write replaces `req`
    // and keeps the names it does not send.
    let (status, body) = provide(
        &ctx,
        &spec("1.0.0", "orders"),
        json!([ann("version", "req", "three")]),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "got: {body}");
    assert_eq!(
        version_annotations(&ctx, "1.0.0").await,
        pairs(&[("doc", "d1"), ("extra", "e"), ("req", "three")])
    );
}

#[tokio::test]
async fn a_dry_run_and_a_refused_provide_store_nothing() {
    let ctx = setup().await;
    let (status, _, body) = send(
        &ctx,
        "POST",
        "/provide",
        Some(json!({
            "producername": "orders",
            "openapi_yaml": spec("1.0.0", "users"),
            "stability": "snapshot",
            "dry_run": true,
            "annotations": [ann("version", "req", "x"), ann("system", "ci", "y")],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "got: {body}");
    // A version this broken is refused before anything is written.
    let (status, _) = provide(
        &ctx,
        "openapi: 3.0.3\ninfo:\n  title: T\npaths: {}\n",
        json!([ann("system", "ci", "y")]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(system_annotations(&ctx, "orders").await.is_empty());
    let rows: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM spec_version_annotation")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    assert_eq!(rows.0, 0);
}

#[tokio::test]
async fn out_of_bounds_annotations_are_a_400_naming_the_entry() {
    let ctx = setup().await;
    let (status, body) = provide(
        &ctx,
        &spec("1.0.0", "users"),
        json!([ann("version", "bad name", "x")]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.contains("annotations: name 'bad name' is not of letters, digits, '-' and '_'"),
        "got: {body}"
    );
    // Nothing of a refused call is kept, the version included.
    let rows: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM spec_versions")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    assert_eq!(rows.0, 0);
}

#[tokio::test]
async fn a_retire_carries_no_annotations() {
    let ctx = setup().await;
    provide(&ctx, &spec("1.0.0", "users"), json!([])).await;
    let (status, _, body) = send(
        &ctx,
        "POST",
        "/provide",
        Some(json!({
            "producername": "orders",
            "retired": true,
            "annotations": [ann("system", "ci", "y")],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.contains("a retire publishes nothing and carries no annotations"),
        "got: {body}"
    );
}

#[cfg(test)]
fn encoded(value: &Value) -> String {
    let raw = value.to_string();
    let mut out = String::new();
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[tokio::test]
async fn a_successful_require_stores_the_consumers_system_annotations() {
    let ctx = setup().await;
    provide(&ctx, &spec("1.0.0", "users"), json!([])).await;
    let annotations = json!([ann("system", "jenkins", "https://jenkins.corp/job/webapp")]);
    let (status, _, body) = send(
        &ctx,
        "GET",
        &format!(
            "/require?consumername=webapp&producername=orders&version=1.0.0&path=/users&method=GET&annotations={}",
            encoded(&annotations)
        ),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    assert_eq!(
        system_annotations(&ctx, "webapp").await,
        pairs(&[("jenkins", "https://jenkins.corp/job/webapp")])
    );

    // A failed require records nothing, its annotations included.
    let (status, _, _) = send(
        &ctx,
        "GET",
        &format!(
            "/require?consumername=other&producername=orders&version=9.9.9&path=/users&method=GET&annotations={}",
            encoded(&annotations)
        ),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(system_annotations(&ctx, "other").await.is_empty());
}

#[tokio::test]
async fn version_scope_on_a_require_and_a_malformed_parameter_are_refused() {
    let ctx = setup().await;
    provide(&ctx, &spec("1.0.0", "users"), json!([])).await;
    let base = "/require?consumername=webapp&producername=orders&version=1.0.0&path=/users&method=GET&annotations=";
    let (status, _, body) = send(
        &ctx,
        "GET",
        &format!("{base}{}", encoded(&json!([ann("version", "req", "x")]))),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.contains("'req' has scope version, which travels only on a provide"),
        "got: {body}"
    );
    let (status, _, body) = send(&ctx, "GET", &format!("{base}not-json"), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.contains("annotations: not a JSON array"),
        "got: {body}"
    );
    // Neither recorded the dependency.
    let rows: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM dependencies")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    assert_eq!(rows.0, 0);
}

#[tokio::test]
async fn a_bundle_and_a_catalog_push_store_system_annotations() {
    let ctx = setup().await;
    provide(&ctx, &spec("1.0.0", "users"), json!([])).await;
    let (status, _, body) = send(
        &ctx,
        "POST",
        "/require-bundle",
        Some(json!({
            "consumername": "webapp",
            "producername": "orders",
            "version": "1.0.0",
            "endpoints": [{"path": "/users", "method": "GET"}],
            "annotations": [ann("system", "team", "web")],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    let catalog = "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: webapp\nspec:\n  type: service\n";
    let (status, _, body) = send(
        &ctx,
        "PUT",
        "/catalog/webapp",
        Some(json!({
            "catalog_info_yaml": catalog,
            "annotations": [ann("system", "team", "web-platform"), ann("system", "wiki", "https://wiki/web")],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    assert_eq!(
        system_annotations(&ctx, "webapp").await,
        pairs(&[("team", "web-platform"), ("wiki", "https://wiki/web")])
    );
}

#[tokio::test]
async fn deleting_a_version_deletes_its_annotations() {
    let ctx = setup().await;
    provide(
        &ctx,
        &spec("1.0.0", "users"),
        json!([ann("version", "req", "x")]),
    )
    .await;
    let (status, _, body) = send(
        &ctx,
        "DELETE",
        "/admin/producers/orders/versions/openapi/1.0.0",
        None,
    )
    .await;
    assert!(status.is_success(), "got: {status} {body}");
    let rows: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM spec_version_annotation")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    assert_eq!(rows.0, 0);
}

#[cfg(test)]
async fn get_ok(ctx: &TestContext, uri: &str) -> Value {
    let (status, _, body) = send(ctx, "GET", uri, None).await;
    assert_eq!(status, StatusCode::OK, "{uri} got: {body}");
    serde_json::from_str(&body).unwrap()
}

#[tokio::test]
async fn the_annotations_show_on_versions_participants_and_the_report() {
    let ctx = setup().await;
    provide(
        &ctx,
        &spec("1.0.0", "users"),
        json!([
            ann("version", "requirements", "https://req/42"),
            ann("system", "jenkins", "https://jenkins.corp/job/orders"),
        ]),
    )
    .await;
    let (status, _, body) = send(
        &ctx,
        "POST",
        "/require-bundle",
        Some(json!({
            "consumername": "webapp",
            "producername": "orders",
            "version": "1.0.0",
            "endpoints": [{"path": "/users", "method": "GET"}],
            "annotations": [ann("system", "team", "web")],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");

    let versions = get_ok(&ctx, "/producers/orders/versions").await;
    assert_eq!(
        versions[0]["annotations"],
        json!([{"name": "requirements", "value": "https://req/42"}])
    );
    let producer = get_ok(&ctx, "/admin/producers/orders").await;
    assert_eq!(
        producer["annotations"],
        json!([{"name": "jenkins", "value": "https://jenkins.corp/job/orders"}])
    );
    let consumer = get_ok(&ctx, "/admin/consumers/webapp").await;
    assert_eq!(
        consumer["annotations"],
        json!([{"name": "team", "value": "web"}])
    );
    let report = get_ok(&ctx, "/report").await;
    assert_eq!(
        report["system_annotations"],
        json!({
            "orders": [{"name": "jenkins", "value": "https://jenkins.corp/job/orders"}],
            "webapp": [{"name": "team", "value": "web"}],
        })
    );
}

#[tokio::test]
async fn a_system_losing_its_last_role_loses_its_annotations() {
    let ctx = setup().await;
    provide(&ctx, &spec("1.0.0", "users"), json!([])).await;
    let (status, _, body) = send(
        &ctx,
        "POST",
        "/require-bundle",
        Some(json!({
            "consumername": "webapp",
            "producername": "orders",
            "version": "1.0.0",
            "endpoints": [{"path": "/users", "method": "GET"}],
            "annotations": [ann("system", "team", "web")],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    let (status, _, body) = send(&ctx, "DELETE", "/admin/consumers/webapp", None).await;
    assert!(status.is_success(), "got: {status} {body}");
    assert!(system_annotations(&ctx, "webapp").await.is_empty());
}
