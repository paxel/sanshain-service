//! The core behavioral suite of the 2.0 version-line model (ADR-0003), at the
//! router seam: producer-declared versions, declared stability, GA
//! immutability, mislabel checking, exact-pin resolution and use-based
//! snapshot expiry.

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use chrono::Utc;
use sanshain_service::application::services;
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
    pool: sqlx::SqlitePool,
    /// Session token of a fully privileged user, for both API and admin calls.
    token: String,
}

// `cfg(test)` is always true in this crate; the attribute marks the helpers as
// test code for clippy's `allow-unwrap-in-tests`.
#[cfg(test)]
async fn setup() -> TestContext {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool.clone());
    repo.run_migrations().await.unwrap();

    // A real seeded user with a real session token — no auth bypasses.
    let hash = services::hash_password("root-pass").unwrap();
    let user = repo.create_user("root", &hash, true).await.unwrap();
    repo.grant_user_role(user.id, "admin").await.unwrap();
    let session = repo
        .create_session(user.id, "2099-12-31T23:59:59")
        .await
        .unwrap();

    let app = create_app(test_app_state(repo));
    TestContext {
        app,
        pool,
        token: session.token,
    }
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
async fn provide(
    ctx: &TestContext,
    producer: &str,
    stability: &str,
    yaml: &str,
) -> (StatusCode, Value) {
    let payload = json!({
        "producername": producer,
        "openapi_yaml": yaml,
        "stability": stability,
    });
    let (status, _, body) = send(ctx, "POST", "/provide", Some(payload), &[]).await;
    (status, as_json(&body))
}

#[cfg(test)]
async fn require(
    ctx: &TestContext,
    consumer: &str,
    producer: &str,
    version: &str,
    path: &str,
    method: &str,
) -> (StatusCode, HeaderMap, String) {
    let uri = format!(
        "/require?consumername={}&producername={}&version={}&path={}&method={}",
        consumer, producer, version, path, method
    );
    send(ctx, "GET", &uri, None, &[]).await
}

/// A spec with `/users` and `/orders`.
#[cfg(test)]
fn spec_two_endpoints(version: &str) -> String {
    format!(
        "openapi: 3.0.3\ninfo:\n  title: Fixture\n  version: {version}\npaths:\n  /users:\n    get:\n      responses:\n        '200':\n          description: OK\n  /orders:\n    get:\n      responses:\n        '200':\n          description: OK\n"
    )
}

/// The same spec with `/orders` removed — breaking relative to `spec_two_endpoints`.
#[cfg(test)]
fn spec_users_only(version: &str) -> String {
    format!(
        "openapi: 3.0.3\ninfo:\n  title: Fixture\n  version: {version}\npaths:\n  /users:\n    get:\n      responses:\n        '200':\n          description: OK\n"
    )
}

/// Same endpoint set as `spec_two_endpoints` but with a changed description —
/// different bytes, compatible shape.
#[cfg(test)]
fn spec_two_endpoints_reworded(version: &str) -> String {
    spec_two_endpoints(version).replace("description: OK", "description: Fine")
}

// ---------------------------------------------------------------------------
// Rule 1: the version comes from the spec document itself.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn provide_rejects_missing_or_loose_versions_with_guidance() {
    let ctx = setup().await;

    // No info.version at all.
    let no_version = "openapi: 3.0.3\ninfo:\n  title: T\npaths:\n  /a:\n    get:\n      responses:\n        '200':\n          description: OK\n";
    let (status, body) = provide(&ctx, "vsvc", "snapshot", no_version).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"].as_str().unwrap().contains("no info.version"),
        "got: {body}"
    );

    // Loose two-part version.
    let (status, body) = provide(&ctx, "vsvc", "snapshot", &spec_users_only("'1.0'")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("MAJOR.MINOR.PATCH"),
        "got: {body}"
    );

    // v-prefix.
    let (status, body) = provide(&ctx, "vsvc", "snapshot", &spec_users_only("v2.0.0")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("drop the 'v' prefix"),
        "got: {body}"
    );

    // -SNAPSHOT suffix: the message must point at the stability flag instead.
    let (status, body) =
        provide(&ctx, "vsvc", "snapshot", &spec_users_only("1.2.0-SNAPSHOT")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let message = body["error"].as_str().unwrap();
    assert!(message.contains("stability"), "got: {message}");
    assert!(message.contains("1.2.0"), "got: {message}");

    // Nothing was stored by any of the rejections.
    let (status, _, _) = send(&ctx, "GET", "/producers/vsvc/versions", None, &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn proto_provide_requires_the_version_marker_comment() {
    let ctx = setup().await;

    let without_marker = "syntax = \"proto3\";\npackage t;\nmessage Req {}\nmessage Res {}\nservice Svc { rpc Do (Req) returns (Res); }\n";
    let payload = json!({
        "producername": "grpc-svc",
        "proto_content": without_marker,
        "stability": "snapshot",
    });
    let (status, _, body) = send(&ctx, "POST", "/provide/grpc", Some(payload), &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.contains("// sanshain-version: MAJOR.MINOR.PATCH"),
        "the error must show the expected marker syntax, got: {body}"
    );

    let with_marker = format!("// sanshain-version: 1.2.0\n{without_marker}");
    let payload = json!({
        "producername": "grpc-svc",
        "proto_content": with_marker,
        "stability": "snapshot",
    });
    let (status, _, body) = send(&ctx, "POST", "/provide/grpc", Some(payload), &[]).await;
    assert_eq!(status, StatusCode::ACCEPTED, "got: {body}");
    let json = as_json(&body);
    assert_eq!(json["version"], "1.2.0");
    assert_eq!(json["stability"], "snapshot");
    assert_eq!(json["changes"]["inserts"], 1);
}

// ---------------------------------------------------------------------------
// Rule 2: `stability` is required; 1.x fields are rejected by name.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn provide_without_stability_is_rejected_naming_the_field() {
    let ctx = setup().await;
    let payload = json!({
        "producername": "svc",
        "openapi_yaml": spec_users_only("1.0.0"),
    });
    let (status, _, body) = send(&ctx, "POST", "/provide", Some(payload), &[]).await;
    // Axum renders a JSON body that fails deserialization as 422.
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        body.contains("missing field `stability`"),
        "the rejection must name the missing field, got: {body}"
    );
}

#[tokio::test]
async fn provide_rejects_branch_era_fields_by_name() {
    let ctx = setup().await;
    for field in ["branch", "base_version", "force", "source_protected_branch"] {
        let mut payload = json!({
            "producername": "svc",
            "openapi_yaml": spec_users_only("1.0.0"),
            "stability": "snapshot",
        });
        payload[field] = json!("x");
        let (status, _, body) = send(&ctx, "POST", "/provide", Some(payload), &[]).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "field {field}: {body}"
        );
        assert!(
            body.contains(&format!("unknown field `{field}`")),
            "the rejection must name `{field}`, got: {body}"
        );
    }

    // Nothing was stored by any of the rejections.
    let (status, _, _) = send(&ctx, "GET", "/producers/svc/versions", None, &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn require_rejects_branch_era_parameters_by_name() {
    let ctx = setup().await;
    provide(&ctx, "svc", "ga", &spec_users_only("1.0.0")).await;

    for param in [
        "branch",
        "timeout",
        "pull_from_branch",
        "source_protected_branch",
    ] {
        let uri = format!(
            "/require?consumername=c&producername=svc&version=1.0.0&path=/users&method=GET&{param}=x"
        );
        let (status, _, body) = send(&ctx, "GET", &uri, None, &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "param {param}: {body}");
        assert!(
            body.contains(&format!("unknown field `{param}`")),
            "the rejection must name `{param}`, got: {body}"
        );
    }
}

// ---------------------------------------------------------------------------
// Rules 3 + 4: snapshot overwrite is wholesale; identical content is a no-op.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn snapshot_overwrite_replaces_the_endpoint_set_wholesale() {
    let ctx = setup().await;

    let (status, body) = provide(&ctx, "svc", "snapshot", &spec_two_endpoints("1.0.0")).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body["changes"]["inserts"], 2);

    // The dropped endpoint is served before the overwrite…
    let (status, _, _) = require(&ctx, "cons", "svc", "1.0.0", "/orders", "GET").await;
    assert_eq!(status, StatusCode::OK);

    // …then the same version is overwritten without it.
    let (status, body) = provide(&ctx, "svc", "snapshot", &spec_users_only("1.0.0")).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body["changes"]["inserts"], 0);
    assert_eq!(body["changes"]["updates"], 0);
    assert_eq!(body["changes"]["deletes"], 1);

    // Really gone: the version exists, the endpoint deliberately does not.
    let (status, _, body) = require(&ctx, "cons", "svc", "1.0.0", "/orders", "GET").await;
    assert_eq!(status, StatusCode::GONE, "got: {body}");
    let (status, _, _) = require(&ctx, "cons", "svc", "1.0.0", "/users", "GET").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn identical_content_reprovide_is_a_no_op_for_both_stabilities() {
    let ctx = setup().await;

    // Snapshot line.
    let yaml = spec_two_endpoints("1.0.0");
    provide(&ctx, "snap-svc", "snapshot", &yaml).await;
    let (status, body) = provide(&ctx, "snap-svc", "snapshot", &yaml).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body["changes"]["inserts"], 0);
    assert_eq!(body["changes"]["updates"], 0);
    assert_eq!(body["changes"]["deletes"], 0);
    assert_eq!(body["stability"], "snapshot");

    // GA line: identical content is a no-op, not an immutability violation.
    let ga_yaml = spec_two_endpoints("2.0.0");
    provide(&ctx, "ga-svc", "ga", &ga_yaml).await;
    let (status, body) = provide(&ctx, "ga-svc", "ga", &ga_yaml).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body["changes"]["inserts"], 0);
    assert_eq!(body["changes"]["updates"], 0);
    assert_eq!(body["changes"]["deletes"], 0);
    assert_eq!(body["stability"], "ga");
}

// ---------------------------------------------------------------------------
// Rule 5: GA immutability, with a machine-readable remedy.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ga_with_different_content_is_rejected_with_a_free_proposed_version() {
    let ctx = setup().await;
    provide(&ctx, "svc", "ga", &spec_two_endpoints("1.0.0")).await;

    let (status, body) = provide(&ctx, "svc", "ga", &spec_two_endpoints_reworded("1.0.0")).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let message = body["error"].as_str().unwrap();
    assert!(message.contains("immutable"), "got: {message}");
    // Compatible rewording → patch bump proposal, and 1.0.1 is free.
    assert_eq!(body["proposed_version"], "1.0.1");
}

#[tokio::test]
async fn snapshot_for_a_ga_number_is_rejected_forever() {
    let ctx = setup().await;
    provide(&ctx, "svc", "ga", &spec_two_endpoints("1.0.0")).await;

    let (status, body) = provide(
        &ctx,
        "svc",
        "snapshot",
        &spec_two_endpoints_reworded("1.0.0"),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let message = body["error"].as_str().unwrap();
    assert!(
        message.contains("can never carry a snapshot again"),
        "got: {message}"
    );
    assert_eq!(body["proposed_version"], "1.0.1");
}

// ---------------------------------------------------------------------------
// Rule 6: promotion flips the stability in place.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn promotion_flips_snapshot_to_ga_preserving_created_at() {
    let ctx = setup().await;
    let yaml = spec_two_endpoints("1.0.0");
    provide(&ctx, "svc", "snapshot", &yaml).await;

    let (_, _, body) = send(&ctx, "GET", "/producers/svc/versions", None, &[]).await;
    let listing = as_json(&body);
    assert_eq!(listing.as_array().unwrap().len(), 1);
    assert_eq!(listing[0]["stability"], "snapshot");
    let created_at = listing[0]["created_at"].as_str().unwrap().to_string();

    // Promote with the *same* content — the normal release flow.
    let (status, body) = provide(&ctx, "svc", "ga", &yaml).await;
    assert_eq!(status, StatusCode::ACCEPTED, "got: {body}");
    assert_eq!(body["stability"], "ga");
    assert_eq!(body["version"], "1.0.0");

    let (_, _, body) = send(&ctx, "GET", "/producers/svc/versions", None, &[]).await;
    let listing = as_json(&body);
    assert_eq!(
        listing.as_array().unwrap().len(),
        1,
        "promotion must not add a second entry: {listing}"
    );
    assert_eq!(listing[0]["version"], "1.0.0");
    assert_eq!(listing[0]["stability"], "ga");
    assert_eq!(
        listing[0]["created_at"].as_str().unwrap(),
        created_at,
        "promotion keeps the row's identity"
    );
}

#[tokio::test]
async fn promotion_with_different_content_also_flips_in_place() {
    let ctx = setup().await;
    provide(&ctx, "svc", "snapshot", &spec_two_endpoints("1.0.0")).await;

    let (status, body) = provide(&ctx, "svc", "ga", &spec_two_endpoints_reworded("1.0.0")).await;
    assert_eq!(status, StatusCode::ACCEPTED, "got: {body}");
    assert_eq!(body["stability"], "ga");
    assert_eq!(body["changes"]["updates"], 2);

    let (_, _, body) = send(&ctx, "GET", "/producers/svc/versions", None, &[]).await;
    let listing = as_json(&body);
    assert_eq!(listing.as_array().unwrap().len(), 1);
    assert_eq!(listing[0]["stability"], "ga");
}

// ---------------------------------------------------------------------------
// Rule 7: mislabel check — a GA that is breaking without a major bump lies.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ga_breaking_without_major_bump_is_rejected_with_major_proposal() {
    let ctx = setup().await;
    provide(&ctx, "svc", "ga", &spec_two_endpoints("1.0.0")).await;

    // 1.1.0 removes /orders — breaking, inside the same major: rejected.
    let (status, body) = provide(&ctx, "svc", "ga", &spec_users_only("1.1.0")).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let message = body["error"].as_str().unwrap();
    assert!(message.contains("breaking"), "got: {message}");
    assert!(
        message.contains("1.0.0"),
        "must name the GA baseline: {message}"
    );
    assert_eq!(body["proposed_version"], "2.0.0");

    // The same content honestly labeled as the next major: accepted.
    let (status, body) = provide(&ctx, "svc", "ga", &spec_users_only("2.0.0")).await;
    assert_eq!(status, StatusCode::ACCEPTED, "got: {body}");
    assert_eq!(body["version"], "2.0.0");
    assert_eq!(body["stability"], "ga");
}

#[tokio::test]
async fn snapshots_are_never_compat_checked() {
    let ctx = setup().await;
    provide(&ctx, "svc", "ga", &spec_two_endpoints("1.0.0")).await;

    // The same breaking content that a GA 1.1.0 would be rejected for is fine
    // as a snapshot — declared work-in-progress.
    let (status, body) = provide(&ctx, "svc", "snapshot", &spec_users_only("1.1.0")).await;
    assert_eq!(status, StatusCode::ACCEPTED, "got: {body}");
    assert_eq!(body["version"], "1.1.0");
    assert_eq!(body["stability"], "snapshot");
}

// ---------------------------------------------------------------------------
// Rule 8: resolution — exact pins, definitive failures, named answers.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn require_serves_exactly_the_pinned_version_with_resolution_headers() {
    let ctx = setup().await;
    provide(&ctx, "svc", "ga", &spec_two_endpoints("1.0.0")).await;
    provide(
        &ctx,
        "svc",
        "snapshot",
        &spec_two_endpoints_reworded("1.1.0"),
    )
    .await;

    // Pin the GA.
    let (status, headers, body) = require(&ctx, "cons", "svc", "1.0.0", "/users", "GET").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get("X-Sanshain-Resolution").unwrap(), "served");
    assert_eq!(headers.get("X-Sanshain-Version").unwrap(), "1.0.0");
    assert_eq!(headers.get("X-Sanshain-Stability").unwrap(), "ga");
    assert!(body.contains("description: OK"), "got: {body}");

    // Pin the snapshot: its (reworded) content, not the GA's.
    let (status, headers, body) = require(&ctx, "cons", "svc", "1.1.0", "/users", "GET").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get("X-Sanshain-Resolution").unwrap(), "served");
    assert_eq!(headers.get("X-Sanshain-Version").unwrap(), "1.1.0");
    assert_eq!(headers.get("X-Sanshain-Stability").unwrap(), "snapshot");
    assert!(body.contains("description: Fine"), "got: {body}");
}

#[tokio::test]
async fn unknown_pinned_version_fails_immediately_as_a_configuration_error() {
    let ctx = setup().await;
    provide(&ctx, "svc", "ga", &spec_two_endpoints("1.0.0")).await;

    let started = std::time::Instant::now();
    let (status, _, body) = require(&ctx, "cons", "svc", "9.9.9", "/users", "GET").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        body.contains("configuration error"),
        "a missing pin is a config error, got: {body}"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "nothing waits for a missing version to appear"
    );

    // An unknown producer is equally a 404.
    let (status, _, _) = require(&ctx, "cons", "nobody", "1.0.0", "/users", "GET").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn endpoint_absent_from_an_existing_version_is_gone() {
    let ctx = setup().await;
    provide(&ctx, "svc", "ga", &spec_users_only("1.0.0")).await;

    let (status, _, body) = require(&ctx, "cons", "svc", "1.0.0", "/orders", "GET").await;
    assert_eq!(status, StatusCode::GONE);
    assert!(body.contains("does not include"), "got: {body}");
    assert!(
        body.contains("1.0.0"),
        "the answer names the version: {body}"
    );
}

#[tokio::test]
async fn require_etag_round_trip_yields_304() {
    let ctx = setup().await;
    provide(&ctx, "svc", "ga", &spec_users_only("1.0.0")).await;

    let (status, headers, _) = require(&ctx, "cons", "svc", "1.0.0", "/users", "GET").await;
    assert_eq!(status, StatusCode::OK);
    let etag = headers.get("ETag").unwrap().to_str().unwrap().to_string();

    let uri = "/require?consumername=cons&producername=svc&version=1.0.0&path=/users&method=GET";
    let (status, _, body) = send(&ctx, "GET", uri, None, &[("If-None-Match", etag.as_str())]).await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty(), "a 304 carries no body, got: {body}");
}

// ---------------------------------------------------------------------------
// Rule 9: require-bundle.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn require_bundle_serves_all_present_endpoints_merged() {
    let ctx = setup().await;
    provide(&ctx, "svc", "ga", &spec_two_endpoints("1.0.0")).await;

    let payload = json!({
        "consumername": "cons",
        "producername": "svc",
        "version": "1.0.0",
        "endpoints": [
            {"path": "/users", "method": "GET"},
            {"path": "/orders", "method": "GET"},
        ],
    });
    let (status, headers, body) = send(&ctx, "POST", "/require-bundle", Some(payload), &[]).await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    assert_eq!(headers.get("X-Sanshain-Resolution").unwrap(), "served");
    assert_eq!(headers.get("X-Sanshain-Version").unwrap(), "1.0.0");
    assert_eq!(headers.get("X-Sanshain-Stability").unwrap(), "ga");
    assert!(body.contains("/users"), "merged yaml has both: {body}");
    assert!(body.contains("/orders"), "merged yaml has both: {body}");
}

#[tokio::test]
async fn require_bundle_with_any_missing_endpoint_is_gone_listing_them() {
    let ctx = setup().await;
    provide(&ctx, "svc", "ga", &spec_users_only("1.0.0")).await;

    let payload = json!({
        "consumername": "cons",
        "producername": "svc",
        "version": "1.0.0",
        "endpoints": [
            {"path": "/users", "method": "GET"},
            {"path": "/orders", "method": "GET"},
        ],
    });
    let (status, _, body) = send(&ctx, "POST", "/require-bundle", Some(payload), &[]).await;
    assert_eq!(status, StatusCode::GONE, "got: {body}");
    assert!(
        body.contains("GET /orders"),
        "the missing endpoints are listed: {body}"
    );
}

#[tokio::test]
async fn require_bundle_with_unknown_version_is_not_found() {
    let ctx = setup().await;
    provide(&ctx, "svc", "ga", &spec_two_endpoints("1.0.0")).await;

    let payload = json!({
        "consumername": "cons",
        "producername": "svc",
        "version": "3.0.0",
        "endpoints": [{"path": "/users", "method": "GET"}],
    });
    let (status, _, body) = send(&ctx, "POST", "/require-bundle", Some(payload), &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "got: {body}");
}

// ---------------------------------------------------------------------------
// Rule 10: dependencies are recorded only on success.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn failed_requires_record_no_dependency_successful_ones_do() {
    let ctx = setup().await;
    provide(&ctx, "svc", "ga", &spec_users_only("1.0.0")).await;

    // Unknown version: 404, no edge.
    let (status, _, _) = require(&ctx, "cons", "svc", "5.0.0", "/users", "GET").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Existing version, absent endpoint: 410, no edge.
    let (status, _, _) = require(&ctx, "cons", "svc", "1.0.0", "/orders", "GET").await;
    assert_eq!(status, StatusCode::GONE);

    let (_, _, body) = send(&ctx, "GET", "/report", None, &[]).await;
    let report = as_json(&body);
    assert_eq!(
        report["dependency_graph"].as_array().unwrap().len(),
        0,
        "failed requires must not create graph entities: {report}"
    );

    // Success: exactly one edge, carrying version and stability.
    let (status, _, _) = require(&ctx, "cons", "svc", "1.0.0", "/users", "GET").await;
    assert_eq!(status, StatusCode::OK);

    let (_, _, body) = send(&ctx, "GET", "/report", None, &[]).await;
    let report = as_json(&body);
    let graph = report["dependency_graph"].as_array().unwrap();
    assert_eq!(graph.len(), 1, "got: {report}");
    assert_eq!(graph[0]["client"], "cons");
    assert_eq!(graph[0]["service"], "svc");
    assert_eq!(graph[0]["version"], "1.0.0");
    assert_eq!(graph[0]["stability"], "ga");
    assert_eq!(graph[0]["path"], "/users");
    assert_eq!(graph[0]["method"], "GET");
}

// ---------------------------------------------------------------------------
// Rule 11: the version-line listing.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn producer_versions_lists_lines_and_filters_by_api_type() {
    let ctx = setup().await;
    provide(&ctx, "svc", "ga", &spec_two_endpoints("1.0.0")).await;
    provide(
        &ctx,
        "svc",
        "snapshot",
        &spec_two_endpoints_reworded("1.1.0"),
    )
    .await;
    let proto = "// sanshain-version: 0.1.0\nsyntax = \"proto3\";\nmessage Req {}\nmessage Res {}\nservice Svc { rpc Do (Req) returns (Res); }\n";
    let payload = json!({
        "producername": "svc",
        "proto_content": proto,
        "stability": "snapshot",
    });
    let (status, _, _) = send(&ctx, "POST", "/provide/grpc", Some(payload), &[]).await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let (status, _, body) = send(&ctx, "GET", "/producers/svc/versions", None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    let listing = as_json(&body);
    let entries = listing.as_array().unwrap();
    assert_eq!(entries.len(), 3, "got: {listing}");

    let openapi: Vec<&Value> = entries
        .iter()
        .filter(|e| e["api_type"] == "openapi")
        .collect();
    assert_eq!(openapi.len(), 2);
    assert_eq!(openapi[0]["version"], "1.1.0", "newest first: {listing}");
    assert_eq!(openapi[0]["stability"], "snapshot");
    assert_eq!(openapi[0]["endpoint_count"], 2);
    assert_eq!(openapi[1]["version"], "1.0.0");
    assert_eq!(openapi[1]["stability"], "ga");

    // API-type filter.
    let (status, _, body) = send(
        &ctx,
        "GET",
        "/producers/svc/versions?api_type=proto",
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let listing = as_json(&body);
    let entries = listing.as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["api_type"], "proto");
    assert_eq!(entries[0]["version"], "0.1.0");
    assert_eq!(entries[0]["endpoint_count"], 1);

    // Unknown producer.
    let (status, _, _) = send(&ctx, "GET", "/producers/ghost/versions", None, &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Rule 13: use-based snapshot expiry via the admin trigger.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn snapshot_cleanup_removes_only_unused_snapshots() {
    let ctx = setup().await;
    provide(&ctx, "svc", "snapshot", &spec_users_only("0.1.0")).await;
    provide(&ctx, "svc", "ga", &spec_two_endpoints("1.0.0")).await;

    let (status, _, _) = send(
        &ctx,
        "POST",
        "/admin/settings/snapshot-max-age",
        Some(json!({"days": 1})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Backdate both entries beyond the cutoff — only the snapshot may die.
    sqlx::query(
        "UPDATE spec_versions SET updated_at = '2020-01-01T00:00:00Z', last_required_at = NULL",
    )
    .execute(&ctx.pool)
    .await
    .unwrap();

    let (status, _, body) = send(&ctx, "POST", "/admin/cleanup/snapshots", None, &[]).await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    assert_eq!(as_json(&body)["deleted"], 1);

    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT major || '.' || minor || '.' || patch, stability, api_type FROM spec_versions",
    )
    .fetch_all(&ctx.pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![("1.0.0".to_string(), "ga".to_string(), "openapi".to_string())],
        "GA is never age-culled"
    );
}

#[tokio::test]
async fn a_recently_required_snapshot_survives_cleanup() {
    let ctx = setup().await;
    provide(&ctx, "svc", "snapshot", &spec_users_only("0.1.0")).await;

    let (status, _, _) = send(
        &ctx,
        "POST",
        "/admin/settings/snapshot-max-age",
        Some(json!({"days": 1})),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Old provide, but a fresh require: use-based expiry counts both.
    sqlx::query("UPDATE spec_versions SET updated_at = '2020-01-01T00:00:00Z'")
        .execute(&ctx.pool)
        .await
        .unwrap();
    let (status, _, _) = require(&ctx, "cons", "svc", "0.1.0", "/users", "GET").await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, body) = send(&ctx, "POST", "/admin/cleanup/snapshots", None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(as_json(&body)["deleted"], 0, "got: {body}");

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM spec_versions")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    assert_eq!(count.0, 1, "the required snapshot survives");
}

// ---------------------------------------------------------------------------
// The free validator: public, stateless, always a 200 verdict.
// ---------------------------------------------------------------------------

/// Send to /validate with NO auth and NO CSRF token — the endpoint is public.
#[cfg(test)]
async fn send_anonymous_validate(ctx: &TestContext, body: Value) -> (StatusCode, String) {
    let request = Request::builder()
        .method("POST")
        .uri("/validate")
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let response = ctx.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

#[tokio::test]
async fn validator_is_public_and_previews_the_split() {
    let ctx = setup().await;
    let (status, body) = send_anonymous_validate(
        &ctx,
        serde_json::json!({
            "api_type": "openapi",
            "content": "openapi: 3.0.3\ninfo:\n  title: T\n  version: 1.2.0\npaths:\n  /users:\n    get:\n      responses:\n        '200':\n          description: OK\n  /orders:\n    post:\n      responses:\n        '201':\n          description: Created\n",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    let verdict = as_json(&body);
    assert_eq!(verdict["valid"], true);
    assert_eq!(verdict["version"], "1.2.0");
    assert_eq!(verdict["endpoint_count"], 2);
    let endpoints: Vec<String> = verdict["endpoints"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(
        endpoints.contains(&"GET /users".to_string()),
        "{endpoints:?}"
    );
    assert!(
        endpoints.contains(&"POST /orders".to_string()),
        "{endpoints:?}"
    );
}

#[tokio::test]
async fn validator_reports_version_errors_with_the_split_preview() {
    let ctx = setup().await;
    let (status, body) = send_anonymous_validate(
        &ctx,
        serde_json::json!({
            "api_type": "openapi",
            "content": "openapi: 3.0.3\ninfo:\n  title: T\n  version: 1.2.0-SNAPSHOT\npaths:\n  /users:\n    get:\n      responses:\n        '200':\n          description: OK\n",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "verdicts are 200, got: {body}");
    let verdict = as_json(&body);
    assert_eq!(verdict["valid"], false);
    let error = verdict["error"].as_str().unwrap();
    assert!(
        error.contains("stability"),
        "the -SNAPSHOT hint must point at the stability flag: {error}"
    );
    // The document itself parses, so the preview still renders.
    assert_eq!(verdict["endpoint_count"], 1);
}

#[tokio::test]
async fn validator_names_the_missing_proto_marker() {
    let ctx = setup().await;
    let (status, body) = send_anonymous_validate(
        &ctx,
        serde_json::json!({
            "api_type": "proto",
            "content": "syntax = \"proto3\";\npackage a.b;\nservice S {\n  rpc Do (In) returns (Out);\n}\nmessage In {}\nmessage Out {}\n",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "got: {body}");
    let verdict = as_json(&body);
    assert_eq!(verdict["valid"], false);
    assert!(
        verdict["error"]
            .as_str()
            .unwrap()
            .contains("// sanshain-version: MAJOR.MINOR.PATCH"),
        "got: {body}"
    );
    assert_eq!(verdict["endpoints"][0], "Do S");
}

#[tokio::test]
async fn validator_rejects_unknown_shapes_but_stores_nothing() {
    let ctx = setup().await;
    let (status, _) = send_anonymous_validate(
        &ctx,
        serde_json::json!({ "api_type": "soap", "content": "x" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // A valid validation stores nothing: no producer appears.
    let (_, body) = send_anonymous_validate(
        &ctx,
        serde_json::json!({
            "api_type": "openapi",
            "content": "openapi: 3.0.3\ninfo:\n  title: T\n  version: 9.9.9\npaths: {}\n",
        }),
    )
    .await;
    assert_eq!(as_json(&body)["valid"], true);
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM spec_versions WHERE major = 9")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    assert_eq!(count.0, 0, "validation must never persist anything");
}

// ---------------------------------------------------------------------------
// VERSION_REJECTED: rejections are self-service but counted.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_version_rule_rejection_is_audited_as_version_rejected() {
    let ctx = setup().await;
    provide(&ctx, "svc", "ga", &spec_users_only("1.0.0")).await;

    // Forgot-to-bump: same GA number, different content.
    let (status, _, _) = send(
        &ctx,
        "POST",
        "/provide",
        Some(serde_json::json!({
            "producername": "svc",
            "stability": "ga",
            "openapi_yaml": spec_users_only("1.0.0").replace("/users", "/changed"),
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let row: (String, String, String, String) = sqlx::query_as(
        "SELECT username, details, version, action_type FROM audit_logs WHERE action = 'VERSION_REJECTED'",
    )
    .fetch_one(&ctx.pool)
    .await
    .expect("the rejection must be audited");
    assert_eq!(row.0, "root", "the audit names the authenticated Actor");
    assert!(
        row.1.contains("immutable"),
        "details carry the reason: {}",
        row.1
    );
    assert_eq!(row.2, "1.0.0");
    assert_eq!(
        row.3, "REJECT",
        "the timeline's Rejected filter must catch it"
    );
}

#[tokio::test]
async fn a_dry_run_rejection_is_previewed_but_not_audited() {
    let ctx = setup().await;
    provide(&ctx, "svc", "ga", &spec_users_only("1.0.0")).await;

    let (status, _, body) = send(
        &ctx,
        "POST",
        "/provide",
        Some(serde_json::json!({
            "producername": "svc",
            "stability": "ga",
            "dry_run": true,
            "openapi_yaml": spec_users_only("1.0.0").replace("/users", "/changed"),
        })),
        &[],
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "the preview still answers 409"
    );
    assert!(body.contains("proposed_version"));

    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM audit_logs WHERE action = 'VERSION_REJECTED'")
            .fetch_one(&ctx.pool)
            .await
            .unwrap();
    assert_eq!(count.0, 0, "a dry run is a preview and records nothing");
}
