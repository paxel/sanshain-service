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
        snapshot_in_progress: Arc::new(AtomicBool::new(false)),
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
      summary: Fetch one order
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
      summary: Fetch one order
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

/// The tool's JSON answer; fails the test on a refusal.
#[cfg(test)]
async fn call_json(app: &axum::Router, token: &str, tool: &str, arguments: Value) -> Value {
    let (is_error, text) = call(app, token, tool, arguments).await;
    assert!(!is_error, "{tool} refused: {text}");
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{tool} answered non-JSON ({e}): {text}"))
}

#[tokio::test]
async fn tools_list_names_the_seven_tools_with_schemas() {
    let seed = seed().await;
    let (status, body) = rpc(&seed.app, Some(&seed.full), tools_list_request()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let tools = body["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "list_services",
            "describe_service",
            "find_endpoint",
            "get_spec",
            "get_graph",
            "diff_versions",
            "blame",
            "endpoint_structure"
        ]
    );
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object", "{}", tool["name"]);
        assert_eq!(
            tool["inputSchema"]["additionalProperties"], false,
            "{}",
            tool["name"]
        );
        assert!(tool["description"].as_str().unwrap().len() > 200);
    }
}

#[tokio::test]
async fn list_services_and_describe_service_answer_the_catalog() {
    let seed = seed().await;
    let list = call_json(&seed.app, &seed.read_only, "list_services", json!({})).await;
    assert_eq!(list["scope"], "dev");
    assert_eq!(list["at"], Value::Null);
    let orders = &list["producers"][0];
    assert_eq!(orders["name"], "orders");
    assert_eq!(orders["api_types"], json!(["openapi"]));
    assert_eq!(orders["versions"][0]["version"], "1.1.0");
    assert_eq!(orders["versions"][0]["stability"], "ga");
    assert_eq!(orders["endpoints"], 3);
    assert_eq!(orders["consumers"], 1);
    assert_eq!(orders["unrequired"], false);
    assert_eq!(list["consumers"][0]["name"], "web");
    assert_eq!(list["consumers"][0]["producers"], json!(["orders"]));

    // The main scope is the trunk pin set: 1.0.0 was the trunk provide.
    let main = call_json(
        &seed.app,
        &seed.full,
        "list_services",
        json!({ "scope": "main" }),
    )
    .await;
    assert_eq!(main["scope"], "main");
    assert_eq!(main["producers"][0]["versions"][0]["version"], "1.0.0");

    let d = call_json(
        &seed.app,
        &seed.full,
        "describe_service",
        json!({ "name": "orders" }),
    )
    .await;
    assert_eq!(d["is_producer"], true);
    let versions = d["versions"].as_array().unwrap();
    assert_eq!(versions.len(), 2);
    let v1 = versions.iter().find(|v| v["version"] == "1.0.0").unwrap();
    assert_eq!(v1["superseded"], true);
    assert!(v1["last_provided_at"].is_string());
    assert_eq!(
        v1["origin"],
        json!({"build_url": null, "change_url": null, "tickets": [], "note": null})
    );
    let endpoints = d["endpoints"].as_array().unwrap();
    assert_eq!(
        endpoints.len(),
        3,
        "the latest version's endpoints: {endpoints:?}"
    );
    assert!(endpoints.iter().all(|e| e["version"] == "1.1.0"));
    let described = endpoints
        .iter()
        .find(|e| e["path"] == "/orders/{id}" && e["method"] == "GET")
        .unwrap();
    assert_eq!(described["summary"], "Fetch one order");
    let bare = endpoints.iter().find(|e| e["path"] == "/orders").unwrap();
    assert!(bare["summary"].is_null(), "{bare}");
    assert_eq!(d["consumers"][0]["name"], "web");
    assert_eq!(d["consumers"][0]["version"], "1.0.0");
    assert_eq!(d["consumers"][0]["endpoints"][0]["path"], "/orders/{id}");

    let (is_error, text) = call(
        &seed.app,
        &seed.full,
        "describe_service",
        json!({ "name": "ghost" }),
    )
    .await;
    assert!(is_error && text.contains("ghost"), "{text}");
    let (is_error, text) = call(
        &seed.app,
        &seed.full,
        "list_services",
        json!({ "scope": "no-such-branch" }),
    )
    .await;
    assert!(is_error && text.contains("no-such-branch"), "{text}");
}

#[tokio::test]
async fn describe_service_carries_the_context_of_the_catalog_info() {
    let seed = seed().await;
    let before = call_json(
        &seed.app,
        &seed.full,
        "describe_service",
        json!({ "name": "orders" }),
    )
    .await;
    assert_eq!(before["context"], Value::Null);

    // The build pushes the catalog-info (#48) through the same router.
    let catalog = "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: orders\n  description: Takes orders.\n  annotations:\n    backstage.io/source-location: url:https://github.com/acme/orders\n  links:\n    - url: https://wiki.acme.corp/orders\n      title: Wiki\n      type: docs\n  tags:\n    - java\nspec:\n  owner: team-orders\n";
    let res = seed
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/catalog/orders")
                .header(
                    axum::http::header::AUTHORIZATION,
                    format!("Bearer {}", seed.session),
                )
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "catalog_info_yaml": catalog, "trunk": true }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let d = call_json(
        &seed.app,
        &seed.read_only,
        "describe_service",
        json!({ "name": "orders" }),
    )
    .await;
    assert_eq!(
        d["context"],
        json!({
            "owner": { "kind": "group", "name": "team-orders" },
            "description": "Takes orders.",
            "source_location": "https://github.com/acme/orders",
            "links": [
                { "url": "https://wiki.acme.corp/orders", "title": "Wiki", "type": "docs", "icon": null }
            ],
            "tags": ["java"]
        })
    );
}

#[tokio::test]
async fn find_endpoint_searches_latest_versions_by_default() {
    let seed = seed().await;
    let found = call_json(
        &seed.app,
        &seed.full,
        "find_endpoint",
        json!({ "query": "orders/*" }),
    )
    .await;
    assert_eq!(found["query"], "orders/*");
    assert_eq!(found["total"], 1, "{found}");
    let hit = &found["hits"][0];
    assert_eq!(hit["producer"], "orders");
    assert_eq!(hit["version"], "1.1.0");
    assert_eq!(hit["method"], "GET");
    assert_eq!(hit["path"], "/orders/{id}");
    assert_eq!(hit["consumer_count"], 0, "the pin is on 1.0.0");
    assert_eq!(hit["summary"], "Fetch one order");
    assert!(hit.get("yaml_content").is_none(), "{hit}");

    let all = call_json(
        &seed.app,
        &seed.full,
        "find_endpoint",
        json!({ "query": "ORDERS/", "all_versions": true, "method": "get" }),
    )
    .await;
    assert_eq!(all["total"], 2, "{all}");
    let pinned = all["hits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["version"] == "1.0.0")
        .unwrap();
    assert_eq!(pinned["consumer_count"], 1);
    let deprecated = call_json(
        &seed.app,
        &seed.full,
        "find_endpoint",
        json!({ "query": "orders", "all_versions": true, "method": "DELETE" }),
    )
    .await;
    assert_eq!(deprecated["hits"][0]["deprecated"], true);

    let none = call_json(
        &seed.app,
        &seed.full,
        "find_endpoint",
        json!({ "query": "x", "api_type": "proto" }),
    )
    .await;
    assert_eq!(none["total"], 0);
    let (is_error, text) = call(
        &seed.app,
        &seed.full,
        "find_endpoint",
        json!({ "query": " " }),
    )
    .await;
    assert!(is_error && text.contains("query"), "{text}");
    let (is_error, text) = call(&seed.app, &seed.full, "find_endpoint", json!({ "q": "x" })).await;
    assert!(is_error && text.contains("invalid parameters"), "{text}");
}

#[tokio::test]
async fn get_spec_answers_documents_snippets_and_pages() {
    let seed = seed().await;
    let doc = call_json(
        &seed.app,
        &seed.full,
        "get_spec",
        json!({ "service": "orders", "api_type": "openapi" }),
    )
    .await;
    assert_eq!(doc["version"], "1.1.0", "latest when omitted");
    assert_eq!(doc["stability"], "ga");
    assert_eq!(doc["lines"], ORDERS_1_1_0.lines().count());
    assert_eq!(doc["next_offset"], Value::Null);
    assert!(doc["text"].as_str().unwrap().contains("version: 1.1.0"));

    let page = call_json(
        &seed.app,
        &seed.full,
        "get_spec",
        json!({ "service": "orders", "api_type": "openapi", "version": "1.0.0", "offset": 2, "limit": 2 }),
    )
    .await;
    assert_eq!(page["version"], "1.0.0");
    assert_eq!(page["offset"], 2);
    assert_eq!(page["next_offset"], 4);
    assert_eq!(page["text"], "info:\n  title: orders");

    let snippet = call_json(
        &seed.app,
        &seed.full,
        "get_spec",
        json!({ "service": "orders", "api_type": "openapi", "path": "/orders/{id}", "method": "get" }),
    )
    .await;
    assert_eq!(snippet["path"], "/orders/{id}");
    assert!(snippet["text"].as_str().unwrap().contains("responses"));

    for (args, needle) in [
        (
            json!({ "service": "nobody", "api_type": "openapi" }),
            "nobody",
        ),
        (
            json!({ "service": "orders", "api_type": "openapi", "version": "9.9.9" }),
            "9.9.9",
        ),
        (
            json!({ "service": "orders", "api_type": "openapi", "path": "/nope", "method": "get" }),
            "/nope",
        ),
        (
            json!({ "service": "orders", "api_type": "openapi", "path": "/orders" }),
            "path and method",
        ),
        (
            json!({ "service": "orders", "api_type": "openapi", "version": "latest" }),
            "MAJOR.MINOR.PATCH",
        ),
    ] {
        let (is_error, text) = call(&seed.app, &seed.full, "get_spec", args).await;
        assert!(
            is_error && text.contains(needle),
            "expected {needle:?} in {text:?}"
        );
    }
}

#[tokio::test]
async fn get_spec_over_budget_answers_a_summary_and_pages_fit() {
    let seed = seed_with(McpConfig {
        max_result_bytes: 300,
        ..Default::default()
    })
    .await;
    let summary = call_json(
        &seed.app,
        &seed.full,
        "get_spec",
        json!({ "service": "orders", "api_type": "openapi" }),
    )
    .await;
    assert_eq!(
        summary["hint"],
        "use offset/limit (line numbers) or path+method"
    );
    assert_eq!(summary["summary"]["lines"], ORDERS_1_1_0.lines().count());
    assert_eq!(summary["summary"]["bytes"], ORDERS_1_1_0.len());
    let endpoints = summary["summary"]["endpoints"].as_array().unwrap();
    assert_eq!(endpoints.len(), 3);
    assert!(summary.get("text").is_none());

    let page = call_json(
        &seed.app,
        &seed.full,
        "get_spec",
        json!({ "service": "orders", "api_type": "openapi", "offset": 1, "limit": 2 }),
    )
    .await;
    assert_eq!(page["text"], "openapi: 3.0.0\ninfo:");
    assert_eq!(page["next_offset"], 3);
}

#[tokio::test]
async fn get_graph_answers_nodes_and_edges_and_narrows() {
    let seed = seed().await;
    let graph = call_json(&seed.app, &seed.read_only, "get_graph", json!({})).await;
    assert_eq!(graph["scope"], "dev");
    let nodes = graph["nodes"].as_array().unwrap();
    let kind = |name: &str| {
        nodes
            .iter()
            .find(|n| n["service"] == name)
            .map(|n| n["kind"].clone())
    };
    assert_eq!(kind("orders"), Some(json!("producer")));
    assert_eq!(kind("web"), Some(json!("consumer")));
    let edge = &graph["edges"][0];
    assert_eq!(edge["from"], "web");
    assert_eq!(edge["to"], "orders");
    assert_eq!(edge["version"], "1.0.0");
    assert_eq!(edge["outdated"], true, "1.1.0 exists");
    assert_eq!(edge["superseded"], false);

    let narrowed = call_json(
        &seed.app,
        &seed.full,
        "get_graph",
        json!({ "scope": "main", "service": "web", "include_periphery": true }),
    )
    .await;
    assert_eq!(narrowed["scope"], "main");
    assert_eq!(narrowed["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(narrowed["edges"].as_array().unwrap().len(), 1);

    let (is_error, text) = call(
        &seed.app,
        &seed.full,
        "get_graph",
        json!({ "service": "ghost" }),
    )
    .await;
    assert!(is_error && text.contains("ghost"), "{text}");
    let (is_error, text) = call(
        &seed.app,
        &seed.full,
        "get_graph",
        json!({ "at": "2026-01-01T00:00:00Z" }),
    )
    .await;
    assert!(
        is_error && text.contains("dev"),
        "dev has no timeline: {text}"
    );

    let seed = seed_with(McpConfig {
        max_result_bytes: 64,
        ..Default::default()
    })
    .await;
    let summary = call_json(&seed.app, &seed.full, "get_graph", json!({})).await;
    assert_eq!(summary["nodes"]["producer"], 1);
    assert_eq!(summary["nodes"]["consumer"], 1);
    assert_eq!(summary["edges"], 1);
    assert!(summary["hint"].as_str().unwrap().contains("service"));
}

#[tokio::test]
async fn diff_versions_and_blame_answer_text_and_blocks() {
    let seed = seed().await;
    let diff = call_json(
        &seed.app,
        &seed.full,
        "diff_versions",
        json!({ "service": "orders", "api_type": "openapi", "from": "1.0.0", "to": "1.1.0" }),
    )
    .await;
    assert_eq!(diff["from"], "1.0.0");
    assert_eq!(diff["to"], "1.1.0");
    let text = diff["diff"].as_str().unwrap();
    assert!(
        text.contains("--- orders 1.0.0") && text.contains("+++ orders 1.1.0"),
        "{text}"
    );
    assert!(text.contains("+    post:"), "{text}");
    assert!(diff["added"].as_u64().unwrap() >= 3);
    assert!(diff["removed"].as_u64().unwrap() >= 3);

    let snippet = call_json(
        &seed.app,
        &seed.full,
        "diff_versions",
        json!({ "service": "orders", "api_type": "openapi", "from": "1.0.0", "to": "1.1.0", "path": "/orders/{id}", "method": "get", "ignore_whitespace": true }),
    )
    .await;
    // An endpoint snippet carries its document header, so only the version
    // line differs between the two.
    assert_eq!(snippet["added"], 1, "{snippet}");
    assert_eq!(snippet["removed"], 1, "{snippet}");
    assert!(
        snippet["diff"]
            .as_str()
            .unwrap()
            .contains("+  version: 1.1.0")
    );

    let (is_error, text) = call(
        &seed.app,
        &seed.full,
        "diff_versions",
        json!({ "service": "orders", "api_type": "openapi", "from": "1.0.0", "to": "2.0.0" }),
    )
    .await;
    assert!(is_error && text.contains("2.0.0"), "{text}");

    let blame = call_json(
        &seed.app,
        &seed.full,
        "blame",
        json!({ "service": "orders", "api_type": "openapi", "version": "1.1.0" }),
    )
    .await;
    let blocks = blame["blocks"].as_array().unwrap();
    assert!(blocks.len() >= 2, "{blame}");
    assert!(blocks.iter().any(|b| b["version"] == "1.0.0"));
    assert!(blocks.iter().any(|b| b["version"] == "1.1.0"));
    assert_eq!(blocks[0]["start_line"], 1);
    assert_eq!(blocks[0]["provided_by"], "test-releaser");
    assert!(blocks[0]["lines"].is_array());
    assert!(blocks[0]["date"].is_string());

    let (is_error, text) = call(
        &seed.app,
        &seed.full,
        "blame",
        json!({ "service": "orders", "api_type": "asyncapi", "version": "1.1.0" }),
    )
    .await;
    assert!(is_error && text.contains("asyncapi"), "{text}");

    let seed = seed_with(McpConfig {
        max_result_bytes: 200,
        ..Default::default()
    })
    .await;
    let bare = call_json(
        &seed.app,
        &seed.full,
        "blame",
        json!({ "service": "orders", "api_type": "openapi", "version": "1.1.0" }),
    )
    .await;
    assert!(bare["blocks"][0].get("lines").is_none());
    assert!(bare["blocks"][0]["line_count"].as_u64().unwrap() >= 1);
    assert!(bare["hint"].as_str().unwrap().contains("path + method"));
    let counts = call_json(
        &seed.app,
        &seed.full,
        "diff_versions",
        json!({ "service": "orders", "api_type": "openapi", "from": "1.0.0", "to": "1.1.0" }),
    )
    .await;
    assert!(counts.get("diff").is_none());
    assert!(counts["added"].as_u64().unwrap() >= 3);
}

#[tokio::test]
async fn tool_answers_are_compact_json_and_counted_in_metrics() {
    let seed = seed().await;
    let (is_error, text) = call(&seed.app, &seed.full, "list_services", json!({})).await;
    assert!(!is_error);
    assert!(!text.contains("\n  "), "no pretty-printing: {text}");
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
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let metrics = String::from_utf8_lossy(&bytes);
    assert!(
        metrics.contains("sanshain_mcp_tool_calls_total{")
            && metrics.contains("tool=\"list_services\""),
        "{metrics}"
    );
    assert!(
        metrics.contains("sanshain_mcp_result_bytes_total{"),
        "{metrics}"
    );
}

#[tokio::test]
async fn endpoint_structure_answers_the_tree_with_last_changed_versions() {
    let seed = seed().await;
    let get = call_json(
        &seed.app,
        &seed.read_only,
        "endpoint_structure",
        json!({ "service": "orders", "api_type": "openapi", "path": "/orders/{id}", "method": "get" }),
    )
    .await;
    assert_eq!(get["version"], "1.1.0", "the latest version by default");
    let endpoint = &get["endpoint"];
    assert_eq!(endpoint["name"], "GET /orders/{id}");
    assert_eq!(endpoint["summary"], "Fetch one order");
    assert_eq!(endpoint["changed_in"], "1.0.0", "{get}");
    let ok = &endpoint["children"][0]["children"][0];
    assert_eq!(ok["key"], "200");
    assert_eq!(ok["description"], "ok");

    let post = call_json(
        &seed.app,
        &seed.read_only,
        "endpoint_structure",
        json!({ "service": "orders", "api_type": "openapi", "path": "/orders", "method": "POST", "with_examples": true }),
    )
    .await;
    assert_eq!(post["endpoint"]["changed_in"], "1.1.0");

    let (is_error, text) = call(
        &seed.app,
        &seed.read_only,
        "endpoint_structure",
        json!({ "service": "orders", "api_type": "openapi", "version": "1.0.0", "path": "/orders", "method": "post" }),
    )
    .await;
    assert!(
        is_error && text.contains("has no endpoint POST /orders"),
        "{text}"
    );

    let seed = seed_with(McpConfig {
        max_result_bytes: 400,
        ..Default::default()
    })
    .await;
    let cut = call_json(
        &seed.app,
        &seed.full,
        "endpoint_structure",
        json!({ "service": "orders", "api_type": "openapi", "path": "/orders/{id}", "method": "get" }),
    )
    .await;
    assert!(
        cut["hint"].as_str().unwrap().contains("two levels"),
        "{cut}"
    );
}
