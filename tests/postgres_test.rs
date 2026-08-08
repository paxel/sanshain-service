use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use chrono::Utc;
use sanshain_service::application::services;
use sanshain_service::domain::models::AuthMode;
use sanshain_service::infrastructure::cached_repository::CachedSpecRepository;
use sanshain_service::infrastructure::database::DatabaseRepo;
use sanshain_service::infrastructure::postgres_repository::PostgresSpecRepository;
use sanshain_service::{AppState, create_app};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio::sync::RwLock;
use tower::ServiceExt;

const TEST_CSRF_TOKEN: &str = "test-csrf-token";

/// Open the `ALLOW_INSECURE_DEV_MODE` safety gate once for this test binary,
/// which exercises the dev-mode auth bypass. Set exactly once, synchronized.
fn ensure_dev_mode_gate_open() {
    use std::sync::Once;
    static GATE: Once = Once::new();
    GATE.call_once(|| unsafe {
        std::env::set_var("ALLOW_INSECURE_DEV_MODE", "true");
    });
}

fn test_app_state(repo: PostgresSpecRepository, db_url: String) -> AppState {
    ensure_dev_mode_gate_open();
    let mut tokens = HashMap::new();
    tokens.insert(
        TEST_CSRF_TOKEN.to_string(),
        Utc::now() + chrono::Duration::hours(1),
    );
    let (spec_updated_tx, _) = tokio::sync::broadcast::channel(100);

    let prometheus_handle = {
        let (_, handle) = axum_prometheus::PrometheusMetricLayer::pair();
        handle
    };

    AppState {
        repo: CachedSpecRepository::new(DatabaseRepo::Postgres(repo), 256),
        db_url,
        dev_user: None,
        csrf_tokens: Arc::new(RwLock::new(tokens)),
        instance_id: "test-postgres".to_string(),
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
        prometheus_handle,
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

#[tokio::test]
async fn test_postgres_full_flow_with_testcontainers() {
    // 1. Start Postgres container
    let postgres_container = Postgres::default().start().await.unwrap();
    let host = postgres_container.get_host().await.unwrap();
    let port = postgres_container.get_host_port_ipv4(5432).await.unwrap();
    let db_url = format!("postgres://postgres:postgres@{}:{}/postgres", host, port);

    // 2. Setup database and repo
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();

    let repo = PostgresSpecRepository::new(pool);
    repo.run_migrations()
        .await
        .expect("Failed to run PostgreSQL migrations");

    // 3. Setup app
    services::ensure_initial_admin(&repo).await.unwrap();
    services::set_auth_mode(&repo, &AuthMode::Dev)
        .await
        .unwrap();
    let dev_user = services::ensure_dev_user(&repo).await.ok();
    let mut state = test_app_state(repo.clone(), db_url);
    state.dev_user = dev_user;
    let app = create_app(state);

    // 4. Run simple flow (Provide -> Require)
    let openapi_yaml = r#"
openapi: 3.0.0
info:
  title: Postgres Test API
  version: 1.0.0
paths:
  /test:
    get:
      responses:
        '200':
          description: OK
"#;

    let provide_payload = json!({
        "producername": "pg-service",
        "stability": "ga",
        "openapi_yaml": openapi_yaml
    });

    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .header("X-CSRF-Token", TEST_CSRF_TOKEN)
                .body(Body::from(serde_json::to_vec(&provide_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::ACCEPTED);

    let response: Response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require?consumername=pg-client&producername=pg-service&version=1.0.0&path=/test&method=GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 10000)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("Postgres Test API"));
}

/// The compare-and-set contract of the upsert, driven against the real
/// Postgres statement: the SQLite twin's module tests cannot vouch for this
/// copy of the guard.
#[tokio::test]
async fn test_postgres_upsert_compare_and_set_contract() {
    use sanshain_service::domain::models::{ApiType, EndpointRecord, Stability};
    use sanshain_service::domain::ports::{RepositoryError, SpecRepository, UpsertSpecVersion};

    let postgres_container = Postgres::default().start().await.unwrap();
    let host = postgres_container.get_host().await.unwrap();
    let port = postgres_container.get_host_port_ipv4(5432).await.unwrap();
    let db_url = format!("postgres://postgres:postgres@{}:{}/postgres", host, port);
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();
    let repo = PostgresSpecRepository::new(pool);
    repo.run_migrations()
        .await
        .expect("Failed to run PostgreSQL migrations");

    fn endpoint(marker: &str) -> EndpointRecord {
        EndpointRecord {
            id: None,
            api_type: ApiType::OpenApi,
            path: format!("/{marker}"),
            normalized_path: format!("/{marker}"),
            method: "GET".to_string(),
            yaml_content: marker.to_string(),
            deprecated: false,
        }
    }

    let sid = repo.ensure_service("cas-svc").await.unwrap();
    let version: sanshain_service::domain::models::SemVer = "1.0.0".parse().unwrap();

    // Seed the snapshot an ordinary provide would store.
    repo.upsert_spec_version(UpsertSpecVersion {
        service_id: sid,
        api_type: ApiType::OpenApi,
        version,
        stability: Stability::Snapshot,
        content: "snapshot content",
        content_hash: "sha256:snapshot",
        provided_by: "alice",
        expected_prior_hash: None,
        now_iso: "2026-01-01T00:00:00Z",
        endpoints: vec![endpoint("a")],
    })
    .await
    .expect("seeding the snapshot lands");

    // A stale expected hash means the row moved since the caller read it.
    let err = repo
        .upsert_spec_version(UpsertSpecVersion {
            service_id: sid,
            api_type: ApiType::OpenApi,
            version,
            stability: Stability::Ga,
            content: "stale content",
            content_hash: "sha256:stale-read",
            provided_by: "releaser",
            expected_prior_hash: Some("sha256:stale"),
            now_iso: "2026-01-02T00:00:00Z",
            endpoints: vec![endpoint("b")],
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RepositoryError::Conflict));
    let meta = repo
        .find_spec_version(sid, ApiType::OpenApi, version)
        .await
        .unwrap()
        .expect("the row is untouched");
    assert_eq!(meta.stability, Stability::Snapshot, "nothing was released");
    assert_eq!(meta.content_hash, "sha256:snapshot");

    // The matching hash releases in place, keeping the row's identity.
    repo.upsert_spec_version(UpsertSpecVersion {
        service_id: sid,
        api_type: ApiType::OpenApi,
        version,
        stability: Stability::Ga,
        content: "snapshot content",
        content_hash: "sha256:snapshot",
        provided_by: "releaser",
        expected_prior_hash: Some("sha256:snapshot"),
        now_iso: "2026-01-03T00:00:00Z",
        endpoints: vec![endpoint("a")],
    })
    .await
    .expect("matching hash lands");
    let meta = repo
        .find_spec_version(sid, ApiType::OpenApi, version)
        .await
        .unwrap()
        .expect("the released row");
    assert_eq!(meta.stability, Stability::Ga);

    // Once GA, the row is immutable even against a plain overwrite.
    let err = repo
        .upsert_spec_version(UpsertSpecVersion {
            service_id: sid,
            api_type: ApiType::OpenApi,
            version,
            stability: Stability::Snapshot,
            content: "other",
            content_hash: "sha256:other",
            provided_by: "racer",
            expected_prior_hash: None,
            now_iso: "2026-01-04T00:00:00Z",
            endpoints: vec![endpoint("c")],
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RepositoryError::Conflict));

    // A CAS caller whose row vanished must not resurrect it as a fresh GA.
    let gone: sanshain_service::domain::models::SemVer = "2.0.0".parse().unwrap();
    let err = repo
        .upsert_spec_version(UpsertSpecVersion {
            service_id: sid,
            api_type: ApiType::OpenApi,
            version: gone,
            stability: Stability::Ga,
            content: "content",
            content_hash: "sha256:x",
            provided_by: "releaser",
            expected_prior_hash: Some("sha256:x"),
            now_iso: "2026-01-05T00:00:00Z",
            endpoints: vec![endpoint("d")],
        })
        .await
        .unwrap_err();
    assert!(matches!(err, RepositoryError::Conflict));
    assert!(
        repo.find_spec_version(sid, ApiType::OpenApi, gone)
            .await
            .unwrap()
            .is_none(),
        "nothing was resurrected"
    );
}

/// ADR-0004: trunk pins are append-only on PostgreSQL exactly as on SQLite —
/// a re-pin closes the open record and inserts, an identical re-pin only
/// refreshes, and the current view is the open rows.
#[tokio::test]
async fn test_postgres_trunk_pins_append_only() {
    use sanshain_service::domain::models::{ApiType, SemVer};
    use sanshain_service::domain::ports::{RecordTrunkPinParams, SpecRepository};

    let postgres_container = Postgres::default().start().await.unwrap();
    let host = postgres_container.get_host().await.unwrap();
    let port = postgres_container.get_host_port_ipv4(5432).await.unwrap();
    let db_url = format!("postgres://postgres:postgres@{}:{}/postgres", host, port);
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();
    let repo = PostgresSpecRepository::new(pool.clone());
    repo.run_migrations().await.expect("migrations");

    let service_id = repo.ensure_service("svc").await.unwrap();
    let client_id = repo.ensure_client("webapp").await.unwrap();
    let pin = |version: SemVer, now: &'static str| RecordTrunkPinParams {
        client_id,
        service_id,
        api_type: ApiType::OpenApi,
        version,
        path: "/users",
        normalized_path: "/users",
        method: "GET",
        now_iso: now,
    };

    repo.record_trunk_pins(vec![pin(SemVer::new(1, 0, 0), "2026-08-07T10:00:00Z")])
        .await
        .unwrap();
    repo.record_trunk_pins(vec![pin(SemVer::new(1, 1, 0), "2026-08-07T11:00:00Z")])
        .await
        .unwrap();
    // Identical re-pin: refresh only.
    repo.record_trunk_pins(vec![pin(SemVer::new(1, 1, 0), "2026-08-07T12:00:00Z")])
        .await
        .unwrap();

    let current = repo.list_current_trunk_pins().await.unwrap();
    assert_eq!(current.len(), 1, "got: {current:?}");
    assert_eq!(current[0].client, "webapp");
    assert_eq!(current[0].service, "svc");
    assert_eq!(current[0].version, SemVer::new(1, 1, 0));
    assert_eq!(current[0].last_required_at, "2026-08-07T12:00:00Z");
    assert_eq!(current[0].valid_from, "2026-08-07T11:00:00Z");

    let rows: Vec<(Option<String>, i32)> =
        sqlx::query_as("SELECT valid_to, minor FROM trunk_dependencies ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(rows.len(), 2, "append-only: close + insert, got {rows:?}");
    assert_eq!(rows[0], (Some("2026-08-07T11:00:00Z".to_string()), 0));
    assert_eq!(rows[1], (None, 1));
}

#[tokio::test]
async fn postgres_audit_stream_filter_is_bound() {
    use sanshain_service::domain::models::AuditLogFilter;
    use sanshain_service::domain::ports::{NewAuditLog, SpecRepository};

    let postgres_container = Postgres::default().start().await.unwrap();
    let host = postgres_container.get_host().await.unwrap();
    let port = postgres_container.get_host_port_ipv4(5432).await.unwrap();
    let db_url = format!("postgres://postgres:postgres@{}:{}/postgres", host, port);
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();
    let repo = PostgresSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    for stream in [Some("trunk"), None] {
        repo.insert_audit_log(
            "tester",
            NewAuditLog {
                action: "TRUNK_PIN",
                details: "d",
                service: None,
                version: None,
                action_type: Some("WRITE"),
                diff: None,
                stream,
                branch_id: None,
            },
        )
        .await
        .unwrap();
    }

    // The stream predicate must bind its value; before the fix the query had
    // one more placeholder than bound parameters and answered an error.
    let logs = repo
        .get_audit_logs(AuditLogFilter {
            from_date: None,
            to_date: None,
            action_type: None,
            service_wildcard: None,
            version_wildcard: None,
            stream: Some("trunk".to_string()),
            branch_id: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].stream.as_deref(), Some("trunk"));
}
