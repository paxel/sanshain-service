//! The MCP endpoint (https://paxel.github.io/sanshain-service/developer/mcp.html): the Streamable HTTP transport at `/mcp`,
//! its bearer-token guard, the per-token rate limit, the result-size budget
//! and the Prometheus counters. The answers themselves come from
//! [`crate::application::mcp_service`]; this module only frames them.
//!
//! Stateless: every request carries its own token, so no session is kept and
//! an instance restart loses nothing a client could notice.

use crate::AppState;
use crate::application::services;
use crate::domain::models::{ApiTokenScope, AppError, AuthMode, User};
use crate::presentation::middleware::Refusal;
use axum::{
    extract::{Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{MethodRouter, any},
};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData as McpError,
    Implementation, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{RoleServer, ServerHandler};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

pub mod tools;

/// `MCP_CALLS_PER_MINUTE` when unset.
pub const DEFAULT_MCP_CALLS_PER_MINUTE: u32 = 60;
/// `MCP_MAX_RESULT_BYTES` when unset.
pub const DEFAULT_MCP_MAX_RESULT_BYTES: usize = 65_536;

/// How the MCP endpoint is configured (`MCP_ENABLED`, `MCP_CALLS_PER_MINUTE`,
/// `MCP_MAX_RESULT_BYTES`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpConfig {
    /// When false the route is not mounted at all: `/mcp` is a 404.
    pub enabled: bool,
    /// Tool calls a token may make per minute (token bucket).
    pub calls_per_minute: u32,
    /// A tool answer above this many bytes is replaced by its tool's summary.
    pub max_result_bytes: usize,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            calls_per_minute: DEFAULT_MCP_CALLS_PER_MINUTE,
            max_result_bytes: DEFAULT_MCP_MAX_RESULT_BYTES,
        }
    }
}

/// The authenticated caller of an MCP request, attached to the HTTP request
/// by [`mcp_auth`] and read by the server for every tool call.
#[derive(Clone, Debug)]
pub struct McpCaller {
    pub user: User,
    pub scope: ApiTokenScope,
    /// What the rate limit is keyed on.
    pub token_id: String,
}

/// The token id the dev-mode substitute user is rate-limited under.
const DEV_MODE_TOKEN_ID: &str = "dev-mode";

fn unauthorized(message: &str, error_code: Option<&str>) -> Response {
    let challenge = match error_code {
        Some(code) => format!("Bearer realm=\"sanshain\", error=\"{code}\""),
        None => "Bearer realm=\"sanshain\"".to_string(),
    };
    let mut response = (
        StatusCode::UNAUTHORIZED,
        axum::Json(serde_json::json!({ "error": message })),
    )
        .into_response();
    if let Ok(value) = HeaderValue::from_str(&challenge) {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, value);
    }
    response
}

/// Guard for `/mcp`: `Authorization: Bearer <API token>` only, both scopes.
/// Session tokens are not accepted — an AI client holds a token minted for
/// it, never a browser session. A missing or invalid header is a `401` with
/// `WWW-Authenticate: Bearer`. Dev mode substitutes the dev user exactly as
/// the API guard does: only when no header was sent at all.
pub async fn mcp_auth(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, Refusal> {
    if let Ok(AuthMode::Disabled) = services::get_auth_mode(&state.repo).await {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Service is in Maintenance Mode / Disabled",
        )
            .into_response()
            .into());
    }
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .map(str::to_string);
    let caller = match auth_header {
        Some(value) => {
            let Some(token) = value.strip_prefix("Bearer ") else {
                return Err(unauthorized(
                    "the MCP endpoint takes `Authorization: Bearer <API token>`",
                    Some("invalid_request"),
                )
                .into());
            };
            match services::validate_api_token(&state.repo, token.trim()).await {
                Ok(Some(identity)) => McpCaller {
                    user: identity.user,
                    scope: identity.scope,
                    token_id: identity.token_id,
                },
                _ => {
                    return Err(unauthorized(
                        "unknown or expired API token (session tokens are not accepted here)",
                        Some("invalid_token"),
                    )
                    .into());
                }
            }
        }
        None => {
            let dev_user = if services::get_dev_mode(&state.repo).await.unwrap_or(false) {
                match &state.dev_user {
                    Some(u) => Some(u.clone()),
                    None => services::ensure_dev_user(&state.repo).await.ok(),
                }
            } else {
                None
            };
            match dev_user {
                Some(user) => McpCaller {
                    user,
                    scope: ApiTokenScope::Full,
                    token_id: DEV_MODE_TOKEN_ID.to_string(),
                },
                None => {
                    return Err(unauthorized(
                        "the MCP endpoint needs `Authorization: Bearer <API token>`; mint one on the account page",
                        None,
                    )
                    .into());
                }
            }
        }
    };
    let mut req = req;
    if let Some(slot) = req
        .extensions()
        .get::<crate::presentation::middleware::CallerSlot>()
    {
        slot.set(&caller.user.username);
    }
    req.extensions_mut().insert(caller);
    Ok(next.run(req).await)
}

/// One token bucket per token id: `per_minute` calls, refilled continuously.
pub struct RateLimiter {
    per_minute: u32,
    buckets: Mutex<HashMap<String, Bucket>>,
}

struct Bucket {
    tokens: f64,
    refilled_at: Instant,
}

impl RateLimiter {
    pub fn new(per_minute: u32) -> Self {
        Self {
            // Zero would refuse every call; the configuration refuses it
            // first, this only keeps the arithmetic sane.
            per_minute: per_minute.max(1),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Take one call from `key`'s bucket, or say how long until one is
    /// available again.
    pub fn try_acquire(&self, key: &str) -> Result<(), Duration> {
        self.try_acquire_at(key, Instant::now())
    }

    pub fn try_acquire_at(&self, key: &str, now: Instant) -> Result<(), Duration> {
        let capacity = f64::from(self.per_minute);
        let per_second = capacity / 60.0;
        let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        let bucket = buckets.entry(key.to_string()).or_insert(Bucket {
            tokens: capacity,
            refilled_at: now,
        });
        let elapsed = now
            .saturating_duration_since(bucket.refilled_at)
            .as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * per_second).min(capacity);
        bucket.refilled_at = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            let wait = (1.0 - bucket.tokens) / per_second;
            Err(Duration::from_secs_f64(wait))
        }
    }
}

/// The text a rate-limited call answers, as an `isError` tool result —
/// never an HTTP 429, which would break the client's session.
pub fn rate_limited_message(wait: Duration) -> String {
    let seconds = wait.as_secs_f64().ceil().max(1.0) as u64;
    format!("rate limited - retry in {seconds} s")
}

/// A tool's answer: the full body, and what to answer instead when the body
/// does not fit the size budget. Both are compact JSON.
pub struct ToolOutput {
    pub body: Value,
    pub over_budget: Value,
}

/// A refusal the client should read: unknown service, bad parameters, a
/// failing lookup. Answered as an `isError` result, never a transport error.
#[derive(Debug)]
pub struct ToolError(pub String);

impl From<AppError> for ToolError {
    fn from(e: AppError) -> Self {
        ToolError(e.to_string())
    }
}

fn record_call(tool: &str, outcome: &'static str, bytes: usize) {
    metrics::counter!(
        "sanshain_mcp_tool_calls_total",
        "tool" => tool.to_string(),
        "outcome" => outcome
    )
    .increment(1);
    if bytes > 0 {
        metrics::counter!("sanshain_mcp_result_bytes_total", "tool" => tool.to_string())
            .increment(bytes as u64);
    }
}

/// Serialize an answer within the budget, or its over-budget replacement.
fn frame(tool: &str, output: ToolOutput, max_bytes: usize) -> CallToolResult {
    let body = output.body.to_string();
    if body.len() <= max_bytes {
        record_call(tool, "ok", body.len());
        return CallToolResult::success(vec![ContentBlock::text(body)]);
    }
    let replacement = output.over_budget.to_string();
    record_call(tool, "over_budget", replacement.len());
    CallToolResult::success(vec![ContentBlock::text(replacement)])
}

const INSTRUCTIONS: &str = "Sanshain is the catalog of the APIs the organisation's services provide and require. \
Start with list_services to learn the names, then describe_service for one service, find_endpoint to \
locate an operation by path or name, get_spec for the stored specification text, get_graph for who \
depends on whom, and diff_versions / blame for how a specification changed. Scopes: `dev` (default) is \
everything ever provided or required, `main` is what trunk builds currently pin, a sanshain-branch name \
is a release cut. Answers are compact JSON; a large answer is replaced by a summary with a hint on how to narrow it.";

/// The MCP server behind `/mcp`. One instance serves one request; the caller
/// is read from that request's extensions.
#[derive(Clone)]
pub struct SanshainMcp {
    state: AppState,
    limiter: Arc<RateLimiter>,
}

impl SanshainMcp {
    fn caller(context: &RequestContext<RoleServer>) -> Option<McpCaller> {
        context
            .extensions
            .get::<axum::http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<McpCaller>().cloned())
    }
}

impl ServerHandler for SanshainMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("sanshain", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        tools::catalog()
            .map(ListToolsResult::with_all_items)
            .map_err(|e| McpError::internal_error(e, None))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let Some(caller) = Self::caller(&context) else {
            return Err(McpError::invalid_request(
                "no authenticated caller on this request",
                None,
            ));
        };
        let tool = request.name.to_string();
        if let Err(wait) = self.limiter.try_acquire(&caller.token_id) {
            record_call(&tool, "rate_limited", 0);
            return Ok(
                CallToolResult::error(vec![ContentBlock::text(rate_limited_message(wait))]).into(),
            );
        }
        let arguments = request
            .arguments
            .map(Value::Object)
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        match tools::dispatch(&self.state, &tool, arguments).await {
            Ok(output) => Ok(frame(&tool, output, self.state.mcp.max_result_bytes).into()),
            Err(ToolError(message)) => {
                record_call(&tool, "error", 0);
                Ok(CallToolResult::error(vec![ContentBlock::text(message)]).into())
            }
        }
    }
}

/// The `/mcp` method router: the transport in stateless mode, JSON answers
/// for plain request/response tools. Host validation is off — Sanshain is
/// reached under whatever name the deployment gave it, and the bearer token
/// is what keeps a rebinding attacker out, not the `Host` header.
pub fn method_router(state: AppState) -> MethodRouter<AppState> {
    let limiter = Arc::new(RateLimiter::new(state.mcp.calls_per_minute));
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_max_request_body_bytes(state.max_body_bytes)
        .disable_allowed_hosts();
    let service = StreamableHttpService::new(
        move || {
            Ok(SanshainMcp {
                state: state.clone(),
                limiter: limiter.clone(),
            })
        },
        Arc::new(NeverSessionManager::default()),
        config,
    );
    any(move |req: Request| {
        let service = service.clone();
        async move {
            let (parts, body) = service.handle(req).await.into_parts();
            Response::from_parts(parts, axum::body::Body::new(body))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_match_the_documented_values() {
        let config = McpConfig::default();
        assert!(config.enabled);
        assert_eq!(config.calls_per_minute, 60);
        assert_eq!(config.max_result_bytes, 65_536);
    }

    #[test]
    fn rate_limiter_allows_the_burst_then_refills_over_time() {
        let limiter = RateLimiter::new(60);
        let start = Instant::now();
        for _ in 0..60 {
            assert!(limiter.try_acquire_at("t", start).is_ok());
        }
        let wait = limiter.try_acquire_at("t", start).unwrap_err();
        assert!(wait > Duration::ZERO && wait <= Duration::from_secs(1));
        assert_eq!(rate_limited_message(wait), "rate limited - retry in 1 s");
        // Another token has its own bucket.
        assert!(limiter.try_acquire_at("other", start).is_ok());
        // One second later one call has refilled.
        assert!(
            limiter
                .try_acquire_at("t", start + Duration::from_secs(1))
                .is_ok()
        );
        assert!(
            limiter
                .try_acquire_at("t", start + Duration::from_secs(1))
                .is_err()
        );
    }

    #[test]
    fn rate_limited_message_rounds_up_to_whole_seconds() {
        assert_eq!(
            rate_limited_message(Duration::from_millis(1)),
            "rate limited - retry in 1 s"
        );
        assert_eq!(
            rate_limited_message(Duration::from_millis(2500)),
            "rate limited - retry in 3 s"
        );
        let limiter = RateLimiter::new(1);
        let start = Instant::now();
        assert!(limiter.try_acquire_at("t", start).is_ok());
        let wait = limiter.try_acquire_at("t", start).unwrap_err();
        assert_eq!(rate_limited_message(wait), "rate limited - retry in 60 s");
    }

    #[test]
    fn frame_replaces_an_answer_over_budget() {
        let output = ToolOutput {
            body: serde_json::json!({ "text": "x".repeat(100) }),
            over_budget: serde_json::json!({ "hint": "narrow" }),
        };
        let framed = frame("t", output, 32);
        assert_eq!(framed.is_error, Some(false));
        let text = framed.content[0].as_text().map(|t| t.text.clone()).unwrap();
        assert_eq!(text, r#"{"hint":"narrow"}"#);

        let output = ToolOutput {
            body: serde_json::json!({ "a": 1 }),
            over_budget: serde_json::json!({ "hint": "narrow" }),
        };
        let framed = frame("t", output, 32);
        let text = framed.content[0].as_text().map(|t| t.text.clone()).unwrap();
        assert_eq!(text, r#"{"a":1}"#, "compact JSON, never pretty-printed");
    }

    #[test]
    fn tool_errors_carry_the_application_message() {
        let err: ToolError = AppError::NotFound("no Producer 'x'".to_string()).into();
        assert_eq!(err.0, "Not Found: no Producer 'x'");
    }
}
