//! The statistics page's routes (san-1021 and its tabs): who reaches them,
//! what the range does, and that no answer carries a person's name.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::Utc;
use sanshain_service::application::services;
use sanshain_service::domain::ports::SpecRepository;
use sanshain_service::infrastructure::cached_repository::CachedSpecRepository;
use sanshain_service::infrastructure::database::DatabaseRepo;
use sanshain_service::infrastructure::sqlite_repository::SqliteSpecRepository;
use sanshain_service::{AppState, create_app};
use serde_json::Value;
use sqlx::sqlite::SqlitePoolOptions;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64};
use tokio::sync::RwLock;
use tower::ServiceExt;

const TEST_CSRF_TOKEN: &str = "statistics-csrf-token";

/// The name that must never appear in a statistics answer.
const PERSON: &str = "zelda-quixote";

static TEST_PROMETHEUS_HANDLE: OnceLock<metrics_exporter_prometheus::PrometheusHandle> =
    OnceLock::new();

#[cfg(test)]
fn prometheus_handle() -> metrics_exporter_prometheus::PrometheusHandle {
    TEST_PROMETHEUS_HANDLE
        .get_or_init(|| {
            let (_, handle) = axum_prometheus::PrometheusMetricLayer::pair();
            handle
        })
        .clone()
}

#[cfg(test)]
fn test_state(repo: SqliteSpecRepository) -> AppState {
    let mut tokens = HashMap::new();
    tokens.insert(
        TEST_CSRF_TOKEN.to_string(),
        Utc::now() + chrono::Duration::hours(1),
    );
    let (spec_updated_tx, _) = tokio::sync::broadcast::channel(100);
    let system = sysinfo::System::new_all().ok();
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
        prometheus_handle: prometheus_handle(),
        system: Arc::new(std::sync::Mutex::new(system)),
        max_body_bytes: sanshain_service::DEFAULT_MAX_BODY_BYTES,
        snapshot_in_progress: Arc::new(AtomicBool::new(false)),
        mcp: Default::default(),
        directory_roles: sanshain_service::application::directory_roles::DirectoryRoleCache::new(
            std::time::Duration::from_secs(300),
        ),
        root_users: Arc::new(sanshain_service::domain::permissions::RootUsers::resolve(
            None, None,
        )),
    }
}

struct World {
    app: axum::Router,
    admin: String,
    report_viewer: String,
    viewer: String,
    plain: String,
}

#[cfg(test)]
async fn world() -> World {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("in-memory database");
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.expect("migrations");
    let hash = services::hash_password("pw").expect("hash");
    let mut tokens = Vec::new();
    for (name, role) in [
        ("admin", Some("admin")),
        ("reports", Some("report_viewer")),
        ("auditor", Some("viewer")),
        (PERSON, None),
    ] {
        let user = repo.create_user(name, &hash, true).await.expect("user");
        if let Some(role) = role {
            repo.grant_user_role(user.id, role).await.expect("grant");
        }
        let session = repo
            .create_session(user.id, "2099-12-31T23:59:59")
            .await
            .expect("session");
        tokens.push(session.token);
    }
    let app = create_app(test_state(repo.clone()));
    World {
        app,
        admin: tokens[0].clone(),
        report_viewer: tokens[1].clone(),
        viewer: tokens[2].clone(),
        plain: tokens[3].clone(),
    }
}

/// Status and JSON body of a call.
#[cfg(test)]
async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("Authorization", format!("Bearer {token}"))
        .header("X-CSRF-Token", TEST_CSRF_TOKEN)
        .header("Content-Type", "application/json")
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .expect("request");
    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// A trunk provide of a one-endpoint spec by [`PERSON`], over HTTP so the
/// audit log has it.
#[cfg(test)]
async fn provide(w: &World, producer: &str, version: &str) {
    let spec = format!(
        "openapi: 3.0.0\ninfo: {{ title: T, version: {version} }}\npaths:\n  /a:\n    get:\n      responses:\n        \"200\": {{ description: ok }}\n"
    );
    let (status, body) = call(
        &w.app,
        "POST",
        "/provide",
        &w.plain,
        Some(serde_json::json!({
            "producername": producer,
            "stability": "snapshot",
            "trunk": true,
            "openapi_yaml": spec,
        })),
    )
    .await;
    assert!(status.is_success(), "provide {producer}: {status} {body}");
}

/// A trunk require of `/a` by [`PERSON`].
#[cfg(test)]
async fn require(w: &World, consumer: &str, producer: &str, version: &str) {
    let uri = format!(
        "/require?consumername={consumer}&producername={producer}&version={version}&path=/a&method=GET&trunk=true"
    );
    let (status, body) = call(&w.app, "GET", &uri, &w.plain, None).await;
    assert!(status.is_success(), "require: {status} {body}");
}

/// No answer may carry a person's name.
#[cfg(test)]
fn assert_no_person(body: &Value) {
    let text = body.to_string();
    assert!(!text.contains(PERSON), "a person's name leaked: {text}");
}

/// Status and JSON body of a GET.
#[cfg(test)]
async fn get(app: &axum::Router, uri: &str, token: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(uri)
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("request");
    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn statistics_need_view_statistics() {
    let w = world().await;
    let uri = "/admin/statistics/no-such-tab";
    assert_eq!(get(&w.app, uri, &w.plain).await.0, StatusCode::FORBIDDEN);
    // view_audit is not enough: the statistics are their own permission.
    assert_eq!(get(&w.app, uri, &w.viewer).await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        get(&w.app, uri, &w.report_viewer).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(get(&w.app, uri, &w.admin).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_broken_range_is_a_bad_request() {
    let w = world().await;
    let (status, body) = get(
        &w.app,
        "/admin/statistics/no-such-tab?from=2026-02-01&to=2026-01-01",
        &w.report_viewer,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("before it starts"),
        "{body}"
    );
}

#[tokio::test]
async fn landscape_counts_what_exists_and_what_is_active() {
    let w = world().await;
    provide(&w, "orders", "1.0.0").await;
    provide(&w, "billing", "1.0.0").await;
    require(&w, "shop", "orders", "1.0.0").await;
    let (status, body) = get(&w.app, "/admin/statistics/landscape", &w.report_viewer).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["tab"], "landscape");
    let data = &body["data"];
    assert_eq!(data["producers"], serde_json::json!(["billing", "orders"]));
    assert_eq!(data["consumers"], serde_json::json!(["shop"]));
    assert_eq!(
        data["active_in_range"],
        serde_json::json!(["billing", "orders", "shop"])
    );
    assert_eq!(
        data["on_main"],
        serde_json::json!(["billing", "orders", "shop"])
    );
    assert_eq!(
        data["api_types"],
        serde_json::json!([{ "label": "openapi", "count": 2 }])
    );
    assert_eq!(
        data["flows"],
        serde_json::json!([{ "from": "no domain", "to": "no domain", "pairs": 1 }])
    );
    assert_eq!(data["domains"][0]["name"], "no domain");
    assert_eq!(data["capped_months"], serde_json::json!([]));
    assert_no_person(&body);

    // A range in the past has no activity, the rest stays.
    let (_, old) = get(
        &w.app,
        "/admin/statistics/landscape?from=2020-01-01&to=2020-01-31",
        &w.report_viewer,
    )
    .await;
    assert_eq!(old["data"]["active_in_range"], serde_json::json!([]));
    assert_eq!(
        old["data"]["producers"],
        serde_json::json!(["billing", "orders"])
    );
}

/// A catalog push naming `owner` for `system`.
#[cfg(test)]
async fn push_owner(w: &World, system: &str, owner: &str) {
    let yaml = format!(
        "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: {system}\n  tags: [web]\nspec:\n  type: service\n  owner: {owner}\n"
    );
    let (status, body) = call(
        &w.app,
        "PUT",
        &format!("/catalog/{system}"),
        &w.plain,
        Some(serde_json::json!({ "catalog_info_yaml": yaml, "trunk": true })),
    )
    .await;
    assert!(status.is_success(), "catalog {system}: {status} {body}");
}

#[tokio::test]
async fn activity_counts_calls_per_owner_and_never_names_a_person() {
    let w = world().await;
    push_owner(&w, "orders", "group:payments").await;
    push_owner(&w, "shop", &format!("user:{PERSON}")).await;
    provide(&w, "orders", "1.0.0").await;
    require(&w, "shop", "orders", "1.0.0").await;
    require(&w, "shop", "orders", "1.0.0").await;
    // A refusal: different content under the admin's own snapshot is fine, so
    // the refusal comes from another user changing PERSON's snapshot.
    let changed = "openapi: 3.0.0\ninfo: { title: T, version: 1.0.0 }\npaths:\n  /b:\n    get:\n      responses:\n        \"200\": { description: ok }\n";
    let (status, _) = call(
        &w.app,
        "POST",
        "/provide",
        &w.admin,
        Some(serde_json::json!({
            "producername": "orders",
            "stability": "snapshot",
            "openapi_yaml": changed,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, body) = get(&w.app, "/admin/statistics/activity", &w.report_viewer).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let data = &body["data"];
    assert_eq!(data["periods"]["bucket"], "day");
    assert_eq!(data["periods"]["starts"].as_array().map(Vec::len), Some(90));
    let total = |label: &str| -> u64 {
        data["calls"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["label"] == label)
            .unwrap()["counts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_u64().unwrap())
            .sum()
    };
    assert_eq!(total("Provides"), 1);
    assert_eq!(total("Requires"), 2);
    assert_eq!(total("Catalog pushes"), 2);
    let refused: u64 = data["refusals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_u64().unwrap())
        .sum();
    assert_eq!(refused, 1);
    assert_eq!(
        data["owners"],
        serde_json::json!([
            { "owner": "no owner", "provides": 0, "requires": 2, "catalog_pushes": 1, "refusals": 0 },
            { "owner": "payments", "provides": 1, "requires": 0, "catalog_pushes": 1, "refusals": 1 },
        ])
    );
    assert_eq!(data["top_systems"][0]["system"], "shop");
    assert_eq!(data["top_systems"][0]["calls"], 3);
    assert_no_person(&body);

    // Twelve months are counted per week.
    let (_, year) = get(
        &w.app,
        "/admin/statistics/activity?from=2026-01-01&to=2026-12-31",
        &w.report_viewer,
    )
    .await;
    assert_eq!(year["data"]["periods"]["bucket"], "week");
}
