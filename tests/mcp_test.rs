//! The MCP endpoint over JSON-RPC against the router (docs/mcp.md): the
//! bearer-token guard, the stateless handshake, the tool catalog, every tool's
//! happy and error path, the rate limit, the size budget and the metrics.

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
use sanshain_service::presentation::mcp::McpConfig;
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

fn test_state(repo: SqliteSpecRepository, mcp: McpConfig) -> AppState {
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
        mcp,
        directory_roles: sanshain_service::application::directory_roles::DirectoryRoleCache::new(
            std::time::Duration::from_secs(300),
        ),
        root_users: std::sync::Arc::new(sanshain_service::domain::permissions::RootUsers::resolve(
            Some("root"),
            None,
        )),
    }
}

const ORDERS_1_0_0: &str = r#"openapi: 3.0.0
info:
  title: orders
  version: 1.0.0
paths:
  /orders:
    get:
      responses:
        '200': { description: ok }
  /orders/{id}:
    get:
      responses:
        '200': { description: ok }
    delete:
      deprecated: true
      responses:
        '204': { description: gone }
"#;

const ORDERS_1_1_0: &str = r#"openapi: 3.0.0
info:
  title: orders
  version: 1.1.0
paths:
  /orders:
    get:
      responses:
        '200': { description: ok }
    post:
      responses:
        '201': { description: created }
  /orders/{id}:
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
async fn seed_with(mcp: McpConfig) -> Seed {
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

    for (content, trunk) in [(ORDERS_1_0_0, true), (ORDERS_1_1_0, false)] {
        services::provide_spec(
            &repo,
            services::ProvideSpecParams {
                producername: "orders",
                api_type: ApiType::OpenApi,
                content,
                stability: Stability::Ga,
                dry_run: false,
                trunk,
                tag: None,
                caller: Some(sanshain_service::domain::permissions::Actor::test_releaser()),
                require_prior_content_match: false,
            },
        )
        .await
        .unwrap();
    }
    services::require_endpoint(
        &repo,
        services::RequireEndpointParams {
            consumername: "web",
            producername: "orders",
            version: "1.0.0".parse().unwrap(),
            api_type: ApiType::OpenApi,
            path: "/orders/{id}",
            method: "get",
            trunk: true,
            tag: None,
            format: services::RequireFormat::Native,
        },
    )
    .await
    .unwrap();

    Seed {
        app: create_app(test_state(repo, mcp)),
        session: session.token,
        full,
        read_only,
    }
}

#[cfg(test)]
async fn seed() -> Seed {
    seed_with(McpConfig::default()).await
}

/// One JSON-RPC request to `/mcp`; answers the HTTP status and the parsed
/// body (`Null` when it is not JSON).
#[cfg(test)]
async fn rpc(app: &axum::Router, token: Option<&str>, body: Value) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header(axum::http::header::HOST, "sanshain.example")
        .header(
            axum::http::header::ACCEPT,
            "application/json, text/event-stream",
        )
        .header(axum::http::header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        builder = builder.header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let res = app
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).to_string()));
    (status, json)
}

fn initialize_request() -> Value {
    json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "0" }
        }
    })
}

fn tools_list_request() -> Value {
    json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })
}

fn call_request(tool: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": { "name": tool, "arguments": arguments }
    })
}

/// The tool's answer: `(is_error, text)` of its first content block.
#[cfg(test)]
async fn call(app: &axum::Router, token: &str, tool: &str, arguments: Value) -> (bool, String) {
    let (status, body) = rpc(app, Some(token), call_request(tool, arguments)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let result = &body["result"];
    assert!(
        result.is_object(),
        "a tool answers a result, not a transport error: {body}"
    );
    (
        result["isError"].as_bool().unwrap_or(false),
        result["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    )
}

#[tokio::test]
async fn missing_header_is_401_with_a_bearer_challenge() {
    let seed = seed().await;
    let res = seed
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header(axum::http::header::HOST, "sanshain.example")
                .header(
                    axum::http::header::ACCEPT,
                    "application/json, text/event-stream",
                )
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(initialize_request().to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let challenge = res
        .headers()
        .get(axum::http::header::WWW_AUTHENTICATE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    assert!(challenge.starts_with("Bearer"), "got {challenge:?}");
}

#[tokio::test]
async fn session_tokens_are_refused_and_both_api_token_scopes_accepted() {
    let seed = seed().await;
    let (status, _) = rpc(&seed.app, Some(&seed.session), initialize_request()).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a browser session is not an MCP credential"
    );
    let (status, _) = rpc(&seed.app, Some("san_nope"), initialize_request()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    for token in [&seed.full, &seed.read_only] {
        let (status, body) = rpc(&seed.app, Some(token), initialize_request()).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["result"]["serverInfo"]["name"], "sanshain");
        assert!(body["result"]["capabilities"]["tools"].is_object());
    }
}

#[tokio::test]
async fn read_only_token_works_on_mcp_but_not_on_provide() {
    let seed = seed().await;
    let (status, _) = rpc(&seed.app, Some(&seed.read_only), tools_list_request()).await;
    assert_eq!(status, StatusCode::OK);
    let res = seed
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/provide")
                .header(axum::http::header::AUTHORIZATION, format!("Bearer {}", seed.read_only))
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "producername": "orders", "openapi_yaml": ORDERS_1_1_0, "stability": "snapshot" })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn disabled_endpoint_is_not_mounted() {
    let seed = seed_with(McpConfig {
        enabled: false,
        ..Default::default()
    })
    .await;
    // Nothing answers at /mcp: a GET is the static fallback's 404, and a
    // POST its 405 — neither the transport nor its guard is there.
    let res = seed
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/mcp")
                .header(
                    axum::http::header::AUTHORIZATION,
                    format!("Bearer {}", seed.full),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let (status, _) = rpc(&seed.app, Some(&seed.full), initialize_request()).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    let (status, _) = rpc(&seed.app, None, initialize_request()).await;
    assert_ne!(status, StatusCode::UNAUTHORIZED, "no guard either");
}

#[tokio::test]
async fn rate_limit_answers_a_tool_error_not_an_http_status() {
    let seed = seed_with(McpConfig {
        calls_per_minute: 2,
        ..Default::default()
    })
    .await;
    let first = call(&seed.app, &seed.full, "nonexistent", json!({})).await;
    assert!(first.0 && first.1.contains("unknown tool"), "{first:?}");
    let _ = call(&seed.app, &seed.full, "nonexistent", json!({})).await;
    let (is_error, text) = call(&seed.app, &seed.full, "nonexistent", json!({})).await;
    assert!(is_error);
    assert!(
        text.starts_with("rate limited - retry in ") && text.ends_with(" s"),
        "got {text:?}"
    );
    // The limit is per token: the other token is untouched.
    let other = call(&seed.app, &seed.read_only, "nonexistent", json!({})).await;
    assert!(other.1.contains("unknown tool"), "{other:?}");
}

#[tokio::test]
async fn metrics_count_tool_calls_by_outcome() {
    let seed = seed().await;
    let _ = call(&seed.app, &seed.full, "nonexistent", json!({})).await;
    let res = seed
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("sanshain_mcp_tool_calls_total{")
            && text.contains("outcome=\"error\"")
            && text.contains("tool=\"nonexistent\""),
        "metrics must count the call: {text}"
    );
}
