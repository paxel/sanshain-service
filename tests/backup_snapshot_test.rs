//! The SQLite snapshot download (#50) through the real router, against an
//! on-disk database so `VACUUM INTO` has a real data directory to work in.
//!
//! A good test downloads the snapshot and opens it as a database: the copy
//! must carry the same Producers and Versions, the temporary file must be
//! gone afterwards (also when the client walks away without reading the
//! body), only one snapshot may run at a time, and every download is audited.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::Utc;
use sanshain_service::application::services;
use sanshain_service::domain::models::{ApiTokenScope, ApiType, Stability};
use sanshain_service::domain::ports::SpecRepository;
use sanshain_service::infrastructure::cached_repository::CachedSpecRepository;
use sanshain_service::infrastructure::database::DatabaseRepo;
use sanshain_service::infrastructure::sqlite_repository::SqliteSpecRepository;
use sanshain_service::{AppState, create_app};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::str::FromStr;
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

fn test_state(repo: SqliteSpecRepository, db_url: String) -> AppState {
    let (spec_updated_tx, _) = tokio::sync::broadcast::channel(16);
    AppState {
        repo: CachedSpecRepository::new(DatabaseRepo::Sqlite(repo), 64),
        db_url,
        dev_user: None,
        csrf_tokens: Arc::new(RwLock::new(HashMap::new())),
        instance_id: "test".into(),
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
            Some("__unused-root__"),
            None,
        )),
    }
}

/// An on-disk SQLite database in a directory of its own, so the test can see
/// exactly which files the snapshot leaves behind. Removed on drop.
struct DataDir {
    dir: PathBuf,
}

// `cfg(test)` is always true in this crate; the attribute marks the helpers as
// test code for clippy's `allow-expect-in-tests`.
#[cfg(test)]
impl DataDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "sanshain-backup-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create data dir");
        Self { dir }
    }

    fn db_url(&self) -> String {
        format!("sqlite:{}?mode=rwc", self.dir.join("sanshain.db").display())
    }

    /// Every file in the directory except the database and its WAL/shm.
    fn stray_files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.dir)
            .expect("read data dir")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with("sanshain.db"))
            .collect();
        names.sort();
        names
    }
}

impl Drop for DataDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn spec(version: &str) -> String {
    format!(
        "openapi: 3.0.0\ninfo:\n  title: t\n  version: {version}\npaths:\n  /a:\n    get:\n      responses:\n        '200': {{ description: ok }}\n"
    )
}

#[cfg(test)]
async fn provide(repo: &SqliteSpecRepository, producer: &str, version: &str) {
    services::provide_spec(
        repo,
        services::ProvideSpecParams {
            producername: producer,
            api_type: ApiType::OpenApi,
            content: &spec(version),
            stability: Stability::Ga,
            dry_run: false,
            trunk: false,
            tag: None,
            caller: Some(sanshain_service::domain::permissions::Actor::test_releaser()),
            require_prior_content_match: false,
        },
    )
    .await
    .expect("provide");
}

struct World {
    app: axum::Router,
    repo: SqliteSpecRepository,
    admin: String,
    plain: String,
    full_token: String,
    read_only_token: String,
}

#[cfg(test)]
async fn open(url: &str) -> SqlitePool {
    let options = SqliteConnectOptions::from_str(url)
        .expect("db url")
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
    SqlitePoolOptions::new()
        .max_connections(3)
        .connect_with(options)
        .await
        .expect("open database")
}

#[cfg(test)]
async fn world(data: &DataDir) -> World {
    let repo = SqliteSpecRepository::new(open(&data.db_url()).await);
    repo.run_migrations().await.expect("migrations");

    let hash = services::hash_password("pw").expect("hash");
    let admin = repo.create_user("admin", &hash, true).await.expect("admin");
    repo.grant_user_role(admin.id, "admin")
        .await
        .expect("grant admin");
    let plain = repo.create_user("plain", &hash, true).await.expect("plain");
    let admin_session = repo
        .create_session(admin.id, "2099-12-31T23:59:59")
        .await
        .expect("admin session");
    let plain_session = repo
        .create_session(plain.id, "2099-12-31T23:59:59")
        .await
        .expect("plain session");
    let (_, full_token) =
        services::create_api_token(&repo, admin.id, "cron", 30, ApiTokenScope::Full)
            .await
            .expect("full token");
    let (_, read_only_token) =
        services::create_api_token(&repo, admin.id, "ai", 30, ApiTokenScope::ReadOnly)
            .await
            .expect("read-only token");

    provide(&repo, "orders", "1.0.0").await;
    provide(&repo, "orders", "1.1.0").await;
    provide(&repo, "billing", "2.0.0").await;

    World {
        app: create_app(test_state(repo.clone(), data.db_url())),
        repo,
        admin: admin_session.token,
        plain: plain_session.token,
        full_token,
        read_only_token,
    }
}

#[cfg(test)]
fn snapshot_request(token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri("/admin/backup/sqlite")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("request")
}

#[cfg(test)]
async fn names_and_version_count(pool: &SqlitePool) -> (Vec<String>, i64) {
    let names: Vec<String> = sqlx::query_scalar("SELECT name FROM services ORDER BY name")
        .fetch_all(pool)
        .await
        .expect("services");
    let versions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM spec_versions")
        .fetch_one(pool)
        .await
        .expect("versions");
    (names, versions)
}

#[tokio::test]
async fn the_snapshot_is_a_database_with_the_same_producers_and_versions() {
    let data = DataDir::new("roundtrip");
    let w = world(&data).await;

    let response = w
        .app
        .clone()
        .oneshot(snapshot_request(&w.admin))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/vnd.sqlite3")
    );
    let disposition = response
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .expect("content-disposition")
        .to_string();
    let expected_prefix = format!(
        "attachment; filename=\"sanshain-{}-",
        env!("CARGO_PKG_VERSION")
    );
    assert!(
        disposition.starts_with(&expected_prefix) && disposition.ends_with("Z.db\""),
        "filename carries the instance version and a UTC timestamp: {disposition}"
    );
    let declared_length: u64 = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .expect("content-length");

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(bytes.len() as u64, declared_length);
    assert!(
        bytes.starts_with(b"SQLite format 3\0"),
        "the body is a SQLite database file"
    );

    // Opening the download as a database is the real assertion.
    let copy = std::env::temp_dir().join(format!(
        "sanshain-backup-test-{}-roundtrip-copy.db",
        std::process::id()
    ));
    std::fs::write(&copy, &bytes).expect("write copy");
    let copied = open(&format!("sqlite:{}?mode=rw", copy.display())).await;
    let (copied_names, copied_versions) = names_and_version_count(&copied).await;
    let (source_names, source_versions) = names_and_version_count(&w.repo.pool).await;
    copied.close().await;
    let _ = std::fs::remove_file(&copy);
    assert_eq!(
        copied_names,
        vec!["billing".to_string(), "orders".to_string()]
    );
    assert_eq!(copied_names, source_names);
    assert_eq!(copied_versions, 3);
    assert_eq!(copied_versions, source_versions);

    // Nothing is left behind in the data directory.
    assert_eq!(
        data.stray_files(),
        Vec::<String>::new(),
        "the temporary snapshot file must be removed after streaming"
    );

    // Every download is audited with the Actor and the size.
    let logs = w.repo.get_recent_audit_logs(10).await.expect("audit logs");
    let entry = logs
        .iter()
        .find(|l| l.action == "DATABASE_SNAPSHOT")
        .expect("a DATABASE_SNAPSHOT audit entry");
    assert_eq!(entry.username, "admin");
    assert!(
        entry.details.contains(&format!("{declared_length} bytes")),
        "the audit entry names the size: {}",
        entry.details
    );
}

#[tokio::test]
async fn a_read_only_token_and_a_user_without_the_permission_are_refused() {
    let data = DataDir::new("refusals");
    let w = world(&data).await;

    for (who, token) in [
        ("plain user", &w.plain),
        ("read-only token", &w.read_only_token),
    ] {
        let response = w
            .app
            .clone()
            .oneshot(snapshot_request(token))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{who}");
    }

    // The scheduled backup authenticates with a full-scope token of an account
    // holding the permission.
    let response = w
        .app
        .clone()
        .oneshot(snapshot_request(&w.full_token))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK, "full token of an admin");
    drop(response);

    let logs = w.repo.get_recent_audit_logs(10).await.expect("audit logs");
    assert_eq!(
        logs.iter()
            .filter(|l| l.action == "DATABASE_SNAPSHOT")
            .count(),
        1,
        "only the successful download is audited"
    );
    assert_eq!(data.stray_files(), Vec::<String>::new());
}

#[tokio::test]
async fn only_one_snapshot_runs_at_a_time() {
    let data = DataDir::new("concurrent");
    let w = world(&data).await;

    let (first, second) = tokio::join!(
        w.app.clone().oneshot(snapshot_request(&w.admin)),
        w.app.clone().oneshot(snapshot_request(&w.admin)),
    );
    let first = first.expect("first response");
    let second = second.expect("second response");
    let mut statuses = [first.status(), second.status()];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::OK, StatusCode::CONFLICT],
        "one succeeds, the other is told a snapshot is already running"
    );

    // The lock is released with the finished download, not leaked.
    drop(first);
    drop(second);
    let third = w
        .app
        .clone()
        .oneshot(snapshot_request(&w.admin))
        .await
        .expect("third response");
    assert_eq!(third.status(), StatusCode::OK);
    let _ = axum::body::to_bytes(third.into_body(), usize::MAX).await;
    assert_eq!(data.stray_files(), Vec::<String>::new());
}

#[tokio::test]
async fn an_abandoned_download_still_removes_the_temporary_file() {
    let data = DataDir::new("abandoned");
    let w = world(&data).await;

    let response = w
        .app
        .clone()
        .oneshot(snapshot_request(&w.admin))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        !data.stray_files().is_empty(),
        "the snapshot exists while the body is unread"
    );
    // The client disconnects before reading the body.
    drop(response);
    assert_eq!(
        data.stray_files(),
        Vec::<String>::new(),
        "dropping the response must remove the temporary file"
    );
}

/// Hygiene for the helper: the stray-file check sees what it is meant to.
#[test]
fn data_dir_lists_files_beside_the_database() {
    let data = DataDir::new("hygiene");
    std::fs::write(data.dir.join("sanshain.db"), b"x").expect("db");
    std::fs::write(data.dir.join("sanshain.db-wal"), b"x").expect("wal");
    assert_eq!(data.stray_files(), Vec::<String>::new());
    std::fs::write(data.dir.join("leftover.tmp"), b"x").expect("tmp");
    assert_eq!(data.stray_files(), vec!["leftover.tmp".to_string()]);
    assert!(Path::new(&data.dir).is_dir());
}
