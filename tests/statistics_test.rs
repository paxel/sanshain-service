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
