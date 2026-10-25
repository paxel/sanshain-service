//! How a refused request is logged.
//!
//! Its own test binary on purpose: the test captures the log with a
//! thread-local subscriber, and tracing caches per log statement, for the whole
//! process, whether anyone listens. A test running in parallel that reaches the
//! same statement first can cache "nobody listens" and leave the capture empty.
//! Alone in its process, the test registers its subscriber before any other
//! thread can get there.

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
use sqlx::sqlite::SqlitePoolOptions;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64};
use tokio::sync::RwLock;
use tower::ServiceExt;

const TEST_CSRF_TOKEN: &str = "refusal-logging-csrf-token";

static TEST_PROMETHEUS_HANDLE: OnceLock<metrics_exporter_prometheus::PrometheusHandle> =
    OnceLock::new();

// `cfg(test)` is always true in this crate; the attribute marks these helpers as
// test code so clippy's `allow-*-in-tests` exemptions apply to them.
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
    let mut system = sysinfo::System::new_all();
    system.refresh_all();

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
            Some("breakglass"),
            None,
        )),
    }
}

#[cfg(test)]
struct World {
    app: axum::Router,
    plain: String,
}

/// An app with one signed-in user, `plain`, who holds no role.
#[cfg(test)]
async fn world() -> World {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("in-memory database");
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.expect("migrations");
    let hash = services::hash_password("pw").expect("hash");
    let user = repo
        .create_user("plain", &hash, true)
        .await
        .expect("create plain");
    let session = repo
        .create_session(user.id, "2099-12-31T23:59:59")
        .await
        .expect("session plain");
    World {
        app: create_app(test_state(repo)),
        plain: session.token,
    }
}

#[cfg(test)]
async fn status(app: &axum::Router, method: &str, uri: &str, token: &str) -> StatusCode {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("Authorization", format!("Bearer {}", token))
        .header("X-CSRF-Token", TEST_CSRF_TOKEN)
        .header("Content-Type", "application/json")
        .body(Body::from("{}"))
        .expect("request");
    app.clone()
        .oneshot(request)
        .await
        .expect("response")
        .status()
}

/// A writer the test reads back: every log line the request produced.
#[derive(Clone, Default)]
struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Ok(mut inner) = self.0.lock() {
            inner.extend_from_slice(buf);
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Captured;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// A refused request is logged with method, path and caller, never with the
/// query string or a credential, so an operator can trace it.
#[tokio::test]
async fn a_refusal_is_logged_with_method_path_and_caller() {
    let w = world().await;
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let uri = "/admin/settings/snapshot-max-age?secret=x";
    assert_eq!(
        status(&w.app, "POST", uri, &w.plain).await,
        StatusCode::FORBIDDEN
    );
    let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(
        log.contains("Client error: Forbidden (POST /admin/settings/snapshot-max-age, user plain)"),
        "{log}"
    );
    assert!(!log.contains("secret=x"), "{log}");
    assert!(
        !log.contains(&w.plain),
        "the token must never be logged: {log}"
    );
}
