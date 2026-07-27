use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::Utc;
use sanshain_service::application::services;
use sanshain_service::domain::models::ApiType;
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
    }
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

    // Seed a demo service and endpoint via provide
    let openapi = r#"openapi: 3.0.0
info: {title: demo, version: v1}
paths:
  /hello:
    get:
      responses:
        '200': { description: ok }
"#;
    let _ = services::provide_spec(
        &repo,
        "demo-svc",
        "main",
        ApiType::OpenApi,
        openapi,
        None,
        false,
    )
    .await
    .unwrap();

    let app = create_app(test_state(repo.clone()));
    (app, repo, session.token)
}

#[tokio::test]
async fn admin_lists_and_cache_endpoints_work() {
    let (app, _repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    // List services (admin authenticated route)
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/services")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // List service endpoints
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/services/demo-svc/branches/main/endpoints")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Cache stats
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/settings/cache")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

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

#[tokio::test]
async fn test_favorites_api_endpoints() {
    let (app, _repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    // 1. Get initial favorites (should be empty lists)
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/auth/favorites")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 10000).await.unwrap();
    let favs: sanshain_service::domain::models::UserFavoritesResponse =
        serde_json::from_slice(&body).unwrap();
    assert!(favs.services.is_empty());
    assert!(favs.clients.is_empty());

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
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/auth/favorites")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 10000).await.unwrap();
    let favs: sanshain_service::domain::models::UserFavoritesResponse =
        serde_json::from_slice(&body).unwrap();
    assert_eq!(favs.services, vec!["demo-svc".to_string()]);

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
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/auth/favorites")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 10000).await.unwrap();
    let favs: sanshain_service::domain::models::UserFavoritesResponse =
        serde_json::from_slice(&body).unwrap();
    assert!(favs.services.is_empty());
}

// The dev-mode *settings* endpoint reports the persisted toggle (intent), not the
// gated effective value: enabling it reads back as `true` even though the
// ALLOW_INSECURE_DEV_MODE gate is closed in this test process. Regression guard —
// it briefly returned the gated value, which broke the admin toggle round-trip.
#[tokio::test]
async fn dev_mode_setting_reports_persisted_intent() {
    let (app, _repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);
    assert!(
        !services::dev_mode_gate_open(),
        "gate must be closed for this test to be meaningful"
    );

    // Enable dev mode via the admin settings endpoint (Bearer exempts CSRF).
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings/dev-mode")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"enabled":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Read it back: must reflect the persisted setting, not the gated value.
    let res = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/admin/settings/dev-mode")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 10000).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["dev_mode"], serde_json::Value::Bool(true));
}

#[cfg(test)]
async fn branch_last_modified(repo: &SqliteSpecRepository, branch: &str) -> String {
    use sanshain_service::domain::ports::SpecRepository;
    repo.list_branches_with_metadata()
        .await
        .unwrap()
        .into_iter()
        .find(|b| b.name == branch)
        .unwrap()
        .last_modified
}

// A branch's `updated_at` must track the last *publish*, not the last *read*.
// `ensure_branch` is on read paths, so it must no longer bump the timestamp;
// only `apply_spec_changes` (provide) advances it.
#[tokio::test]
async fn reads_do_not_advance_last_published_but_publishes_do() {
    use sanshain_service::domain::ports::SpecRepository;

    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    let spec = r#"openapi: 3.0.0
info: {title: t, version: v1}
paths:
  /p:
    get:
      responses:
        '200': { description: ok }
"#;

    // First publish establishes the branch timestamp.
    services::provide_spec(&repo, "svc", "main", ApiType::OpenApi, spec, None, false)
        .await
        .unwrap();
    let t1 = branch_last_modified(&repo, "main").await;

    // Cross a whole-second boundary (timestamps are second-granular).
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

    // A read-path call to `ensure_branch` must NOT advance `updated_at`.
    let sid = repo.ensure_service("svc").await.unwrap();
    let _ = repo.ensure_branch(sid, "main").await.unwrap();
    assert_eq!(
        branch_last_modified(&repo, "main").await,
        t1,
        "reading/ensuring a branch must not advance its last-published time"
    );

    // A publish that changes the spec MUST advance it.
    let spec2 = r#"openapi: 3.0.0
info: {title: t, version: v1}
paths:
  /p:
    get:
      responses:
        '200': { description: ok }
  /q:
    get:
      responses:
        '200': { description: ok }
"#;
    services::provide_spec(&repo, "svc", "main", ApiType::OpenApi, spec2, None, false)
        .await
        .unwrap();
    assert!(
        branch_last_modified(&repo, "main").await > t1,
        "a publish that changes the spec must advance the branch's last-published time"
    );
}

// Overview branch ordering: protected first, then most-recently-published, then name.
#[tokio::test]
async fn overview_branches_ordered_protected_then_recent() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    let spec = r#"openapi: 3.0.0
info: {title: t, version: v1}
paths:
  /p:
    get:
      responses:
        '200': { description: ok }
"#;
    // main (protected) and feature-old published first.
    services::provide_spec(&repo, "svc", "main", ApiType::OpenApi, spec, None, false)
        .await
        .unwrap();
    services::provide_spec(
        &repo,
        "svc",
        "feature-old",
        ApiType::OpenApi,
        spec,
        None,
        false,
    )
    .await
    .unwrap();

    // Cross a whole-second boundary so feature-new is strictly newer.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

    services::provide_spec(
        &repo,
        "svc",
        "feature-new",
        ApiType::OpenApi,
        spec,
        None,
        false,
    )
    .await
    .unwrap();

    let svcs = services::list_services_detailed(&repo, None).await.unwrap();
    let svc = svcs.iter().find(|s| s.name == "svc").unwrap();
    assert_eq!(
        svc.branches,
        vec!["main", "feature-new", "feature-old"],
        "protected first, then most-recently-published, then alphabetical"
    );
    // The per-branch last-published map is populated for display (#10).
    assert!(svc.branches_last_published.contains_key("feature-new"));
    assert!(
        svc.branches_last_published["feature-new"] > svc.branches_last_published["feature-old"],
        "feature-new was published later than feature-old"
    );
}

// Endpoint history for a branch the server never published falls back to the
// protected branch's history, instead of 404-ing.
#[tokio::test]
async fn version_history_falls_back_to_protected_branch() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    let spec = r#"openapi: 3.0.0
info: {title: t, version: v1}
paths:
  /p:
    get:
      responses:
        '200': { description: ok }
"#;
    // Publish only to the protected branch "main".
    services::provide_spec(&repo, "svc", "main", ApiType::OpenApi, spec, None, false)
        .await
        .unwrap();

    // Requesting history for a branch the server never published falls back to main.
    let versions = services::get_endpoint_version_history(
        &repo,
        "svc",
        "feature-x",
        ApiType::OpenApi,
        "/p",
        "GET",
    )
    .await
    .unwrap();
    assert!(
        !versions.is_empty(),
        "should fall back to the protected branch's history"
    );
    assert_eq!(
        versions[0].branch_name.as_deref(),
        Some("main"),
        "history served from the protected branch"
    );

    // A path that exists on no branch still errors (nothing to fall back to).
    let missing = services::get_endpoint_version_history(
        &repo,
        "svc",
        "feature-x",
        ApiType::OpenApi,
        "/nope",
        "GET",
    )
    .await;
    assert!(missing.is_err(), "no branch has this endpoint");
}

// A feature branch created from a protected branch with identical content
// genuinely has the endpoint (its own row, same yaml), but never accumulates its
// own `endpoint_versions` rows since only protected branches record history. It
// must not report "no history" — it should inherit the protected branch's real
// history, distinct from #13's case where the endpoint doesn't exist at all.
#[tokio::test]
async fn version_history_falls_back_when_local_branch_has_no_history_of_its_own() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    let spec = r#"openapi: 3.0.0
info: {title: t, version: v1}
paths:
  /p:
    get:
      responses:
        '200': { description: ok }
"#;
    // Two real versions on the protected branch "main".
    services::provide_spec(&repo, "svc", "main", ApiType::OpenApi, spec, None, false)
        .await
        .unwrap();
    let spec_v2 = r#"openapi: 3.0.0
info: {title: t, version: v2}
paths:
  /p:
    get:
      responses:
        '200': { description: ok }
  /q:
    get:
      responses:
        '200': { description: ok }
"#;
    services::provide_spec(&repo, "svc", "main", ApiType::OpenApi, spec_v2, None, false)
        .await
        .unwrap();

    // Branch "feature-x" actually holds the same endpoint content (a real row,
    // not a missing one) — but since it's not protected, no endpoint_versions
    // rows were ever written for it.
    services::provide_spec(
        &repo,
        "svc",
        "feature-x",
        ApiType::OpenApi,
        spec_v2,
        None,
        false,
    )
    .await
    .unwrap();

    let versions = services::get_endpoint_version_history(
        &repo,
        "svc",
        "feature-x",
        ApiType::OpenApi,
        "/p",
        "GET",
    )
    .await
    .unwrap();
    assert!(
        versions.len() >= 2,
        "must inherit main's real multi-version history, not report empty"
    );
    assert_eq!(
        versions[0].branch_name.as_deref(),
        Some("main"),
        "history is served from main, where it was actually recorded"
    );
}

// Only non-protected branches get a stale-cleanup expiry (protected branches are
// exempt from cleanup), and it is after the last publish.
#[tokio::test]
async fn overview_shows_expiry_for_non_protected_branches_only() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    let spec = r#"openapi: 3.0.0
info: {title: t, version: v1}
paths:
  /p:
    get:
      responses:
        '200': { description: ok }
"#;
    services::provide_spec(&repo, "svc", "main", ApiType::OpenApi, spec, None, false)
        .await
        .unwrap();
    services::provide_spec(&repo, "svc", "feature", ApiType::OpenApi, spec, None, false)
        .await
        .unwrap();

    let svcs = services::list_services_detailed(&repo, None).await.unwrap();
    let svc = svcs.iter().find(|s| s.name == "svc").unwrap();
    assert!(
        svc.branches_expire_at.contains_key("feature"),
        "a non-protected branch has a stale-cleanup expiry"
    );
    assert!(
        !svc.branches_expire_at.contains_key("main"),
        "a protected branch is exempt from cleanup, so has no expiry"
    );
    assert!(
        svc.branches_expire_at["feature"] > svc.branches_last_published["feature"],
        "expiry is after the last publish"
    );
}

// The full-spec view reassembles a branch's stored per-endpoint OpenAPI specs
// into one document; an api_type with no endpoints errors.
#[tokio::test]
async fn full_spec_merges_branch_endpoints() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    let spec = r#"openapi: 3.0.0
info: {title: t, version: v1}
paths:
  /a:
    get:
      responses:
        '200': { description: ok }
  /b:
    post:
      responses:
        '201': { description: created }
"#;
    services::provide_spec(&repo, "svc", "main", ApiType::OpenApi, spec, None, false)
        .await
        .unwrap();

    let full = services::get_full_spec(&repo, "svc", "main", ApiType::OpenApi)
        .await
        .unwrap();
    assert!(full.contains("/a"), "merged spec includes path /a");
    assert!(full.contains("/b"), "merged spec includes path /b");
    assert!(
        full.contains("openapi"),
        "reassembled as one OpenAPI document"
    );

    // No endpoints of the requested type -> NotFound.
    let err = services::get_full_spec(&repo, "svc", "main", ApiType::Proto).await;
    assert!(err.is_err(), "no proto endpoints on this branch");
}

// A wildcard protected pattern must protect the branches it covers, matching how
// stale-cleanup exempts them (SQLite GLOB). Exact patterns keep working.
#[tokio::test]
async fn wildcard_protected_pattern_matches_branches() {
    use sanshain_service::domain::ports::SpecRepository;

    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    // Defaults (exact) still work.
    assert!(repo.is_branch_protected("main").await.unwrap());
    assert!(!repo.is_branch_protected("feature-x").await.unwrap());

    repo.add_protected_branch("release/*").await.unwrap();
    assert!(
        repo.is_branch_protected("release/1.5").await.unwrap(),
        "a wildcard pattern protects matching branches"
    );
    assert!(
        !repo.is_branch_protected("feature-x").await.unwrap(),
        "a non-matching branch stays unprotected"
    );
    assert!(
        repo.is_branch_protected("main").await.unwrap(),
        "exact patterns are unaffected"
    );
}

// An admin manual endpoint edit is branch activity and must advance the branch's
// last-published time (so it does not look stale / get culled).
#[tokio::test]
async fn admin_edit_advances_last_published() {
    use sanshain_service::domain::ports::{SpecRepository, UpdateEndpointParams};

    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    let spec = r#"openapi: 3.0.0
info: {title: t, version: v1}
paths:
  /p:
    get:
      responses:
        '200': { description: ok }
"#;
    services::provide_spec(&repo, "svc", "main", ApiType::OpenApi, spec, None, false)
        .await
        .unwrap();
    let t1 = branch_last_modified(&repo, "main").await;

    // Cross a whole-second boundary so any advance is observable.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

    let sid = repo.ensure_service("svc").await.unwrap();
    let bid = repo.ensure_branch(sid, "main").await.unwrap();
    repo.update_endpoint(UpdateEndpointParams {
        branch_id: bid,
        api_type: ApiType::OpenApi,
        path: "/p",
        method: "GET",
        yaml_content: "openapi: 3.0.0\ninfo: {title: t, version: v2}\npaths: {}\n",
        deprecated: false,
        external: false,
    })
    .await
    .unwrap();

    assert!(
        branch_last_modified(&repo, "main").await > t1,
        "an admin manual edit must advance the branch's last-published time"
    );
}

// An OpenAPI spec with no paths is accepted (creates an empty branch), but the
// per-branch endpoint count must correctly report 0 for it, distinguishing it
// from a branch that actually serves something — the signal the discovery UI
// uses to hide branches/services that provide nothing.
#[tokio::test]
async fn empty_spec_branch_reports_zero_endpoint_count() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    let empty_spec = r#"openapi: 3.0.0
info: {title: empty, version: v1}
paths: {}
"#;
    services::provide_spec(
        &repo,
        "empty-svc",
        "main",
        ApiType::OpenApi,
        empty_spec,
        None,
        false,
    )
    .await
    .unwrap();

    let real_spec = r#"openapi: 3.0.0
info: {title: real, version: v1}
paths:
  /p:
    get:
      responses:
        '200': { description: ok }
"#;
    services::provide_spec(
        &repo,
        "real-svc",
        "main",
        ApiType::OpenApi,
        real_spec,
        None,
        false,
    )
    .await
    .unwrap();

    let svcs = services::list_services_detailed(&repo, None).await.unwrap();
    let empty = svcs.iter().find(|s| s.name == "empty-svc").unwrap();
    assert_eq!(
        empty.branches_endpoint_count.get("main").copied(),
        Some(0),
        "an empty-paths provide creates the branch, but with a reported 0 endpoint count"
    );

    let real = svcs.iter().find(|s| s.name == "real-svc").unwrap();
    assert_eq!(
        real.branches_endpoint_count.get("main").copied(),
        Some(1),
        "a branch with one endpoint reports count 1"
    );

    // `branches` itself stays unfiltered (raw truth for admin management) —
    // the empty branch is still listed, just reported as having 0 endpoints.
    assert!(empty.branches.contains(&"main".to_string()));
}

// Revoking a token that doesn't exist (or was already revoked) now correctly
// 404s instead of silently "succeeding" with a false audit entry. Revoking a
// real token succeeds, and neither audit entry names the token or its ID.
#[tokio::test]
async fn revoke_token_404s_when_not_found_and_audit_has_no_identifying_details() {
    use sanshain_service::domain::ports::SpecRepository;

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
    let body = axum::body::to_bytes(res.into_body(), 10000).await.unwrap();
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
    auth_service::register_user(&repo, "regular-user", "password123")
        .await
        .unwrap();
    let users = auth_service::list_users(&repo).await.unwrap();
    let regular = users.iter().find(|u| u.username == "regular-user").unwrap();
    assert!(!regular.is_admin, "newly registered users are not admins");
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

fn all_audit_logs_filter() -> sanshain_service::domain::models::AuditLogFilter {
    sanshain_service::domain::models::AuditLogFilter {
        from_date: None,
        to_date: None,
        action_type: None,
        service_wildcard: None,
        branch_wildcard: None,
        limit: 100,
    }
}

// A no-op re-provide (identical spec, zero endpoint changes) must not create a
// second PROVIDE_SPEC audit entry — audit noise reported during testing.
#[tokio::test]
async fn provide_without_changes_is_not_audited() {
    use sanshain_service::domain::ports::SpecRepository;

    let (app, repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    let spec = r#"openapi: 3.0.0
info: {title: audit-demo, version: v1}
paths:
  /ping:
    get:
      responses:
        '200': { description: ok }
"#;
    let body = serde_json::json!({
        "servicename": "audit-svc",
        "branch": "main",
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

// Fetching the branch report (used to render the branch view) must not create a
// REPORT audit entry — otherwise merely viewing a branch is audited.
#[tokio::test]
async fn viewing_report_is_not_audited() {
    use sanshain_service::domain::ports::SpecRepository;

    let (app, repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    let res = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/report?branch=main")
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
        "viewing a branch report must not create a REPORT audit entry"
    );
}

// --- Item #17: source_protected_branch / pull_from_branch (GitHub issue #1) ---

// `cfg(test)` is always true in this crate; the attribute marks the helper as
// test code for clippy's `allow-unwrap-in-tests`.
#[cfg(test)]
async fn spb_test_repo() -> SqliteSpecRepository {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();
    repo
}

const SPB_SPEC: &str = r#"openapi: 3.0.0
info: {title: t, version: v1}
paths:
  /p:
    get:
      responses:
        '200': { description: ok }
"#;

// First caller write sets source_protected_branch on a previously-unset branch.
#[tokio::test]
async fn source_protected_branch_first_write_sets_it() {
    use sanshain_service::domain::ports::SpecRepository;

    let repo = spb_test_repo().await;
    services::provide_spec_with_actor(
        &repo,
        services::ProvideSpecParams {
            servicename: "svc",
            branch: "hotfix/1.0.1",
            api_type: ApiType::OpenApi,
            content: SPB_SPEC,
            base_version: None,
            force: false,
            source_protected_branch: Some("release/1.0"),
            author: None,
        },
        Some("ci-bot"),
    )
    .await
    .unwrap();

    let sid = repo.ensure_service("svc").await.unwrap();
    let bid = repo
        .find_branch(sid, "hotfix/1.0.1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        repo.get_source_protected_branch(bid)
            .await
            .unwrap()
            .as_deref(),
        Some("release/1.0")
    );
}

// A second, differing caller-supplied value is ignored (stored value
// unchanged) and a mismatch log entry (tagged service/branch) is produced.
// `current_thread` flavor so the thread-local tracing subscriber reliably
// stays active across every .await in this test (see item #7's note on why a
// multi-threaded runtime would make this flaky).
#[tokio::test(flavor = "current_thread")]
async fn source_protected_branch_mismatch_is_logged_and_ignored() {
    use sanshain_service::domain::ports::SpecRepository;
    use sanshain_service::presentation::middleware::LogCaptureLayer;
    use tracing_subscriber::layer::SubscriberExt;

    let repo = spb_test_repo().await;
    services::provide_spec_with_actor(
        &repo,
        services::ProvideSpecParams {
            servicename: "svc",
            branch: "hotfix/1.0.1",
            api_type: ApiType::OpenApi,
            content: SPB_SPEC,
            base_version: None,
            force: false,
            source_protected_branch: Some("release/1.0"),
            author: None,
        },
        Some("ci-bot"),
    )
    .await
    .unwrap();

    let warn_buffer = Arc::new(std::sync::Mutex::new(VecDeque::new()));
    let layer = LogCaptureLayer {
        error_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
        warn_buffer: warn_buffer.clone(),
        info_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
        debug_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
        business_logic_debug: Arc::new(AtomicBool::new(true)),
        admin_user_debug: Arc::new(AtomicBool::new(true)),
    };
    let subscriber = tracing_subscriber::registry().with(layer);
    // `set_default` (RAII guard), not `with_default` (sync-closure-only): we're
    // on a `current_thread` runtime, so the thread-local subscriber stays
    // active across the `.await` below as long as the guard hasn't dropped yet.
    {
        let _guard = tracing::subscriber::set_default(subscriber);
        services::provide_spec_with_actor(
            &repo,
            services::ProvideSpecParams {
                servicename: "svc",
                branch: "hotfix/1.0.1",
                api_type: ApiType::OpenApi,
                content: SPB_SPEC,
                base_version: None,
                force: false,
                source_protected_branch: Some("master"),
                author: None,
            },
            Some("ci-bot"),
        )
        .await
        .unwrap();
    }

    let sid = repo.ensure_service("svc").await.unwrap();
    let bid = repo
        .find_branch(sid, "hotfix/1.0.1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        repo.get_source_protected_branch(bid)
            .await
            .unwrap()
            .as_deref(),
        Some("release/1.0"),
        "stored value must be unchanged by the differing second write"
    );

    let warns = warn_buffer.lock().unwrap();
    assert_eq!(warns.len(), 1, "exactly one mismatch warning expected");
    assert_eq!(warns[0].service.as_deref(), Some("svc"));
    assert_eq!(warns[0].branch.as_deref(), Some("hotfix/1.0.1"));
    assert!(warns[0].message.contains("mismatch"));
}

// A second, matching caller-supplied value is a no-op (no spurious log entry).
#[tokio::test(flavor = "current_thread")]
async fn source_protected_branch_matching_resupply_is_noop_no_log() {
    use sanshain_service::domain::ports::SpecRepository;
    use sanshain_service::presentation::middleware::LogCaptureLayer;
    use tracing_subscriber::layer::SubscriberExt;

    let repo = spb_test_repo().await;
    services::provide_spec_with_actor(
        &repo,
        services::ProvideSpecParams {
            servicename: "svc",
            branch: "hotfix/1.0.1",
            api_type: ApiType::OpenApi,
            content: SPB_SPEC,
            base_version: None,
            force: false,
            source_protected_branch: Some("release/1.0"),
            author: None,
        },
        Some("ci-bot"),
    )
    .await
    .unwrap();

    let warn_buffer = Arc::new(std::sync::Mutex::new(VecDeque::new()));
    let layer = LogCaptureLayer {
        error_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
        warn_buffer: warn_buffer.clone(),
        info_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
        debug_buffer: Arc::new(std::sync::Mutex::new(VecDeque::new())),
        business_logic_debug: Arc::new(AtomicBool::new(true)),
        admin_user_debug: Arc::new(AtomicBool::new(true)),
    };
    let subscriber = tracing_subscriber::registry().with(layer);
    // Re-provide identical content + a matching source_protected_branch.
    // apply_source_protected_branch_hint runs regardless of whether the spec
    // content itself changed (it happens before the #9 no-op short-circuit),
    // so this still exercises the "matching value" comparison path.
    {
        let _guard = tracing::subscriber::set_default(subscriber);
        services::provide_spec_with_actor(
            &repo,
            services::ProvideSpecParams {
                servicename: "svc",
                branch: "hotfix/1.0.1",
                api_type: ApiType::OpenApi,
                content: SPB_SPEC,
                base_version: None,
                force: false,
                source_protected_branch: Some("release/1.0"),
                author: None,
            },
            Some("ci-bot"),
        )
        .await
        .unwrap();
    }

    let sid = repo.ensure_service("svc").await.unwrap();
    let bid = repo
        .find_branch(sid, "hotfix/1.0.1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        repo.get_source_protected_branch(bid)
            .await
            .unwrap()
            .as_deref(),
        Some("release/1.0")
    );
    assert!(
        warn_buffer.lock().unwrap().is_empty(),
        "a matching resupply must not log a mismatch"
    );
}

// An admin correction always overwrites, regardless of what's currently
// stored, and a later differing caller-supplied value cannot undo it (only
// an admin can change it once set).
#[tokio::test]
async fn admin_correction_always_overwrites_source_protected_branch() {
    use sanshain_service::domain::ports::SpecRepository;

    let repo = spb_test_repo().await;
    services::provide_spec_with_actor(
        &repo,
        services::ProvideSpecParams {
            servicename: "svc",
            branch: "hotfix/1.0.1",
            api_type: ApiType::OpenApi,
            content: SPB_SPEC,
            base_version: None,
            force: false,
            source_protected_branch: Some("release/1.0"),
            author: None,
        },
        Some("ci-bot"),
    )
    .await
    .unwrap();

    services::admin_set_source_protected_branch(&repo, "svc", "hotfix/1.0.1", Some("release/2.0"))
        .await
        .unwrap();

    let sid = repo.ensure_service("svc").await.unwrap();
    let bid = repo
        .find_branch(sid, "hotfix/1.0.1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        repo.get_source_protected_branch(bid)
            .await
            .unwrap()
            .as_deref(),
        Some("release/2.0"),
        "admin correction must overwrite the caller-set value"
    );

    // A later differing caller-supplied value must not undo the admin's fix.
    services::provide_spec_with_actor(
        &repo,
        services::ProvideSpecParams {
            servicename: "svc",
            branch: "hotfix/1.0.1",
            api_type: ApiType::OpenApi,
            content: SPB_SPEC,
            base_version: None,
            force: false,
            source_protected_branch: Some("master"),
            author: None,
        },
        Some("ci-bot"),
    )
    .await
    .unwrap();
    assert_eq!(
        repo.get_source_protected_branch(bid)
            .await
            .unwrap()
            .as_deref(),
        Some("release/2.0"),
        "only an admin may change an already-set value"
    );
}

// pull_from_branch bypasses source_protected_branch/fallback resolution
// entirely and does not persist anything (no branch row is even created for
// the requesting branch).
#[tokio::test]
async fn pull_from_branch_bypasses_resolution_without_persisting() {
    use sanshain_service::domain::ports::SpecRepository;

    let repo = spb_test_repo().await;
    // Only "release/1.0" (protected) has the endpoint.
    repo.add_protected_branch("release/1.0").await.unwrap();
    services::provide_spec(
        &repo,
        "svc",
        "release/1.0",
        ApiType::OpenApi,
        SPB_SPEC,
        None,
        false,
    )
    .await
    .unwrap();

    let res = services::require_endpoint(
        &repo,
        None,
        services::RequireEndpointParams {
            clientname: "client-a",
            servicename: "svc",
            branch: "feature-x",
            api_type: ApiType::OpenApi,
            path: "/p",
            method: "GET",
            timeout_secs: None,
            source_protected_branch: None,
            pull_from_branch: Some("release/1.0"),
        },
    )
    .await
    .unwrap();
    assert!(res.yaml.contains("/p"));

    // No branch row was created for "feature-x" — pull_from_branch has zero
    // persistence side effects.
    let sid = repo.ensure_service("svc").await.unwrap();
    assert!(
        repo.find_branch(sid, "feature-x").await.unwrap().is_none(),
        "pull_from_branch must not create or touch a branch row for the requesting branch"
    );
}

// Regression guard: requiring against a branch that IS itself protected, with
// no data, still resolves to "not found" — never silently substituting a
// different protected branch. Confirms item #17's changes did not weaken
// this pre-existing, already-correct invariant.
#[tokio::test]
async fn protected_branch_with_no_data_still_not_found() {
    use sanshain_service::domain::ports::SpecRepository;

    let repo = spb_test_repo().await;
    repo.add_protected_branch("release/2.0").await.unwrap();
    // Some other protected branch DOES have the endpoint — must not leak here.
    services::provide_spec(
        &repo,
        "svc",
        "main",
        ApiType::OpenApi,
        SPB_SPEC,
        None,
        false,
    )
    .await
    .unwrap();

    let result = services::require_endpoint(
        &repo,
        None,
        services::RequireEndpointParams {
            clientname: "client-a",
            servicename: "svc",
            branch: "release/2.0",
            api_type: ApiType::OpenApi,
            path: "/p",
            method: "GET",
            timeout_secs: None,
            source_protected_branch: None,
            pull_from_branch: None,
        },
    )
    .await;
    assert!(
        result.is_err(),
        "a protected branch with no data must not substitute another protected branch"
    );
}

// get_endpoint_version_history prefers a branch's own source_protected_branch
// over the plain protected-branch fallback chain, even when a different
// protected branch would otherwise win alphabetically.
#[tokio::test]
async fn version_history_prefers_source_protected_branch_over_alphabetical() {
    use sanshain_service::domain::ports::SpecRepository;

    let repo = spb_test_repo().await;
    // "main" sorts before "release/1.0" alphabetically, and both are protected
    // with real (different) history — without source_protected_branch, the
    // plain chain would incorrectly prefer "main".
    services::provide_spec(
        &repo,
        "svc",
        "main",
        ApiType::OpenApi,
        SPB_SPEC,
        None,
        false,
    )
    .await
    .unwrap();
    repo.add_protected_branch("release/1.0").await.unwrap();
    services::provide_spec(
        &repo,
        "svc",
        "release/1.0",
        ApiType::OpenApi,
        SPB_SPEC,
        None,
        false,
    )
    .await
    .unwrap();

    // The admin-set endpoint requires the branch row to already exist.
    let sid = repo.ensure_service("svc").await.unwrap();
    repo.ensure_branch(sid, "hotfix/1.0.1").await.unwrap();
    services::admin_set_source_protected_branch(&repo, "svc", "hotfix/1.0.1", Some("release/1.0"))
        .await
        .unwrap();

    let versions = services::get_endpoint_version_history(
        &repo,
        "svc",
        "hotfix/1.0.1",
        ApiType::OpenApi,
        "/p",
        "GET",
    )
    .await
    .unwrap();
    assert!(!versions.is_empty());
    assert_eq!(
        versions[0].branch_name.as_deref(),
        Some("release/1.0"),
        "source_protected_branch must be preferred over the alphabetically-earlier 'main'"
    );
}

// HTTP-contract level: GET/PUT the admin source_protected_branch endpoint
// through the real router, and confirm a non-admin gets 403 (matching #18's
// admin_auth-gating test pattern).
#[tokio::test]
async fn admin_source_protected_branch_endpoint_is_admin_only_and_works() {
    use sanshain_service::application::auth_service;
    use sanshain_service::domain::models::AuthMode;

    let (app, repo, admin_token) = app_with_seed().await;
    let admin_auth = format!("Bearer {}", admin_token);
    let uri = "/admin/services/demo-svc/branches/main/source-protected-branch";

    // GET: initially unset.
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
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 10000).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["source_protected_branch"], serde_json::Value::Null);

    // PUT: set it.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(uri)
                .header(axum::http::header::AUTHORIZATION, &admin_auth)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({"source_protected_branch": "release/1.0"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // GET again: now set.
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
    let body = axum::body::to_bytes(res.into_body(), 10000).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["source_protected_branch"], "release/1.0");

    // A non-admin is forbidden from both GET and PUT.
    auth_service::set_auth_mode(&repo, &AuthMode::Local)
        .await
        .unwrap();
    auth_service::register_user(&repo, "regular-user", "password123")
        .await
        .unwrap();
    let users = auth_service::list_users(&repo).await.unwrap();
    let regular = users.iter().find(|u| u.username == "regular-user").unwrap();
    auth_service::approve_user(&repo, regular.id).await.unwrap();
    let (session, _user) = auth_service::login(&repo, "regular-user", "password123")
        .await
        .unwrap();
    let regular_auth = format!("Bearer {}", session.token);

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
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    let res = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(uri)
                .header(axum::http::header::AUTHORIZATION, &regular_auth)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({"source_protected_branch": "release/2.0"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

// --- Item #15: `author` provide override (blame only, not the audit log) ---

// A client-supplied `author` overrides who gets credited in the endpoint's
// version history (blame), but the audit log must still record the real
// authenticated caller — the two must never be conflated.
#[tokio::test]
async fn author_override_affects_blame_but_not_audit_log() {
    use sanshain_service::domain::ports::SpecRepository;

    let (app, repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    let spec = r#"openapi: 3.0.0
info: {title: demo, version: v1}
paths:
  /hello:
    get:
      responses:
        '200': { description: ok }
  /blame-test:
    get:
      responses:
        '200': { description: ok }
"#;
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "servicename": "demo-svc",
                        "branch": "main",
                        "openapi_yaml": spec,
                        "author": "external.author@example.com",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);

    // Blame: the version history for the new endpoint must credit the
    // client-supplied author, not the real authenticated caller ("root").
    let versions = services::get_endpoint_version_history(
        &repo,
        "demo-svc",
        "main",
        ApiType::OpenApi,
        "/blame-test",
        "GET",
    )
    .await
    .unwrap();
    assert_eq!(
        versions[0].username.as_deref(),
        Some("external.author@example.com"),
        "author override must be used for version-history blame"
    );

    // Audit log: must still record the real authenticated caller, never the
    // client-supplied author.
    let logs = repo.get_audit_logs(all_audit_logs_filter()).await.unwrap();
    let provide_log = logs
        .iter()
        .find(|l| l.action == "PROVIDE_SPEC" && l.service.as_deref() == Some("demo-svc"))
        .expect("provide must be audited");
    assert_eq!(
        provide_log.username, "root",
        "the audit log must record the real authenticated caller, not the author override"
    );
}

// Without an `author` override, blame falls back to the real authenticated
// caller — unchanged prior behavior.
#[tokio::test]
async fn author_absent_falls_back_to_real_username_for_blame() {
    let (app, repo, token) = app_with_seed().await;
    let auth = format!("Bearer {}", token);

    let spec = r#"openapi: 3.0.0
info: {title: demo, version: v1}
paths:
  /hello:
    get:
      responses:
        '200': { description: ok }
  /no-author:
    get:
      responses:
        '200': { description: ok }
"#;
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header(axum::http::header::AUTHORIZATION, &auth)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "servicename": "demo-svc",
                        "branch": "main",
                        "openapi_yaml": spec,
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);

    let versions = services::get_endpoint_version_history(
        &repo,
        "demo-svc",
        "main",
        ApiType::OpenApi,
        "/no-author",
        "GET",
    )
    .await
    .unwrap();
    assert_eq!(versions[0].username.as_deref(), Some("root"));
}

// --- /require-bundle must resolve lineage exactly like /require ---

// Same endpoint, distinguishable content per branch, so a test can tell which
// branch actually served the bundle.
const SPB_SPEC_FROM_RELEASE: &str = r#"openapi: 3.0.0
info: {title: t, version: v1}
paths:
  /p:
    get:
      responses:
        '200': { description: served-by-release }
"#;

const SPB_SPEC_FROM_MAIN: &str = r#"openapi: 3.0.0
info: {title: t, version: v1}
paths:
  /p:
    get:
      responses:
        '200': { description: served-by-main }
"#;

// A bundle require from a feature branch must follow that branch's
// source_protected_branch, not the alphabetically-first protected branch.
// Before this was wired up, /require and /require-bundle disagreed: the single
// endpoint resolved against release/1.0 while the bundle silently returned
// main's diverged API.
#[tokio::test]
async fn require_bundle_prefers_source_protected_branch_over_alphabetical() {
    use sanshain_service::domain::ports::SpecRepository;

    let repo = spb_test_repo().await;
    // "main" sorts before "release/1.0"; both protected, both hold /p with
    // different content.
    services::provide_spec(
        &repo,
        "svc",
        "main",
        ApiType::OpenApi,
        SPB_SPEC_FROM_MAIN,
        None,
        false,
    )
    .await
    .unwrap();
    repo.add_protected_branch("release/1.0").await.unwrap();
    services::provide_spec(
        &repo,
        "svc",
        "release/1.0",
        ApiType::OpenApi,
        SPB_SPEC_FROM_RELEASE,
        None,
        false,
    )
    .await
    .unwrap();

    let eps = vec![("/p".to_string(), "GET".to_string())];
    let res = services::require_bundle(
        &repo,
        None,
        services::RequireBundleParams {
            clientname: "client-a",
            servicename: "svc",
            branch: "hotfix/1.0.1",
            api_type: ApiType::OpenApi,
            endpoints: &eps,
            timeout_secs: None,
            source_protected_branch: Some("release/1.0"),
            pull_from_branch: None,
        },
    )
    .await
    .unwrap();
    assert!(
        res.yaml.contains("served-by-release"),
        "bundle must resolve against source_protected_branch, got: {}",
        res.yaml
    );

    // The hint was persisted by the bundle path, exactly as /require does.
    let sid = repo.ensure_service("svc").await.unwrap();
    let bid = repo
        .find_branch(sid, "hotfix/1.0.1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        repo.get_source_protected_branch(bid)
            .await
            .unwrap()
            .as_deref(),
        Some("release/1.0")
    );
}

// pull_from_branch pins a bundle request to an exact branch and persists
// nothing — matching the single-endpoint behavior.
#[tokio::test]
async fn require_bundle_pull_from_branch_bypasses_resolution_without_persisting() {
    use sanshain_service::domain::ports::SpecRepository;

    let repo = spb_test_repo().await;
    services::provide_spec(
        &repo,
        "svc",
        "main",
        ApiType::OpenApi,
        SPB_SPEC_FROM_MAIN,
        None,
        false,
    )
    .await
    .unwrap();
    repo.add_protected_branch("release/1.0").await.unwrap();
    services::provide_spec(
        &repo,
        "svc",
        "release/1.0",
        ApiType::OpenApi,
        SPB_SPEC_FROM_RELEASE,
        None,
        false,
    )
    .await
    .unwrap();

    let eps = vec![("/p".to_string(), "GET".to_string())];
    let res = services::require_bundle(
        &repo,
        None,
        services::RequireBundleParams {
            clientname: "client-a",
            servicename: "svc",
            branch: "feature-x",
            api_type: ApiType::OpenApi,
            endpoints: &eps,
            timeout_secs: None,
            source_protected_branch: None,
            pull_from_branch: Some("release/1.0"),
        },
    )
    .await
    .unwrap();
    assert!(res.yaml.contains("served-by-release"));

    let sid = repo.ensure_service("svc").await.unwrap();
    assert!(
        repo.find_branch(sid, "feature-x").await.unwrap().is_none(),
        "pull_from_branch must not create a branch row on the bundle path either"
    );
}

// A protected branch that holds the endpoint but has no version rows of its
// own must not inherit a *different* protected branch's history — that would
// show another release line's changes under this branch's name. It reports an
// empty history instead.
#[tokio::test]
async fn protected_branch_without_history_does_not_inherit_another_protected_branch() {
    use sanshain_service::domain::ports::SpecRepository;

    let repo = spb_test_repo().await;
    // "main" is protected and accumulates real version rows.
    services::provide_spec(
        &repo,
        "svc",
        "main",
        ApiType::OpenApi,
        SPB_SPEC_FROM_MAIN,
        None,
        false,
    )
    .await
    .unwrap();
    let versions_on_main =
        services::get_endpoint_version_history(&repo, "svc", "main", ApiType::OpenApi, "/p", "GET")
            .await
            .unwrap();
    assert!(
        !versions_on_main.is_empty(),
        "precondition: main must have real history"
    );

    // "release/1.0" is protected and holds the endpoint, but its endpoint row is
    // created directly so it has no version rows of its own.
    repo.add_protected_branch("release/1.0").await.unwrap();
    let sid = repo.ensure_service("svc").await.unwrap();
    let bid = repo.ensure_branch(sid, "release/1.0").await.unwrap();
    repo.insert_endpoint(
        bid,
        &sanshain_service::domain::models::EndpointRecord {
            id: None,
            api_type: ApiType::OpenApi,
            path: "/p".to_string(),
            normalized_path: "/p".to_string(),
            method: "GET".to_string(),
            yaml_content: SPB_SPEC_FROM_RELEASE.to_string(),
            deprecated: false,
            external: false,
        },
    )
    .await
    .unwrap();

    let versions = services::get_endpoint_version_history(
        &repo,
        "svc",
        "release/1.0",
        ApiType::OpenApi,
        "/p",
        "GET",
    )
    .await
    .unwrap();
    assert!(
        versions.is_empty(),
        "a protected branch must not inherit another protected branch's history, got: {:?}",
        versions
            .iter()
            .map(|v| v.branch_name.as_deref())
            .collect::<Vec<_>>()
    );
}

// A wildcard protected pattern (release/*) must actually participate in
// fallback resolution. `list_protected_branches` returns patterns, so before
// the patterns were expanded over real branch names the resolver looked up a
// branch literally named "release/*" and found nothing.
#[tokio::test]
async fn wildcard_protected_pattern_resolves_as_fallback() {
    use sanshain_service::domain::ports::SpecRepository;

    let repo = spb_test_repo().await;
    repo.add_protected_branch("release/*").await.unwrap();
    // Remove the seeded exact patterns so only the wildcard can match.
    repo.remove_protected_branch("main").await.unwrap();
    repo.remove_protected_branch("master").await.unwrap();

    services::provide_spec(
        &repo,
        "svc",
        "release/1.0",
        ApiType::OpenApi,
        SPB_SPEC_FROM_RELEASE,
        None,
        false,
    )
    .await
    .unwrap();

    // A feature branch with nothing of its own must fall back to the
    // wildcard-protected release/1.0.
    let res = services::require_endpoint(
        &repo,
        None,
        services::RequireEndpointParams {
            clientname: "client-a",
            servicename: "svc",
            branch: "feature-x",
            api_type: ApiType::OpenApi,
            path: "/p",
            method: "GET",
            timeout_secs: None,
            source_protected_branch: None,
            pull_from_branch: None,
        },
    )
    .await
    .unwrap();
    assert!(
        res.yaml.contains("served-by-release"),
        "a wildcard-protected branch must be reachable as a fallback target"
    );
}
