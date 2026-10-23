#![expect(
    unsafe_code,
    reason = "edition 2024: std::env::set_var/remove_var are unsafe"
)]

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
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, OnceLock};

mod common;
#[path = "common/credential_expiry.rs"]
mod credential_expiry;
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

    // `pair()` installs the process-wide metrics recorder and panics on a
    // second call, so every test in this binary shares one handle.
    static PROMETHEUS_HANDLE: OnceLock<metrics_exporter_prometheus::PrometheusHandle> =
        OnceLock::new();
    let prometheus_handle = PROMETHEUS_HANDLE
        .get_or_init(|| {
            let (_, handle) = axum_prometheus::PrometheusMetricLayer::pair();
            handle
        })
        .clone();

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

#[tokio::test]
async fn test_postgres_full_flow_with_testcontainers() {
    // 1. Start Postgres container
    let postgres_container = common::start_postgres().await;
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
    services::ensure_initial_admin(&repo, "http://127.0.0.1:3000/admin.html")
        .await
        .unwrap();
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
/// #50: the snapshot download is SQLite-only. On PostgreSQL the route must
/// refuse in a way nobody mistakes for a backup, naming the tool to use.
#[tokio::test]
async fn snapshot_download_refuses_on_postgres_and_names_pg_dump() {
    let postgres_container = common::start_postgres().await;
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

    services::ensure_initial_admin(&repo, "http://127.0.0.1:3000/admin.html")
        .await
        .unwrap();
    services::set_auth_mode(&repo, &AuthMode::Dev)
        .await
        .unwrap();
    let dev_user = services::ensure_dev_user(&repo).await.ok();
    // Dev mode opens the API, not the administration: the backup route still
    // needs a signed-in administrator.
    let admin = services::list_users(&repo)
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let session = sanshain_service::domain::ports::SpecRepository::create_session(
        &repo,
        admin.id,
        "2999-01-01T00:00:00Z",
    )
    .await
    .unwrap();
    let mut state = test_app_state(repo.clone(), db_url);
    state.dev_user = dev_user;
    let app = create_app(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/backup/sqlite")
                .header("Authorization", format!("Bearer {}", session.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let message = body["error"].as_str().unwrap_or_default();
    assert!(
        message.contains("PostgreSQL") && message.contains("pg_dump"),
        "the refusal names the engine and the tool: {message}"
    );
}

#[tokio::test]
async fn test_postgres_upsert_compare_and_set_contract() {
    use sanshain_service::domain::models::{ApiType, EndpointRecord, Stability};
    use sanshain_service::domain::ports::{RepositoryError, SpecRepository, UpsertSpecVersion};

    let postgres_container = common::start_postgres().await;
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
        outbox: Vec::new(),
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
            outbox: Vec::new(),
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
        outbox: Vec::new(),
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
            outbox: Vec::new(),
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
            outbox: Vec::new(),
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

/// ADR-0008: the one-time normalization rewrites CRLF/BOM text stored by an
/// earlier release in place on PostgreSQL — LF bytes, the LF text's hash,
/// attribution and timestamps untouched — and records itself.
#[tokio::test]
async fn postgres_stored_spec_text_is_normalized_once() {
    use sanshain_service::domain::models::{ApiType, EndpointRecord, Stability};
    use sanshain_service::domain::ports::{SpecRepository, UpsertSpecVersion};

    let postgres_container = common::start_postgres().await;
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

    let sid = repo.ensure_service("legacy-grpc").await.unwrap();
    let version: sanshain_service::domain::models::SemVer = "1.0.0".parse().unwrap();
    let raw = "\u{feff}syntax = \"proto3\";\r\n// sanshain-version: 1.0.0\r\nservice S {\r\n  rpc Get (R) returns (R);\r\n}\r\n";
    let lf = "syntax = \"proto3\";\n// sanshain-version: 1.0.0\nservice S {\n  rpc Get (R) returns (R);\n}\n";
    let id = repo
        .upsert_spec_version(UpsertSpecVersion {
            outbox: Vec::new(),
            service_id: sid,
            api_type: ApiType::Proto,
            version,
            stability: Stability::Ga,
            content: raw,
            content_hash: "sha256:raw",
            provided_by: "alice",
            expected_prior_hash: None,
            now_iso: "2026-01-01T00:00:00Z",
            endpoints: vec![EndpointRecord {
                id: None,
                api_type: ApiType::Proto,
                path: "S".to_string(),
                normalized_path: "S".to_string(),
                method: "Get".to_string(),
                yaml_content: "syntax = \"proto3\";\r\n".to_string(),
                deprecated: false,
            }],
        })
        .await
        .unwrap();

    let summary = services::normalize_stored_spec_text(&repo).await.unwrap();
    assert_eq!((summary.versions, summary.endpoints), (1, 1));
    assert_eq!(
        repo.get_spec_content(id).await.unwrap().as_deref(),
        Some(lf)
    );
    let meta = repo
        .find_spec_version(sid, ApiType::Proto, version)
        .await
        .unwrap()
        .unwrap();
    let expected_hash = {
        use sha2::{Digest, Sha256};
        format!("sha256:{}", hex::encode(Sha256::digest(lf.as_bytes())))
    };
    assert_eq!(meta.content_hash, expected_hash);
    assert_eq!(meta.provided_by, "alice");
    assert_eq!(meta.updated_at, "2026-01-01T00:00:00Z");
    assert_eq!(
        repo.get_endpoints_for_version(id).await.unwrap()[0].yaml_content,
        "syntax = \"proto3\";\n"
    );
    assert!(
        services::normalize_stored_spec_text(&repo)
            .await
            .unwrap()
            .already_done
    );
}

/// ADR-0004: trunk pins are append-only on PostgreSQL exactly as on SQLite —
/// a re-pin closes the open record and inserts, an identical re-pin only
/// refreshes, and the current view is the open rows.
#[tokio::test]
async fn test_postgres_trunk_pins_append_only() {
    use sanshain_service::domain::models::{ApiType, SemVer};
    use sanshain_service::domain::ports::{RecordTrunkPinParams, SpecRepository};

    let postgres_container = common::start_postgres().await;
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

    let postgres_container = common::start_postgres().await;
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

#[tokio::test]
async fn test_postgres_periphery_append_only_and_at_an_instant() {
    use sanshain_service::domain::models::{PeripheralDependency, PeripheryTrack};
    use sanshain_service::domain::ports::SpecRepository;

    let postgres_container = common::start_postgres().await;
    let host = postgres_container.get_host().await.unwrap();
    let port = postgres_container.get_host_port_ipv4(5432).await.unwrap();
    let db_url = format!("postgres://postgres:postgres@{}:{}/postgres", host, port);
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();
    let repo = PostgresSpecRepository::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let db = |r: &str| format!("resource:default/{r}");
    let names = |v: Vec<PeripheralDependency>| -> Vec<String> {
        v.into_iter().map(|p| p.resource).collect()
    };
    repo.reconcile_periphery(
        "orders",
        true,
        &[db("a"), db("b")],
        "2026-01-01T00:00:00Z",
        Vec::new(),
    )
    .await
    .unwrap();
    repo.reconcile_periphery(
        "orders",
        true,
        &[db("b"), db("c")],
        "2026-02-01T00:00:00Z",
        Vec::new(),
    )
    .await
    .unwrap();
    // The open-row index decides a racing duplicate instead of erroring.
    repo.reconcile_periphery(
        "orders",
        true,
        &[db("b"), db("c")],
        "2026-02-02T00:00:00Z",
        Vec::new(),
    )
    .await
    .unwrap();

    assert_eq!(
        names(
            repo.list_periphery(PeripheryTrack::Trunk, None)
                .await
                .unwrap()
        ),
        vec![db("b"), db("c")]
    );
    assert_eq!(
        names(
            repo.list_periphery(PeripheryTrack::Trunk, Some("2026-01-15T00:00:00Z"))
                .await
                .unwrap()
        ),
        vec![db("a"), db("b")]
    );
    assert_eq!(
        names(
            repo.list_periphery(PeripheryTrack::Dev, None)
                .await
                .unwrap()
        ),
        vec![db("b"), db("c")]
    );

    // A cut copies trunk's periphery as of its instant; a branch off the
    // branch copies the branch's; the timeline sees the periphery changes.
    let rel = repo
        .insert_branch(
            "rel",
            "2026-03-01T00:00:00Z",
            "root",
            "trunk",
            "2026-01-15T00:00:00Z",
            Vec::new(),
        )
        .await
        .unwrap();
    repo.copy_trunk_graph_to_branch(rel, "2026-01-15T00:00:00Z", "2026-03-01T00:00:00Z")
        .await
        .unwrap();
    assert_eq!(
        names(
            repo.list_periphery(PeripheryTrack::Branch(rel), None)
                .await
                .unwrap()
        ),
        vec![db("a"), db("b")]
    );
    let hotfix = repo
        .insert_branch(
            "hotfix",
            "2026-03-02T00:00:00Z",
            "root",
            "rel",
            "2026-03-02T00:00:00Z",
            Vec::new(),
        )
        .await
        .unwrap();
    repo.copy_branch_graph_to_branch(hotfix, rel, "2026-03-02T00:00:00Z", "2026-03-02T00:00:00Z")
        .await
        .unwrap();
    assert_eq!(
        names(
            repo.list_periphery(PeripheryTrack::Branch(hotfix), None)
                .await
                .unwrap()
        ),
        vec![db("a"), db("b")]
    );
    let dates = repo.list_graph_change_dates(None).await.unwrap();
    assert!(
        dates.contains(&"2026-02-01T00:00:00Z".to_string()),
        "{dates:?}"
    );
    let branch_dates = repo.list_graph_change_dates(Some(rel)).await.unwrap();
    assert_eq!(branch_dates, vec!["2026-03-01T00:00:00Z".to_string()]);

    // Deleting a branch cascades its copy away.
    repo.delete_branch(rel).await.unwrap();
    assert!(
        repo.list_periphery(PeripheryTrack::Branch(rel), None)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn test_postgres_system_context_upsert_get_list_delete() {
    use sanshain_service::domain::models::{SystemContext, SystemLink, SystemOwner};
    use sanshain_service::domain::ports::SpecRepository;

    let postgres_container = common::start_postgres().await;
    let host = postgres_container.get_host().await.unwrap();
    let port = postgres_container.get_host_port_ipv4(5432).await.unwrap();
    let db_url = format!("postgres://postgres:postgres@{}:{}/postgres", host, port);
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();
    let repo = PostgresSpecRepository::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let context = SystemContext {
        owner: Some(SystemOwner {
            kind: "group".to_string(),
            name: "team-orders".to_string(),
        }),
        description: Some("Orders.".to_string()),
        source_location: Some("https://git.example/orders".to_string()),
        links: vec![SystemLink {
            url: "https://wiki.example/orders".to_string(),
            title: "Wiki".to_string(),
            link_type: Some("docs".to_string()),
            icon: None,
        }],
        tags: vec!["java".to_string()],
    };
    assert!(
        repo.upsert_system_context("orders", &context, "2026-01-01T00:00:00Z")
            .await
            .unwrap()
    );
    // The same content again: nothing changes.
    assert!(
        !repo
            .upsert_system_context("orders", &context, "2026-01-02T00:00:00Z")
            .await
            .unwrap()
    );
    assert_eq!(
        repo.get_system_context("orders").await.unwrap(),
        Some(context.clone())
    );
    assert!(
        repo.upsert_system_context("billing", &SystemContext::default(), "2026-01-03T00:00:00Z")
            .await
            .unwrap()
    );
    let all = repo.list_system_contexts().await.unwrap();
    assert_eq!(
        all.keys().cloned().collect::<Vec<_>>(),
        vec!["billing".to_string(), "orders".to_string()]
    );

    // A different content replaces the row in place.
    let emptied = SystemContext::default();
    assert!(
        repo.upsert_system_context("orders", &emptied, "2026-01-04T00:00:00Z")
            .await
            .unwrap()
    );
    assert_eq!(
        repo.get_system_context("orders").await.unwrap(),
        Some(emptied)
    );
    repo.delete_system_context("orders").await.unwrap();
    assert_eq!(repo.get_system_context("orders").await.unwrap(), None);
    assert_eq!(repo.list_system_contexts().await.unwrap().len(), 1);
}

#[tokio::test]
async fn test_postgres_member_versions_record_close_and_read_at_an_instant() {
    use sanshain_service::domain::models::{ApiType, MemberTrack, Stability};
    use sanshain_service::domain::ports::{SpecRepository, UpsertSpecVersion};

    let postgres_container = common::start_postgres().await;
    let host = postgres_container.get_host().await.unwrap();
    let port = postgres_container.get_host_port_ipv4(5432).await.unwrap();
    let db_url = format!("postgres://postgres:postgres@{}:{}/postgres", host, port);
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();
    let repo = PostgresSpecRepository::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let sid = repo.ensure_service("orders").await.unwrap();
    let mut ids = Vec::new();
    for (version, at) in [
        ("1.0.0", "2026-01-01T00:00:00Z"),
        ("2.0.0", "2026-02-01T00:00:00Z"),
    ] {
        let id = repo
            .upsert_spec_version(UpsertSpecVersion {
                outbox: Vec::new(),
                service_id: sid,
                api_type: ApiType::OpenApi,
                version: version.parse().unwrap(),
                stability: Stability::Ga,
                content: "content",
                content_hash: "sha256:x",
                provided_by: "ci",
                expected_prior_hash: None,
                now_iso: at,
                endpoints: vec![],
            })
            .await
            .unwrap();
        repo.touch_spec_version_trunk(id, at).await.unwrap();
        ids.push(id);
    }
    repo.touch_spec_version_trunk(ids[1], "2026-03-01T00:00:00Z")
        .await
        .unwrap();

    let live = repo
        .list_member_versions(MemberTrack::Trunk, None)
        .await
        .unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].version.to_string(), "2.0.0");
    assert_eq!(live[0].valid_from, "2026-02-01T00:00:00Z");
    assert_eq!(
        live[0].last_provided_at.as_deref(),
        Some("2026-03-01T00:00:00Z")
    );
    let past = repo
        .list_member_versions(MemberTrack::Trunk, Some("2026-01-15T00:00:00Z"))
        .await
        .unwrap();
    assert_eq!(past.len(), 1);
    assert_eq!(past[0].version.to_string(), "1.0.0");

    repo.close_trunk_pins_for_service_api(sid, ApiType::OpenApi, "2026-04-01T00:00:00Z")
        .await
        .unwrap();
    assert!(
        repo.list_member_versions(MemberTrack::Trunk, None)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(repo.delete_spec_version(ids[0]).await.unwrap());
    assert!(repo.delete_producer("orders").await.unwrap());

    let branch = repo
        .insert_branch(
            "rel",
            "2026-01-01T00:00:00Z",
            "alice",
            "main",
            "2026-01-01T00:00:00Z",
            Vec::new(),
        )
        .await
        .unwrap();
    let other = repo.ensure_service("hotfix").await.unwrap();
    repo.record_branch_member_version(
        branch,
        other,
        ApiType::AsyncApi,
        "1.0.1".parse().unwrap(),
        "2026-02-01T00:00:00Z",
    )
    .await
    .unwrap();
    let members = repo
        .list_member_versions(MemberTrack::Branch(branch), Some("2026-03-01T00:00:00Z"))
        .await
        .unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].service, "hotfix");
    assert_eq!(members[0].last_provided_at, None);
}

/// Webhooks (#47): two worker passes claiming at the same instant — two
/// replicas on one database — send a delivery once. `FOR UPDATE SKIP LOCKED`
/// in the claim is what the test pins.
#[tokio::test]
async fn test_postgres_concurrent_worker_passes_send_a_delivery_once() {
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::routing::post;
    use sanshain_service::application::webhook_delivery::{WebhookDeliveryConfig, deliver_due};
    use sanshain_service::domain::models::{
        NewWebhookEvent, WebhookDeliveryStatus, WebhookEventType, WebhookFormat, WebhookScope,
    };
    use sanshain_service::domain::ports::{NewWebhook, SpecRepository};
    use sanshain_service::infrastructure::webhook_sender::ReqwestWebhookSender;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let postgres_container = common::start_postgres().await;
    let host = postgres_container.get_host().await.unwrap();
    let port = postgres_container.get_host_port_ipv4(5432).await.unwrap();
    let db_url = format!("postgres://postgres:postgres@{}:{}/postgres", host, port);
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();
    let repo = PostgresSpecRepository::new(pool.clone());
    repo.run_migrations().await.unwrap();

    // A throwaway receiver that counts what it gets and answers 200.
    let hits = Arc::new(AtomicUsize::new(0));
    async fn record(State(hits): State<Arc<AtomicUsize>>) -> StatusCode {
        hits.fetch_add(1, Ordering::SeqCst);
        StatusCode::OK
    }
    let app = axum::Router::new()
        .route("/hook", post(record))
        .with_state(hits.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/hook", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let scope = WebhookScope::Global;
    let hook = repo
        .create_webhook(NewWebhook {
            scope: &scope,
            url: &url,
            format: WebhookFormat::Json,
            event_types: &[],
            producer_glob: None,
            secret: "whsec_test",
            secret_hint: "deadbeef",
            allow_list_exempt: true,
            created_by: "root",
            created_at: "2026-01-01T00:00:00Z",
        })
        .await
        .unwrap();
    let now = Utc::now();
    let occurred_at = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let events: Vec<NewWebhookEvent> = (0..5)
        .map(|i| NewWebhookEvent {
            id: format!("evt-{i}"),
            event_type: WebhookEventType::BranchCut,
            producer: None,
            occurred_at: occurred_at.clone(),
            data: json!({"name": format!("rel-{i}")}),
            hook_ids: vec![hook],
        })
        .collect();
    repo.insert_webhook_events(&events).await.unwrap();

    let sender = ReqwestWebhookSender::new(std::time::Duration::from_secs(5)).unwrap();
    let config = WebhookDeliveryConfig::default();
    let (a, b) = tokio::join!(
        deliver_due(&repo, &sender, &config, now),
        deliver_due(&repo, &sender, &config, now),
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(
        a.delivered + b.delivered,
        5,
        "every delivery once: {a:?} {b:?}"
    );
    assert_eq!(a.failed + b.failed + a.dead + b.dead, 0);
    assert_eq!(
        hits.load(Ordering::SeqCst),
        5,
        "the receiver saw each delivery exactly once"
    );
    let rows = repo.list_webhook_deliveries(hook, 10).await.unwrap();
    assert_eq!(rows.len(), 5);
    assert!(
        rows.iter()
            .all(|d| d.status == WebhookDeliveryStatus::Delivered && d.attempts == 1)
    );
    assert_eq!(repo.count_pending_webhook_deliveries().await.unwrap(), 0);
}

#[tokio::test]
async fn test_postgres_version_origin_set_get_list_and_swept_with_its_version() {
    use sanshain_service::domain::models::{ApiType, Stability, VersionOrigin};
    use sanshain_service::domain::ports::{SpecRepository, UpsertSpecVersion};

    let postgres_container = common::start_postgres().await;
    let host = postgres_container.get_host().await.unwrap();
    let port = postgres_container.get_host_port_ipv4(5432).await.unwrap();
    let db_url = format!("postgres://postgres:postgres@{}:{}/postgres", host, port);
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();
    let repo = PostgresSpecRepository::new(pool.clone());
    repo.run_migrations().await.unwrap();

    let sid = repo.ensure_service("origin-svc").await.unwrap();
    let id = repo
        .upsert_spec_version(UpsertSpecVersion {
            outbox: Vec::new(),
            service_id: sid,
            api_type: ApiType::OpenApi,
            version: "1.0.0".parse().unwrap(),
            stability: Stability::Snapshot,
            content: "c",
            content_hash: "sha256:c",
            provided_by: "alice",
            expected_prior_hash: None,
            now_iso: "2026-01-01T00:00:00Z",
            endpoints: Vec::new(),
        })
        .await
        .unwrap();
    let origin = VersionOrigin {
        build_url: Some("https://ci/1".to_string()),
        change_url: None,
        tickets: vec!["ORD-7".to_string()],
        note: Some("why".to_string()),
        trunk_build: true,
    };
    repo.set_version_origin(id, &origin, "2026-01-01T00:00:00Z")
        .await
        .unwrap();
    assert_eq!(
        repo.get_version_origin(id).await.unwrap(),
        Some(origin.clone())
    );
    assert_eq!(
        repo.list_version_origins().await.unwrap().get(&id),
        Some(&origin)
    );
    // An empty origin removes the row.
    repo.set_version_origin(id, &VersionOrigin::default(), "2026-01-02T00:00:00Z")
        .await
        .unwrap();
    assert_eq!(repo.get_version_origin(id).await.unwrap(), None);
    // Expiry sweeps the origin with its version.
    repo.set_version_origin(id, &origin, "2026-01-03T00:00:00Z")
        .await
        .unwrap();
    assert_eq!(
        repo.delete_expired_snapshots("2999-01-01T00:00:00Z")
            .await
            .unwrap(),
        1
    );
    assert!(repo.list_version_origins().await.unwrap().is_empty());
}

#[tokio::test]
async fn test_postgres_refuses_and_cleans_expired_credentials() {
    let postgres_container = common::start_postgres().await;
    let host = postgres_container.get_host().await.unwrap();
    let port = postgres_container.get_host_port_ipv4(5432).await.unwrap();
    let db_url = format!("postgres://postgres:postgres@{}:{}/postgres", host, port);
    // A database whose default zone is behind UTC: sqlx starts every
    // connection in UTC, so expiries still compare as instants.
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await
        .unwrap();
    sqlx::query("ALTER DATABASE postgres SET timezone TO 'Pacific/Pago_Pago'")
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();
    let repo = PostgresSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();
    credential_expiry::expired_credentials_are_refused_and_cleaned(&repo).await;
}
