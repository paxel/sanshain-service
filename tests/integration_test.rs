//! Router-seam integration tests of the general 2.0 surface: the
//! provide/require/report flow, wire-compat aliases, dry runs, auth gating,
//! response headers and audit trails. The version-line *model* rules
//! (GA immutability, promotion, mislabel checks, expiry) live in
//! `tests/spec_version_rules_test.rs`.

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use chrono::Utc;
use sanshain_service::application::services;
use sanshain_service::domain::models::AuthMode;
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
use tower::ServiceExt; // for `oneshot`

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
    /// Session token of a fully privileged seeded user.
    token: String,
}

// `cfg(test)` is always true in this crate; the attribute marks the helpers as
// test code for clippy's `allow-unwrap-in-tests`.
#[cfg(test)]
async fn seeded_repo() -> (SqliteSpecRepository, String) {
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
    (repo, session.token)
}

#[cfg(test)]
async fn setup() -> TestContext {
    let (repo, token) = seeded_repo().await;
    let app = create_app(test_app_state(repo.clone()));
    TestContext { app, repo, token }
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

#[cfg(test)]
fn users_spec(version: &str) -> String {
    format!(
        "openapi: 3.0.3\ninfo:\n  title: Test API\n  version: {version}\npaths:\n  /users:\n    get:\n      responses:\n        '200':\n          description: OK\n"
    )
}

// ---------------------------------------------------------------------------
// Plumbing: headers, probes, static assets.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_security_headers() {
    let ctx = setup().await;
    let response = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(headers.get("X-Content-Type-Options").unwrap(), "nosniff");
    assert_eq!(headers.get("X-Frame-Options").unwrap(), "SAMEORIGIN");
    assert_eq!(
        headers.get("Referrer-Policy").unwrap(),
        "strict-origin-when-cross-origin"
    );
    assert!(
        headers
            .get("Content-Security-Policy")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("default-src 'self'")
    );
}

#[tokio::test]
async fn test_health_endpoint() {
    let ctx = setup().await;
    let response = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_metrics_endpoint_is_available() {
    let ctx = setup().await;
    let response = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "text/plain; version=0.0.4; charset=utf-8"
    );
}

#[tokio::test]
async fn test_license_file_is_served_from_root_path() {
    let ctx = setup().await;
    let (status, _, body) = send(&ctx, "GET", "/LICENSE", None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.is_empty(), "the license text must not be empty");
}

// ---------------------------------------------------------------------------
// The provide → require → report flow.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_full_flow() {
    let ctx = setup().await;

    // 1. Provide a GA spec — the version comes from the document.
    let provide_payload = json!({
        "producername": "test-service",
        "openapi_yaml": users_spec("1.0.0"),
        "stability": "ga",
    });
    let (status, _, body) = send(&ctx, "POST", "/provide", Some(provide_payload), &[]).await;
    assert_eq!(status, StatusCode::ACCEPTED, "got: {body}");
    let provided = as_json(&body);
    assert_eq!(provided["version"], "1.0.0");
    assert_eq!(provided["stability"], "ga");
    assert_eq!(provided["changes"]["inserts"], 1);

    // 2. Require the endpoint at the exact pin.
    let (status, headers, _) = send(
        &ctx,
        "GET",
        "/require?consumername=client-a&producername=test-service&version=1.0.0&path=/users&method=GET",
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get("X-Sanshain-Resolution").unwrap(), "served");
    assert_eq!(headers.get("X-Sanshain-Version").unwrap(), "1.0.0");
    assert_eq!(headers.get("X-Sanshain-Stability").unwrap(), "ga");

    // 3. The JSON report carries the pinned edge with version and stability.
    let (status, _, body) = send(&ctx, "GET", "/report", None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    let report = as_json(&body);
    let graph = report["dependency_graph"].as_array().unwrap();
    assert_eq!(graph.len(), 1);
    assert_eq!(graph[0]["client"], "client-a");
    assert_eq!(graph[0]["service"], "test-service");
    assert_eq!(graph[0]["version"], "1.0.0");
    assert_eq!(graph[0]["stability"], "ga");
    assert_eq!(report["missing_endpoints"].as_array().unwrap().len(), 0);

    // 4. The Markdown report renders the same edge.
    let (status, headers, md_report) = send(&ctx, "GET", "/report/markdown", None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get("content-type").unwrap(),
        "text/markdown; charset=utf-8"
    );
    assert!(md_report.contains("# Sanshain Dependency Report"));
    assert!(
        md_report.contains("| client-a | test-service | OpenApi | 1.0.0 | ga | `/users` | `GET` |"),
        "got: {md_report}"
    );
}

#[tokio::test]
async fn test_require_bundle_etag_stability() {
    let ctx = setup().await;
    let spec = "openapi: 3.0.3\ninfo:\n  title: ETag Test\n  version: 1.0.0\npaths:\n  /alpha:\n    get:\n      responses:\n        '200':\n          description: Alpha\n  /beta:\n    post:\n      responses:\n        '201':\n          description: Beta\n";
    let (status, _, _) = send(
        &ctx,
        "POST",
        "/provide",
        Some(json!({
            "producername": "etag-svc",
            "openapi_yaml": spec,
            "stability": "ga",
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let bundle = |endpoints: Value| {
        json!({
            "consumername": "etag-client",
            "producername": "etag-svc",
            "version": "1.0.0",
            "endpoints": endpoints,
        })
    };

    // Order A: alpha first. Order B: beta first. Same ETag either way.
    let (status, headers_a, _) = send(
        &ctx,
        "POST",
        "/require-bundle",
        Some(bundle(json!([
            {"path": "/alpha", "method": "GET"},
            {"path": "/beta", "method": "POST"},
        ]))),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, headers_b, _) = send(
        &ctx,
        "POST",
        "/require-bundle",
        Some(bundle(json!([
            {"path": "/beta", "method": "POST"},
            {"path": "/alpha", "method": "GET"},
        ]))),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let etag_a = headers_a.get("ETag").unwrap();
    let etag_b = headers_b.get("ETag").unwrap();
    assert_eq!(
        etag_a, etag_b,
        "bundle ETags must not depend on endpoint order"
    );
}

// ---------------------------------------------------------------------------
// Dry runs store nothing.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_provide_dry_run_does_not_store_data() {
    let ctx = setup().await;
    let (status, _, body) = send(
        &ctx,
        "POST",
        "/provide",
        Some(json!({
            "producername": "dry-svc",
            "openapi_yaml": users_spec("1.0.0"),
            "stability": "ga",
            "dry_run": true,
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let response = as_json(&body);
    assert_eq!(response["version"], "1.0.0");
    assert_eq!(response["changes"]["inserts"], 1);

    // Nothing was created — the producer does not even exist.
    let (status, _, _) = send(&ctx, "GET", "/producers/dry-svc/versions", None, &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_require_dry_run_does_not_create_dependency() {
    let ctx = setup().await;
    let (status, _, _) = send(
        &ctx,
        "POST",
        "/provide",
        Some(json!({
            "producername": "svc",
            "openapi_yaml": users_spec("1.0.0"),
            "stability": "ga",
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let (status, _, _) = send(
        &ctx,
        "GET",
        "/require?consumername=curious&producername=svc&version=1.0.0&path=/users&method=GET&dry_run=true",
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, _, body) = send(&ctx, "GET", "/report", None, &[]).await;
    let report = as_json(&body);
    assert_eq!(
        report["dependency_graph"].as_array().unwrap().len(),
        0,
        "a dry-run require must not be recorded: {report}"
    );
}

// ---------------------------------------------------------------------------
// Auth gating of the API surface.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_api_is_locked_for_anonymous_callers() {
    let ctx = setup().await;

    // GET /require without any credential (dev mode is off).
    let response = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require?consumername=c&producername=s&version=1.0.0&path=/x&method=GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // POST /provide without any credential.
    let response = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "producername": "svc",
                        "openapi_yaml": users_spec("1.0.0"),
                        "stability": "ga",
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // A garbage token is 401, not 403.
    let response = ctx
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/require?consumername=c&producername=s&version=1.0.0&path=/x&method=GET")
                .header("Authorization", "Bearer not-a-real-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_disabled_mode_endpoints_return_503() {
    let ctx = setup().await;
    services::set_auth_mode(&ctx.repo, &AuthMode::Disabled)
        .await
        .unwrap();

    let (status, _, _) = send(
        &ctx,
        "GET",
        "/require?consumername=c&producername=s&version=1.0.0&path=/x&method=GET",
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

// ---------------------------------------------------------------------------
// 1.x wire-compat is gone entirely: the legacy name aliases are rejected by
// name, like every other branch-era leftover.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn legacy_servicename_on_provide_is_rejected_by_name() {
    let ctx = setup().await;
    let (status, _, body) = send(
        &ctx,
        "POST",
        "/provide",
        Some(json!({
            "servicename": "legacy-svc",
            "openapi_yaml": users_spec("1.0.0"),
            "stability": "ga",
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "got: {body}");
    assert!(
        body.contains("servicename"),
        "the rejected field must be named, got: {body}"
    );
}

#[tokio::test]
async fn legacy_clientname_and_servicename_on_require_are_rejected() {
    let ctx = setup().await;
    let (status, _, _) = send(
        &ctx,
        "POST",
        "/provide",
        Some(json!({
            "producername": "svc",
            "openapi_yaml": users_spec("1.0.0"),
            "stability": "ga",
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let (status, _, body) = send(
        &ctx,
        "GET",
        "/require?clientname=legacy-client&servicename=svc&version=1.0.0&path=/users&method=GET",
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "got: {body}");

    // No dependency may be minted by the failed require.
    let (_, _, body) = send(&ctx, "GET", "/report", None, &[]).await;
    let report = as_json(&body);
    assert!(
        report["dependency_graph"].as_array().unwrap().is_empty(),
        "a rejected require must not record a dependency"
    );
}

// ---------------------------------------------------------------------------
// Request body limit.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_request_body_over_limit_is_rejected_under_limit_accepted() {
    let (repo, token) = seeded_repo().await;
    let mut state = test_app_state(repo.clone());
    state.max_body_bytes = 4096;
    let ctx = TestContext {
        app: create_app(state),
        repo,
        token,
    };

    // Under the limit: accepted.
    let (status, _, body) = send(
        &ctx,
        "POST",
        "/provide",
        Some(json!({
            "producername": "svc",
            "openapi_yaml": users_spec("1.0.0"),
            "stability": "ga",
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "got: {body}");

    // Over the limit: 413, before any parsing happens.
    let huge_description = "x".repeat(8192);
    let huge = users_spec("1.0.1").replace(
        "description: OK",
        &format!("description: {huge_description}"),
    );
    let (status, _, _) = send(
        &ctx,
        "POST",
        "/provide",
        Some(json!({
            "producername": "svc",
            "openapi_yaml": huge,
            "stability": "ga",
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

// ---------------------------------------------------------------------------
// Audit trail.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn provide_and_require_are_audited_with_the_version() {
    let ctx = setup().await;
    let (status, _, _) = send(
        &ctx,
        "POST",
        "/provide",
        Some(json!({
            "producername": "audited-svc",
            "openapi_yaml": users_spec("1.0.0"),
            "stability": "ga",
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let (status, _, _) = send(
        &ctx,
        "GET",
        "/require?consumername=c&producername=audited-svc&version=1.0.0&path=/users&method=GET",
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, body) = send(
        &ctx,
        "GET",
        "/api/audit/timeline?limit=50&service=audited-svc",
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    let entries = as_json(&body);
    let entries = entries.as_array().unwrap();
    let provide_entry = entries
        .iter()
        .find(|e| e["action"] == "PROVIDE_SPEC")
        .unwrap_or_else(|| panic!("no PROVIDE_SPEC entry in {entries:?}"));
    assert_eq!(provide_entry["version"], "1.0.0");
    assert_eq!(provide_entry["username"], "root");
    let require_entry = entries
        .iter()
        .find(|e| e["action"] == "REQUIRE_SPEC")
        .unwrap_or_else(|| panic!("no REQUIRE_SPEC entry in {entries:?}"));
    assert_eq!(require_entry["version"], "1.0.0");
    assert_eq!(require_entry["action_type"], "READ");
}

#[tokio::test]
async fn no_op_reprovide_writes_no_audit_entry() {
    let ctx = setup().await;
    let payload = json!({
        "producername": "quiet-svc",
        "openapi_yaml": users_spec("1.0.0"),
        "stability": "ga",
    });
    let (status, _, _) = send(&ctx, "POST", "/provide", Some(payload.clone()), &[]).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let (status, _, _) = send(&ctx, "POST", "/provide", Some(payload), &[]).await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let (_, _, body) = send(
        &ctx,
        "GET",
        "/api/audit/timeline?limit=50&service=quiet-svc",
        None,
        &[],
    )
    .await;
    let entries = as_json(&body);
    let provides = entries
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["action"] == "PROVIDE_SPEC")
        .count();
    assert_eq!(
        provides, 1,
        "an identical re-provide changes nothing and must not be recorded: {entries}"
    );
}
