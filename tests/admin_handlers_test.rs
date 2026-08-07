//! The admin surface through the real router (in-memory SQLite seam).
//!
//! Sanshain 2.0 (ADR-0003): versions replace branches. The admin surface
//! manages Producer version lines — listing them, downloading and diffing the
//! stored documents, and the audited delete-version escape hatch from GA
//! immutability.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::Utc;
use sanshain_service::application::services;
use sanshain_service::domain::models::{ApiType, Stability};
use sanshain_service::domain::ports::SpecRepository;
use sanshain_service::infrastructure::cached_repository::CachedSpecRepository;
use sanshain_service::infrastructure::database::DatabaseRepo;
use sanshain_service::infrastructure::sqlite_repository::SqliteSpecRepository;

/// A root set that reserves no username used by these tests.
fn no_root_users() -> sanshain_service::domain::permissions::RootUsers {
    sanshain_service::domain::permissions::RootUsers::resolve(Some("__unused-root__"), None)
}
use sanshain_service::{AppState, create_app};
use sqlx::sqlite::SqlitePoolOptions;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64};
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

fn test_state(repo: SqliteSpecRepository) -> AppState {
    let (spec_updated_tx, _) = tokio::sync::broadcast::channel(16);
    AppState {
        repo: CachedSpecRepository::new(DatabaseRepo::Sqlite(repo), 64),
        db_url: "sqlite::memory:".into(),
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
        directory_roles: sanshain_service::application::directory_roles::DirectoryRoleCache::new(
            std::time::Duration::from_secs(300),
        ),
        root_users: std::sync::Arc::new(sanshain_service::domain::permissions::RootUsers::resolve(
            Some("root"),
            None,
        )),
    }
}

/// The document seeded for `demo-svc` version 1.0.0 (GA). Kept as a constant so
/// the full-spec test can assert the download is byte-for-byte what was stored.
const DEMO_SPEC_1_0_0: &str = r#"openapi: 3.0.0
info:
  title: demo
  version: 1.0.0
paths:
  /hello:
    get:
      responses:
        '200': { description: ok }
"#;

/// A spec document for `producer` with the given version and endpoint paths.
#[cfg(test)]
fn spec_with_paths(version: &str, paths: &[&str]) -> String {
    let mut doc = format!("openapi: 3.0.0\ninfo:\n  title: t\n  version: {version}\npaths:\n");
    for path in paths {
        doc.push_str(&format!(
            "  {path}:\n    get:\n      responses:\n        '200': {{ description: ok }}\n"
        ));
    }
    doc
}

#[cfg(test)]
async fn provide(repo: &SqliteSpecRepository, producer: &str, content: &str, stability: Stability) {
    services::provide_spec(
        repo,
        services::ProvideSpecParams {
            producername: producer,
            api_type: ApiType::OpenApi,
            content,
            stability,
            dry_run: false,
            trunk: false,
            tag: None,
            caller: Some(if stability == Stability::Ga {
                sanshain_service::domain::permissions::Actor::test_releaser()
            } else {
                sanshain_service::domain::permissions::Actor::test_caller()
            }),
            require_prior_content_match: false,
        },
    )
    .await
    .expect("provide");
}

/// Record a Consumer pin on demo-svc 1.0.0's /hello, the way a real require
/// does — resolution succeeded, so the dependency is recorded.
#[cfg(test)]
async fn pin_consumer(repo: &SqliteSpecRepository, consumer: &str, producer: &str, version: &str) {
    services::require_endpoint(
        repo,
        services::RequireEndpointParams {
            consumername: consumer,
            producername: producer,
            version: version.parse().expect("semver"),
            api_type: ApiType::OpenApi,
            path: "/hello",
            method: "GET",
            trunk: false,
            tag: None,
        },
    )
    .await
    .expect("require");
}

// `cfg(test)` is always true in this crate; the attribute marks the helper as
// test code for clippy's `allow-unwrap-in-tests`.
#[cfg(test)]
async fn app_with_seed() -> (axum::Router, SqliteSpecRepository, String) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    // Seed initial admin and get token
    unsafe {
        std::env::set_var("INITIAL_ADMIN_USERNAME", "root");
        std::env::set_var("INITIAL_ADMIN_PASSWORD", "root_password");
    }
    services::ensure_initial_admin(&repo).await.unwrap();
    let (session, _user) = services::login(&repo, "root", "root_password")
        .await
        .unwrap();

    // Seed a demo Producer with one GA version line entry.
    provide(&repo, "demo-svc", DEMO_SPEC_1_0_0, Stability::Ga).await;

    let app = create_app(test_state(repo.clone()));
    (app, repo, session.token)
}

#[cfg(test)]
async fn get_json(app: &axum::Router, uri: &str, auth: &str) -> (StatusCode, serde_json::Value) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header(axum::http::header::AUTHORIZATION, auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    (status, json)
}

fn all_audit_logs_filter() -> sanshain_service::domain::models::AuditLogFilter {
    sanshain_service::domain::models::AuditLogFilter {
        from_date: None,
        to_date: None,
        action_type: None,
        service_wildcard: None,
        version_wildcard: None,
        stream: None,
        limit: 100,
    }
}

#[tokio::test]
async fn admin_lists_and_cache_endpoints_work() {
    let (app, _repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    // The producers listing carries each Producer's versions array.
    let (status, producers) = get_json(&app, "/admin/producers", &auth).await;
    assert_eq!(status, StatusCode::OK);
    let demo = producers
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "demo-svc")
        .expect("demo-svc listed");
    let versions = demo["versions"].as_array().expect("versions array");
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0]["version"], "1.0.0");
    assert_eq!(versions[0]["stability"], "ga");

    // List one version's endpoints, addressed by (api_type, version).
    let (status, endpoints) = get_json(
        &app,
        "/admin/producers/demo-svc/endpoints?api_type=openapi&version=1.0.0",
        &auth,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let listed = endpoints.as_array().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["path"], "/hello");
    assert_eq!(listed[0]["method"], "GET");

    // Cache stats
    let (status, _) = get_json(&app, "/admin/settings/cache", &auth).await;
    assert_eq!(status, StatusCode::OK);

    // Clear cache
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/cache/clear")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

// The per-Producer versions endpoint lists the line entries with stability and
// endpoint count — what the versions view renders.
#[tokio::test]
async fn versions_listing_carries_stability_and_endpoint_count() {
    let (app, repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);
    provide(
        &repo,
        "demo-svc",
        &spec_with_paths("1.1.0", &["/hello", "/extra"]),
        Stability::Snapshot,
    )
    .await;

    let (status, versions) = get_json(&app, "/admin/producers/demo-svc/versions", &auth).await;
    assert_eq!(status, StatusCode::OK);
    let lines = versions.as_array().unwrap();
    assert_eq!(lines.len(), 2, "two entries on the openapi line");
    // Newest first within the API type.
    assert_eq!(lines[0]["version"], "1.1.0");
    assert_eq!(lines[0]["stability"], "snapshot");
    assert_eq!(lines[0]["endpoint_count"], 2);
    assert_eq!(lines[1]["version"], "1.0.0");
    assert_eq!(lines[1]["stability"], "ga");
    assert_eq!(lines[1]["endpoint_count"], 1);

    // An unknown Producer is 404, not an empty list.
    let (status, _) = get_json(&app, "/admin/producers/no-such-svc/versions", &auth).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// The full-spec download returns the stored document verbatim, named
// producer-version in the Content-Disposition.
#[tokio::test]
async fn full_spec_download_is_verbatim_and_named() {
    let (app, _repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    let res = app
        .oneshot(
            Request::builder()
                .uri("/admin/producers/demo-svc/full-spec?api_type=openapi&version=1.0.0")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let disposition = res
        .headers()
        .get(axum::http::header::CONTENT_DISPOSITION)
        .expect("content-disposition")
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(disposition, "attachment; filename=\"demo-svc-1.0.0.yaml\"");
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        std::str::from_utf8(&body).unwrap(),
        DEMO_SPEC_1_0_0,
        "the download is exactly what the Producer submitted"
    );
}

// The diff endpoint returns a unified diff between two entries of a line.
#[tokio::test]
async fn diff_endpoint_returns_a_unified_diff() {
    let (app, repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);
    provide(
        &repo,
        "demo-svc",
        &spec_with_paths("1.1.0", &["/hello", "/extra"]),
        Stability::Snapshot,
    )
    .await;

    let res = app
        .oneshot(
            Request::builder()
                .uri("/admin/producers/demo-svc/diff?api_type=openapi&from=1.0.0&to=1.1.0")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let diff = std::str::from_utf8(&body).unwrap();
    assert!(
        diff.contains("--- demo-svc 1.0.0"),
        "old side named: {diff}"
    );
    assert!(
        diff.contains("+++ demo-svc 1.1.0"),
        "new side named: {diff}"
    );
    assert!(diff.contains("-  version: 1.0.0"), "removed line: {diff}");
    assert!(diff.contains("+  version: 1.1.0"), "added line: {diff}");
    assert!(
        diff.contains("+  /extra:"),
        "the added endpoint shows as +: {diff}"
    );
}

// --- Delete-version: the sole escape hatch from GA immutability ---

#[tokio::test]
async fn admin_deletes_a_version_sees_dependents_and_frees_the_number() {
    let (app, repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);
    pin_consumer(&repo, "pinned-app", "demo-svc", "1.0.0").await;

    // The dependents endpoint names the pinned Consumers before the delete.
    let (status, dependents) = get_json(
        &app,
        "/admin/producers/demo-svc/versions/openapi/1.0.0/dependents",
        &auth,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(dependents, serde_json::json!(["pinned-app"]));

    // Delete the GA version.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/admin/producers/demo-svc/versions/openapi/1.0.0")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["deleted"], "1.0.0");
    assert_eq!(json["dependents"], serde_json::json!(["pinned-app"]));

    // The audit trail records DELETE_VERSION and names the pinned Consumers.
    let logs = repo.get_audit_logs(all_audit_logs_filter()).await.unwrap();
    let entry = logs
        .iter()
        .find(|l| l.action == "DELETE_VERSION")
        .expect("delete-version is audited");
    assert_eq!(entry.username, "root");
    assert_eq!(entry.service.as_deref(), Some("demo-svc"));
    assert_eq!(entry.version.as_deref(), Some("1.0.0"));
    assert!(
        entry.details.contains("pinned-app"),
        "the audit entry names the dependents: {}",
        entry.details
    );

    // Requiring the deleted version now hard-fails with 404 (no fallback).
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/require?consumername=pinned-app&producername=demo-svc&version=1.0.0&path=/hello&method=GET")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // ...and the number is free again: re-providing 1.0.0 succeeds.
    provide(&repo, "demo-svc", DEMO_SPEC_1_0_0, Stability::Ga).await;
}

#[tokio::test]
async fn a_maintainer_deletes_their_own_versions_but_not_anothers() {
    let (app, repo, _token) = app_with_seed().await;

    // A second Producer the maintainer is NOT responsible for.
    provide(
        &repo,
        "other-svc",
        &spec_with_paths("2.0.0", &["/x"]),
        Stability::Ga,
    )
    .await;

    // A maintainer of demo-svc only.
    let hash = services::hash_password("pw").unwrap();
    let user = repo
        .create_user("demo-maintainer", &hash, true)
        .await
        .unwrap();
    let demo_id = repo.find_service("demo-svc").await.unwrap().unwrap();
    repo.add_user_maintainer(demo_id, user.id).await.unwrap();
    let session = repo
        .create_session(user.id, "2099-12-31T23:59:59")
        .await
        .unwrap();
    let auth = format!("Bearer {}", session.token);

    // Not another's: 403, and the version survives.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/admin/producers/other-svc/versions/openapi/2.0.0")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    let other_id = repo.find_service("other-svc").await.unwrap().unwrap();
    assert!(
        repo.find_spec_version(other_id, ApiType::OpenApi, "2.0.0".parse().unwrap())
            .await
            .unwrap()
            .is_some(),
        "the refused delete must not have removed anything"
    );

    // Their own: allowed.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/admin/producers/demo-svc/versions/openapi/1.0.0")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(
        repo.find_spec_version(demo_id, ApiType::OpenApi, "1.0.0".parse().unwrap())
            .await
            .unwrap()
            .is_none(),
        "the maintainer's delete landed"
    );
}

#[tokio::test]
async fn deleting_an_unknown_version_is_not_found() {
    let (app, _repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    for uri in [
        // Known Producer, unknown version.
        "/admin/producers/demo-svc/versions/openapi/9.9.9",
        // Unknown Producer.
        "/admin/producers/no-such-svc/versions/openapi/1.0.0",
    ] {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(uri)
                    .header(axum::http::header::AUTHORIZATION, &auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{uri}");
    }
}

// --- Consumer endpoints carry the Pin's version and stability ---

#[tokio::test]
async fn consumer_endpoints_listing_carries_version_and_stability() {
    let (app, repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);
    pin_consumer(&repo, "pinned-app", "demo-svc", "1.0.0").await;

    let (status, endpoints) = get_json(&app, "/admin/consumers/pinned-app/endpoints", &auth).await;
    assert_eq!(status, StatusCode::OK);
    let listed = endpoints.as_array().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["service"], "demo-svc");
    assert_eq!(listed[0]["version"], "1.0.0");
    assert_eq!(listed[0]["stability"], "ga");
    assert_eq!(listed[0]["path"], "/hello");
    assert_eq!(listed[0]["method"], "GET");
}

// --- Snapshot cleanup configuration ---

#[tokio::test]
async fn snapshot_max_age_roundtrip_and_cleanup_trigger() {
    let (app, _repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    let (status, json) = get_json(&app, "/admin/settings/snapshot-max-age", &auth).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["days"], 30, "the ADR-0003 default");

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/snapshot-max-age")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"days":5}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let (status, json) = get_json(&app, "/admin/settings/snapshot-max-age", &auth).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["days"], 5);

    // Trigger the cleanup: everything seeded is fresh (and the only line entry
    // is GA, which is never age-culled), so nothing dies.
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/cleanup/snapshots")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["deleted"], 0);
}

// --- Carried-over surfaces that survived the rework unchanged ---

#[tokio::test]
async fn test_favorites_api_endpoints() {
    let (app, _repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    // 1. Get initial favorites (should be empty lists)
    let (status, _) = get_json(&app, "/auth/favorites", &auth).await;
    assert_eq!(status, StatusCode::OK);
    let (_, favs) = get_json(&app, "/auth/favorites", &auth).await;
    assert!(favs["services"].as_array().unwrap().is_empty());
    assert!(favs["clients"].as_array().unwrap().is_empty());

    // 2. Add a favorite service
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/favorites/service/demo-svc")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 3. Add an invalid item type (should fail)
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/favorites/invalid_type/demo-svc")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // 4. Get favorites again (should contain demo-svc)
    let (_, favs) = get_json(&app, "/auth/favorites", &auth).await;
    assert_eq!(favs["services"], serde_json::json!(["demo-svc"]));

    // 5. Remove the favorite service
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/auth/favorites/service/demo-svc")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 6. Get favorites again (should be empty again)
    let (_, favs) = get_json(&app, "/auth/favorites", &auth).await;
    assert!(favs["services"].as_array().unwrap().is_empty());
}

// The dev-mode toggle (persisted via the auth-config surface) records intent,
// not the gated effective value: `is_dev_mode_requested` reads back `true`
// even though the ALLOW_INSECURE_DEV_MODE gate is closed in this test process,
// while the effective `get_dev_mode` stays `false`.
#[tokio::test]
async fn dev_mode_setting_records_intent_but_stays_gated() {
    let (_app, repo, _token) = app_with_seed().await;
    assert!(
        !services::dev_mode_gate_open(),
        "gate must be closed for this test to be meaningful"
    );

    services::set_dev_mode(&repo, true).await.unwrap();
    assert!(
        services::is_dev_mode_requested(&repo).await.unwrap(),
        "the persisted intent must read back"
    );
    assert!(
        !services::get_dev_mode(&repo).await.unwrap(),
        "the closed gate must keep dev mode ineffective"
    );
}

// Revoking a token that doesn't exist (or was already revoked) now correctly
// 404s instead of silently "succeeding" with a false audit entry. Revoking a
// real token succeeds, and neither audit entry names the token or its ID.
#[tokio::test]
async fn revoke_token_404s_when_not_found_and_audit_has_no_identifying_details() {
    let (app, repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/tokens")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({"name": "very-secret-name", "expires_in_days": 30})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let new_id = created["id"].as_str().unwrap().to_string();

    // First revoke succeeds.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/auth/tokens/{new_id}"))
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Revoking the same (now-gone) token again correctly 404s.
    let res = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/auth/tokens/{new_id}"))
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // Neither the token's name nor its ID appear in the audit log.
    let logs = repo.get_audit_logs(all_audit_logs_filter()).await.unwrap();
    let token_logs: Vec<_> = logs
        .iter()
        .filter(|l| l.action == "CREATE_TOKEN" || l.action == "REVOKE_TOKEN")
        .collect();
    assert_eq!(
        token_logs.len(),
        2,
        "one CREATE_TOKEN and one REVOKE_TOKEN entry (the failed second revoke logs nothing)"
    );
    for log in token_logs {
        assert!(
            !log.details.contains("very-secret-name") && !log.details.contains(&new_id),
            "audit details must not name the token or its ID: {}",
            log.details
        );
    }
}

// The audit log (and its CSV export) is restricted to admins: a non-admin,
// authenticated user gets 403, while an admin still gets 200.
#[tokio::test]
async fn audit_logs_are_admin_only() {
    use sanshain_service::application::auth_service;
    use sanshain_service::domain::models::AuthMode;

    let (app, repo, admin_token) = app_with_seed().await;
    let admin_auth = format!("Bearer {}", admin_token);

    // Register + approve + log in a non-admin user.
    auth_service::set_auth_mode(&repo, &AuthMode::Local)
        .await
        .unwrap();
    auth_service::register_user(&repo, &no_root_users(), "regular-user", "password123")
        .await
        .unwrap();
    let users = auth_service::list_users(&repo).await.unwrap();
    let regular = users.iter().find(|u| u.username == "regular-user").unwrap();
    assert!(
        repo.list_user_roles(regular.id)
            .await
            .expect("roles readable")
            .is_empty(),
        "newly registered users hold no roles"
    );
    auth_service::approve_user(&repo, regular.id).await.unwrap();
    let (session, _user) = auth_service::login(&repo, "regular-user", "password123")
        .await
        .unwrap();
    let regular_auth = format!("Bearer {}", session.token);

    // Non-admin: 403 on both audit-log routes.
    for uri in [
        "/admin/observability/audit-logs",
        "/admin/observability/audit-logs/export",
    ] {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header(axum::http::header::AUTHORIZATION, &regular_auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::FORBIDDEN,
            "non-admin must be forbidden from {uri}"
        );
    }

    // Admin: still 200 on both.
    for uri in [
        "/admin/observability/audit-logs",
        "/admin/observability/audit-logs/export",
    ] {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header(axum::http::header::AUTHORIZATION, &admin_auth)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "admin must retain access to {uri}"
        );
    }
}

// A no-op re-provide (identical spec, zero endpoint changes) must not create a
// second PROVIDE_SPEC audit entry — audit noise reported during testing.
#[tokio::test]
async fn provide_without_changes_is_not_audited() {
    let (app, repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    let spec = spec_with_paths("1.0.0", &["/ping"]);
    let body = serde_json::json!({
        "producername": "audit-svc",
        "stability": "snapshot",
        "openapi_yaml": spec,
    })
    .to_string();

    // First provide creates the endpoint -> audited.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);

    // Second provide of the identical spec -> no changes -> NOT audited.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);

    let logs = repo.get_audit_logs(all_audit_logs_filter()).await.unwrap();
    let provide_count = logs
        .iter()
        .filter(|l| l.action == "PROVIDE_SPEC" && l.service.as_deref() == Some("audit-svc"))
        .count();
    assert_eq!(
        provide_count, 1,
        "a no-op re-provide must not add a second PROVIDE_SPEC audit entry"
    );
}

// Fetching the report (used to render the dependency view) must not create a
// REPORT audit entry — otherwise merely viewing the graph is audited.
#[tokio::test]
async fn viewing_report_is_not_audited() {
    let (app, repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    let res = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/report")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let logs = repo.get_audit_logs(all_audit_logs_filter()).await.unwrap();
    assert!(
        logs.iter().all(|l| l.action != "REPORT"),
        "viewing the report must not create a REPORT audit entry"
    );
}

// --- Attribution: the token defines the user (no author field) ---

// The `author` hint was removed from the contract: attribution is the
// authenticated Actor. Sending it must fail by name like every other unknown
// field, and blame credits the token identity.
#[tokio::test]
async fn author_field_is_rejected_and_blame_credits_the_actor() {
    let (app, repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    let spec = spec_with_paths("1.0.0", &["/blame-test"]);
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "producername": "blame-svc",
                        "stability": "snapshot",
                        "openapi_yaml": spec,
                        "author": "external.author@example.com",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "the removed author field must be rejected by name"
    );

    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "producername": "blame-svc",
                        "stability": "snapshot",
                        "openapi_yaml": spec,
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);

    let history =
        services::get_endpoint_history(&repo, "blame-svc", ApiType::OpenApi, "/blame-test", "GET")
            .await
            .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(
        history[0].provided_by, "root",
        "blame credits the authenticated Actor"
    );
}

// Promoting a snapshot with byte-identical content keeps the snapshot's
// provider on the version — the human who built it stays credited — while the
// audit's VERSION_PROMOTED entry names the promoting Actor (e.g. CI).
#[tokio::test]
async fn same_content_promotion_preserves_the_snapshot_provider() {
    let (app, repo, root_token) = app_with_seed().await;

    // The developer pushes the snapshot with their own token.
    let hash = services::hash_password("pw").unwrap();
    let dev = repo.create_user("dev-user", &hash, true).await.unwrap();
    repo.grant_user_role(dev.id, "admin").await.unwrap();
    let dev_session = repo
        .create_session(dev.id, "2099-12-31T23:59:59")
        .await
        .unwrap();

    let spec = spec_with_paths("3.0.0", &["/promoted"]);
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header(
                    axum::http::header::AUTHORIZATION,
                    format!("Bearer {}", dev_session.token),
                )
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "producername": "promo-svc",
                        "stability": "snapshot",
                        "openapi_yaml": spec,
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);

    // CI (root's token here) promotes the identical content to GA.
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header(
                    axum::http::header::AUTHORIZATION,
                    format!("Bearer {}", root_token),
                )
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "producername": "promo-svc",
                        "stability": "ga",
                        "openapi_yaml": spec,
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);

    let sid = repo.find_service("promo-svc").await.unwrap().unwrap();
    let meta = repo
        .find_spec_version(sid, ApiType::OpenApi, "3.0.0".parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(meta.stability, Stability::Ga, "promotion landed");
    assert_eq!(
        meta.provided_by, "dev-user",
        "identical-content promotion keeps the snapshot's provider"
    );

    let logs = repo.get_audit_logs(all_audit_logs_filter()).await.unwrap();
    let promoted = logs
        .iter()
        .find(|l| l.action == "VERSION_PROMOTED" && l.service.as_deref() == Some("promo-svc"))
        .expect("promotion must be audited");
    assert_eq!(
        promoted.username, "root",
        "the audit names the promoting Actor, not the credited provider"
    );
}

/// Seed `promo-svc` 2.0.0 as a snapshot pushed by a fresh role-less
/// `dev-user`, through the real HTTP surface. Returns dev-user's token.
#[cfg(test)]
async fn seed_promo_snapshot(app: &axum::Router, repo: &SqliteSpecRepository) -> String {
    let hash = services::hash_password("pw").unwrap();
    let dev = repo.create_user("dev-user", &hash, true).await.unwrap();
    let dev_session = repo
        .create_session(dev.id, "2099-12-31T23:59:59")
        .await
        .unwrap();
    let spec = spec_with_paths("2.0.0", &["/promoted"]);
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header(
                    axum::http::header::AUTHORIZATION,
                    format!("Bearer {}", dev_session.token),
                )
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "producername": "promo-svc",
                        "stability": "snapshot",
                        "openapi_yaml": spec,
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    dev_session.token
}

#[cfg(test)]
async fn post_promote(
    app: &axum::Router,
    token: &str,
    producer: &str,
    version: &str,
) -> (StatusCode, serde_json::Value) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/admin/producers/{producer}/versions/openapi/{version}/promote"
                ))
                .header(
                    axum::http::header::AUTHORIZATION,
                    format!("Bearer {}", token),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

// The one-click promote (#28) is definitionally "provide the stored content
// as GA": in-place flip, original provider credited, VERSION_PROMOTED naming
// the promoting Actor, and an idempotent no-op the second time.
#[tokio::test]
async fn one_click_promote_releases_the_stored_snapshot() {
    let (app, repo, root_token) = app_with_seed().await;

    let _dev_token = seed_promo_snapshot(&app, &repo).await;

    let (status, body) = post_promote(&app, &root_token, "promo-svc", "2.0.0").await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body["promoted"], true);
    assert_eq!(body["stability"], "ga");

    let sid = repo.find_service("promo-svc").await.unwrap().unwrap();
    let meta = repo
        .find_spec_version(sid, ApiType::OpenApi, "2.0.0".parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(meta.stability, Stability::Ga);
    assert_eq!(
        meta.provided_by, "dev-user",
        "the stored content is byte-identical, so the snapshot's provider stays credited"
    );

    let logs = repo.get_audit_logs(all_audit_logs_filter()).await.unwrap();
    let promoted = logs
        .iter()
        .find(|l| l.action == "VERSION_PROMOTED" && l.service.as_deref() == Some("promo-svc"))
        .expect("promotion must be audited");
    assert_eq!(promoted.username, "root");

    // Double-click / racing colleague: the second promote is a harmless no-op.
    let (status, body) = post_promote(&app, &root_token, "promo-svc", "2.0.0").await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body["promoted"], false);
}

#[tokio::test]
async fn promote_of_an_unknown_version_is_not_found() {
    let (app, _repo, root_token) = app_with_seed().await;
    let (status, _) = post_promote(&app, &root_token, "demo-svc", "9.9.9").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = post_promote(&app, &root_token, "no-such-svc", "1.0.0").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// The button is hidden for non-releasers, but the guard is the boundary —
/// and because enforcement runs through the shared GA gate, the refusal is
/// instructive and audited, unlike a bare route-guard 403.
#[tokio::test]
async fn promote_without_release_ga_is_refused_and_audited() {
    let (app, repo, _root_token) = app_with_seed().await;

    let dev_token = seed_promo_snapshot(&app, &repo).await;

    let (status, body) = post_promote(&app, &dev_token, "promo-svc", "2.0.0").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        body["error"].as_str().unwrap_or("").contains("releaser"),
        "the refusal names the missing role: {body}"
    );

    let logs = repo.get_audit_logs(all_audit_logs_filter()).await.unwrap();
    let rejected = logs
        .iter()
        .find(|l| l.action == "VERSION_REJECTED" && l.service.as_deref() == Some("promo-svc"))
        .expect("an unauthorized promote is a real release attempt and must be audited");
    assert_eq!(rejected.username, "dev-user");
    assert_eq!(rejected.version.as_deref(), Some("2.0.0"));

    // Nothing changed.
    let sid = repo.find_service("promo-svc").await.unwrap().unwrap();
    let meta = repo
        .find_spec_version(sid, ApiType::OpenApi, "2.0.0".parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(meta.stability, Stability::Snapshot);
}

// A promotion carrying *different* content transfers attribution to the
// promoter — whoever pushed those bytes owns them.
#[tokio::test]
async fn different_content_promotion_transfers_the_provider() {
    let (app, repo, root_token) = app_with_seed().await;

    let hash = services::hash_password("pw").unwrap();
    let dev = repo.create_user("dev-user", &hash, true).await.unwrap();
    repo.grant_user_role(dev.id, "admin").await.unwrap();
    let dev_session = repo
        .create_session(dev.id, "2099-12-31T23:59:59")
        .await
        .unwrap();

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header(
                    axum::http::header::AUTHORIZATION,
                    format!("Bearer {}", dev_session.token),
                )
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "producername": "promo2-svc",
                        "stability": "snapshot",
                        "openapi_yaml": spec_with_paths("3.0.0", &["/a"]),
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);

    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header(
                    axum::http::header::AUTHORIZATION,
                    format!("Bearer {}", root_token),
                )
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "producername": "promo2-svc",
                        "stability": "ga",
                        "openapi_yaml": spec_with_paths("3.0.0", &["/a", "/b"]),
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);

    let sid = repo.find_service("promo2-svc").await.unwrap().unwrap();
    let meta = repo
        .find_spec_version(sid, ApiType::OpenApi, "3.0.0".parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(meta.stability, Stability::Ga);
    assert_eq!(
        meta.provided_by, "root",
        "different content: the promoter owns what they pushed"
    );
}
