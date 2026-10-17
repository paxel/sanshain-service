//! Read-only API tokens at the router seam: a `read_only` token is refused
//! on every route that changes the catalog, accepted on every read, and its
//! `last_used_at` stamp is throttled rather than written per request.

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
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
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

struct Seed {
    app: axum::Router,
    session: String,
    full: String,
    read_only: String,
}

// `cfg(test)` is always true in this crate; the attribute marks the helpers as
// test code for clippy's `allow-unwrap-in-tests`.
#[cfg(test)]
async fn seed() -> Seed {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    let hash = services::hash_password("pw").unwrap();
    let user = repo.create_user("root", &hash, true).await.unwrap();
    let (session, _) = services::login(&repo, "root", "pw").await.unwrap();
    let (_, full) = services::create_api_token(&repo, user.id, "ci", 30, ApiTokenScope::Full)
        .await
        .unwrap();
    let (_, read_only) =
        services::create_api_token(&repo, user.id, "ai", 30, ApiTokenScope::ReadOnly)
            .await
            .unwrap();

    services::provide_spec(
        &repo,
        services::ProvideSpecParams {
            producername: "demo-svc",
            api_type: ApiType::OpenApi,
            content: DEMO_SPEC_1_0_0,
            stability: Stability::Ga,
            dry_run: false,
            trunk: false,
            tag: None,
            caller: Some(sanshain_service::domain::permissions::Actor::test_releaser()),
            require_prior_content_match: false,
        },
    )
    .await
    .unwrap();

    Seed {
        app: create_app(test_state(repo)),
        session: session.token,
        full,
        read_only,
    }
}

#[cfg(test)]
async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<&Value>,
) -> (StatusCode, Value) {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
        .header(axum::http::header::CONTENT_TYPE, "application/json");
    let req = match body {
        Some(b) => builder.body(Body::from(b.to_string())).unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

fn require_uri() -> &'static str {
    "/require?consumername=web&producername=demo-svc&version=1.0.0&path=/hello&method=get"
}

#[tokio::test]
async fn token_creation_takes_a_scope_and_the_listing_shows_it() {
    let seed = seed().await;

    let (status, created) = call(
        &seed.app,
        "POST",
        "/auth/tokens",
        &seed.session,
        Some(&json!({ "name": "mcp", "expires_in_days": 7, "scope": "read_only" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(created["scope"], "read_only");
    assert!(created["token"].as_str().unwrap().starts_with("san_"));

    // Omitting the scope mints a full token, as every pre-2.5 client does.
    let (status, created) = call(
        &seed.app,
        "POST",
        "/auth/tokens",
        &seed.session,
        Some(&json!({ "name": "legacy", "expires_in_days": 7 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(created["scope"], "full");

    let (status, listed) = call(&seed.app, "GET", "/auth/tokens", &seed.session, None).await;
    assert_eq!(status, StatusCode::OK);
    let scopes: HashMap<&str, &str> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["name"].as_str().unwrap(), t["scope"].as_str().unwrap()))
        .collect();
    assert_eq!(scopes["mcp"], "read_only");
    assert_eq!(scopes["legacy"], "full");
    assert_eq!(scopes["ai"], "read_only");
    assert_eq!(scopes["ci"], "full");
    let never_used = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "mcp")
        .unwrap();
    assert_eq!(never_used["last_used_at"], Value::Null);
}

#[tokio::test]
async fn the_token_listing_never_carries_the_token_hash() {
    let seed = seed().await;

    let (status, created) = call(
        &seed.app,
        "POST",
        "/auth/tokens",
        &seed.session,
        Some(&json!({ "name": "listed", "expires_in_days": 7 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let raw = created["token"].as_str().unwrap();
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    let hash = hex::encode(hasher.finalize());

    let (status, listed) = call(&seed.app, "GET", "/auth/tokens", &seed.session, None).await;
    assert_eq!(status, StatusCode::OK);
    let body = listed.to_string();
    assert!(
        !body.contains("token_hash"),
        "the listing must not expose the hash field: {body}"
    );
    assert!(
        !body.contains(&hash) && !body.contains(raw),
        "neither the hash nor the raw token may appear in the listing: {body}"
    );
    for entry in listed.as_array().unwrap() {
        assert!(entry.get("token_hash").is_none());
    }
}

#[tokio::test]
async fn read_only_token_is_refused_on_every_catalog_changing_route() {
    let seed = seed().await;
    let writes: &[(&str, &str, Value)] = &[
        (
            "POST",
            "/provide",
            json!({ "producername": "demo-svc", "openapi_yaml": DEMO_SPEC_1_0_0, "stability": "snapshot" }),
        ),
        (
            "POST",
            "/provide",
            json!({ "producername": "demo-svc", "retired": true }),
        ),
        (
            "POST",
            "/provide/asyncapi",
            json!({ "producername": "demo-svc", "asyncapi_yaml": "x", "stability": "snapshot" }),
        ),
        (
            "POST",
            "/provide/grpc",
            json!({ "producername": "demo-svc", "proto": "x", "stability": "snapshot" }),
        ),
        ("GET", require_uri(), Value::Null),
        (
            "GET",
            "/require/asyncapi?consumername=web&producername=demo-svc&version=1.0.0&channel=c",
            Value::Null,
        ),
        (
            "GET",
            "/require/grpc?consumername=web&producername=demo-svc&version=1.0.0&service=s&method=m",
            Value::Null,
        ),
        (
            "POST",
            "/require-bundle",
            json!({ "consumername": "web", "producername": "demo-svc", "version": "1.0.0", "endpoints": [] }),
        ),
        (
            "PUT",
            "/catalog/web",
            json!({ "catalog_info_yaml": "kind: Component" }),
        ),
    ];

    for (method, uri, body) in writes {
        let body = if body.is_null() { None } else { Some(body) };
        let (status, answer) = call(&seed.app, method, uri, &seed.read_only, body).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{method} {uri} must refuse a read-only token"
        );
        let message = answer["error"].as_str().unwrap_or_default();
        assert!(
            message.contains("read_only") && message.contains("full"),
            "{method} {uri}: the refusal must name the token's scope and the needed one, got {message:?}"
        );
    }
}

#[tokio::test]
async fn full_token_still_writes_and_read_only_token_still_reads() {
    let seed = seed().await;

    let (status, _) = call(&seed.app, "GET", require_uri(), &seed.full, None).await;
    assert_eq!(status, StatusCode::OK, "a full token requires as before");
    let (status, _) = call(
        &seed.app,
        "POST",
        "/provide",
        &seed.full,
        Some(&json!({
            "producername": "demo-svc",
            "openapi_yaml": DEMO_SPEC_1_0_0.replace("version: 1.0.0", "version: 1.0.1"),
            "stability": "snapshot"
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "a full token provides as before"
    );

    for uri in [
        "/version",
        "/report",
        "/report/markdown",
        "/report/isolation",
        "/producers/demo-svc/versions",
        "/endpoint-versions?service=demo-svc&path=/hello&method=get",
    ] {
        let (status, _) = call(&seed.app, "GET", uri, &seed.read_only, None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "GET {uri} must accept a read-only token"
        );
    }
}

#[tokio::test]
async fn last_used_at_is_stamped_on_use_and_throttled() {
    let seed = seed().await;

    let (status, listed) = call(&seed.app, "GET", "/auth/tokens", &seed.session, None).await;
    assert_eq!(status, StatusCode::OK);
    let ai = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "ai")
        .unwrap()
        .clone();
    assert_eq!(
        ai["last_used_at"],
        Value::Null,
        "unused until it authenticates"
    );

    let (status, _) = call(&seed.app, "GET", "/report", &seed.read_only, None).await;
    assert_eq!(status, StatusCode::OK);
    let (_, listed) = call(&seed.app, "GET", "/auth/tokens", &seed.session, None).await;
    let stamped = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "ai")
        .unwrap()["last_used_at"]
        .as_str()
        .expect("first use stamps last_used_at")
        .to_string();
    chrono::DateTime::parse_from_rfc3339(&stamped).expect("the stamp is RFC 3339");

    // A second use within the minute leaves the stamp untouched.
    let (status, _) = call(&seed.app, "GET", "/report", &seed.read_only, None).await;
    assert_eq!(status, StatusCode::OK);
    let (_, listed) = call(&seed.app, "GET", "/auth/tokens", &seed.session, None).await;
    let again = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "ai")
        .unwrap()["last_used_at"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        again, stamped,
        "no write per request: the stamp is throttled"
    );

    // The full token is untouched by the other token's use.
    let ci = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "ci")
        .unwrap();
    assert_eq!(ci["last_used_at"], Value::Null);
}
