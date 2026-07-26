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
