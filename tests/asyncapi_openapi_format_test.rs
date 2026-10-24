//! `format=openapi` on the AsyncAPI requires, at the router seam: a single
//! `/require/asyncapi` and a `/require-bundle` answer the kept messages'
//! payload schemas as OpenAPI 3.0.3; other API types, unknown formats and
//! unconvertible specs are `400` and record nothing.

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
    /// Session token of a fully privileged user.
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

    TestContext {
        app: create_app(test_app_state(repo)),
        token: session.token,
    }
}

#[cfg(test)]
async fn send(
    ctx: &TestContext,
    method: &str,
    uri: &str,
    body: Option<Value>,
    extra_headers: &[(&str, &str)],
) -> (StatusCode, HeaderMap, String) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("Authorization", format!("Bearer {}", ctx.token));
    for (name, value) in extra_headers {
        builder = builder.header(*name, *value);
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

const EVENTS_V2: &str = "asyncapi: 2.6.0
info:
  title: Events
  version: 1.0.0
channels:
  user.created:
    publish:
      message:
        $ref: '#/components/messages/UserCreated'
  order.placed:
    publish:
      message:
        messageId: OrderPlaced
        payload:
          type: object
          properties:
            order:
              $ref: '#/components/schemas/Order'
components:
  messages:
    UserCreated:
      payload:
        $ref: '#/components/schemas/User'
  schemas:
    User:
      type: object
      properties:
        address:
          $ref: '#/components/schemas/Address'
    Address:
      type: object
      properties:
        street:
          type: string
    Order:
      type: object
      properties:
        id:
          type: string
";

const AVRO_V2: &str = "asyncapi: 2.6.0
info:
  title: Avro
  version: 1.0.0
channels:
  user.created:
    publish:
      message:
        name: UserCreated
        schemaFormat: application/vnd.apache.avro;version=1.9.0
        payload:
          type: record
          name: User
          fields: []
";

#[cfg(test)]
async fn provide_asyncapi(ctx: &TestContext, producer: &str, yaml: &str) {
    let payload = json!({
        "producername": producer,
        "asyncapi_yaml": yaml,
        "stability": "ga",
    });
    let (status, _, body) = send(ctx, "POST", "/provide/asyncapi", Some(payload), &[]).await;
    assert_eq!(status, StatusCode::ACCEPTED, "got: {body}");
}

#[cfg(test)]
fn require_uri(producer: &str, channel: &str, format: Option<&str>) -> String {
    let mut uri = format!(
        "/require/asyncapi?consumername=cons&producername={producer}&version=1.0.0&path={channel}&method=PUB"
    );
    if let Some(format) = format {
        uri.push_str(&format!("&format={format}"));
    }
    uri
}

#[cfg(test)]
fn schema_names(yaml: &str) -> Vec<String> {
    let doc: sanshain_service::yaml::Value = sanshain_service::yaml::from_str(yaml).unwrap();
    doc["components"]["schemas"]
        .as_object()
        .unwrap()
        .keys()
        .map(|k| k.to_string())
        .collect()
}

#[tokio::test]
async fn single_require_with_format_openapi_answers_the_payload_schemas() {
    let ctx = setup().await;
    provide_asyncapi(&ctx, "events", EVENTS_V2).await;

    let (status, headers, body) = send(
        &ctx,
        "GET",
        &require_uri("events", "user.created", Some("openapi")),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    assert!(body.starts_with("openapi: \"3.0.3\"\n"), "got: {body}");
    assert!(
        body.contains("title: events message schemas"),
        "got: {body}"
    );
    assert!(body.contains("version: \"1.0.0\""), "got: {body}");
    assert_eq!(schema_names(&body), vec!["Address", "User"]);
    assert_eq!(headers.get("X-Sanshain-Version").unwrap(), "1.0.0");
}

#[tokio::test]
async fn single_require_without_format_is_unchanged_asyncapi() {
    let ctx = setup().await;
    provide_asyncapi(&ctx, "events", EVENTS_V2).await;

    let (status, _, body) = send(
        &ctx,
        "GET",
        &require_uri("events", "user.created", None),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    let doc: sanshain_service::yaml::Value = sanshain_service::yaml::from_str(&body).unwrap();
    assert!(doc.get("openapi").is_none(), "got: {body}");
    assert_eq!(doc["asyncapi"].as_str(), Some("2.6.0"));
}

#[tokio::test]
async fn the_two_formats_never_share_an_etag() {
    let ctx = setup().await;
    provide_asyncapi(&ctx, "events", EVENTS_V2).await;
    let native_uri = require_uri("events", "user.created", None);
    let openapi_uri = require_uri("events", "user.created", Some("openapi"));

    let (_, native, _) = send(&ctx, "GET", &native_uri, None, &[]).await;
    let (_, openapi, _) = send(&ctx, "GET", &openapi_uri, None, &[]).await;
    let native_etag = native.get("ETag").unwrap().to_str().unwrap().to_string();
    let openapi_etag = openapi.get("ETag").unwrap().to_str().unwrap().to_string();
    assert_ne!(native_etag, openapi_etag);

    // The native ETag does not validate the OpenAPI answer, its own does.
    let (status, _, body) = send(
        &ctx,
        "GET",
        &openapi_uri,
        None,
        &[("If-None-Match", native_etag.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    let (status, _, _) = send(
        &ctx,
        "GET",
        &openapi_uri,
        None,
        &[("If-None-Match", openapi_etag.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    let (status, _, _) = send(
        &ctx,
        "GET",
        &native_uri,
        None,
        &[("If-None-Match", openapi_etag.as_str())],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn bundle_with_format_openapi_merges_the_payload_schemas() {
    let ctx = setup().await;
    provide_asyncapi(&ctx, "events", EVENTS_V2).await;

    let payload = json!({
        "consumername": "cons",
        "producername": "events",
        "version": "1.0.0",
        "api_type": "asyncapi",
        "endpoints": [
            {"path": "user.created", "method": "PUB"},
            {"path": "order.placed", "method": "PUB"},
        ],
    });
    let (status, _, body) = send(
        &ctx,
        "POST",
        "/require-bundle?format=openapi",
        Some(payload.clone()),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    assert!(body.starts_with("openapi: \"3.0.3\"\n"), "got: {body}");
    assert_eq!(
        schema_names(&body),
        vec!["Address", "Order", "OrderPlacedPayload", "User"]
    );

    // Without the query parameter the bundle answer stays AsyncAPI.
    let (status, _, body) = send(&ctx, "POST", "/require-bundle", Some(payload), &[]).await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    assert!(body.contains("asyncapi: \"2.6.0\""), "got: {body}");
    assert!(!body.contains("openapi: "), "got: {body}");
}

#[tokio::test]
async fn format_on_another_api_type_or_an_unknown_format_is_a_bad_request() {
    let ctx = setup().await;
    provide_asyncapi(&ctx, "events", EVENTS_V2).await;

    let (status, _, body) = send(
        &ctx,
        "GET",
        &require_uri("events", "user.created", Some("json")),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body}");
    assert!(body.contains("Unknown format 'json'"), "got: {body}");

    let (status, _, body) = send(
        &ctx,
        "GET",
        "/require?consumername=cons&producername=events&version=1.0.0&path=/a&method=GET&format=openapi",
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body}");
    assert!(
        body.contains("only supported for apiType asyncapi"),
        "got: {body}"
    );

    // The bundle's api_type defaults to openapi, so format alone is a 400 too.
    let payload = json!({
        "consumername": "cons",
        "producername": "events",
        "version": "1.0.0",
        "endpoints": [{"path": "user.created", "method": "PUB"}],
    });
    let (status, _, body) = send(
        &ctx,
        "POST",
        "/require-bundle?format=openapi",
        Some(payload),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body}");
    assert!(body.contains("not openapi"), "got: {body}");
}

#[tokio::test]
async fn unconvertible_spec_is_a_bad_request_and_records_no_dependency() {
    let ctx = setup().await;
    provide_asyncapi(&ctx, "avro", AVRO_V2).await;

    let (status, _, body) = send(
        &ctx,
        "GET",
        &require_uri("avro", "user.created", Some("openapi")),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body}");
    assert!(body.contains("application/vnd.apache.avro"), "got: {body}");

    let (_, _, report) = send(&ctx, "GET", "/report", None, &[]).await;
    let report = as_json(&report);
    assert_eq!(
        report["dependency_graph"].as_array().unwrap().len(),
        0,
        "a failed conversion must not record a pin: {report}"
    );

    // The native answer of the same spec still works and records the pin.
    let (status, _, _) = send(
        &ctx,
        "GET",
        &require_uri("avro", "user.created", None),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, _, report) = send(&ctx, "GET", "/report", None, &[]).await;
    assert_eq!(
        as_json(&report)["dependency_graph"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
