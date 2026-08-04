use crate::AppState;
use crate::application::services::{self, AppError};
use crate::domain::models::{ApiType, SemVer, Stability};
use crate::domain::ports::{NewAuditLog, SpecRepository};
use axum::{
    Json,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{
        IntoResponse,
        sse::{Event, Sse},
    },
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::convert::Infallible;
use tokio_stream::Stream;

async fn record_audit_log(
    repo: &impl crate::domain::ports::SpecRepository,
    user: Option<axum::Extension<crate::domain::models::User>>,
    log: crate::domain::ports::NewAuditLog<'_>,
) -> Result<(), AppError> {
    let actor = if let Some(axum::Extension(u)) = user {
        u.username.clone()
    } else {
        "DevMode/Anonymous".to_string()
    };
    repo.insert_audit_log(&actor, log)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))
}

/// The 2.0 Provide payload. The version is *not* here — it lives in the spec
/// document itself. `deny_unknown_fields` turns a 1.x-shaped request (`branch`,
/// `base_version`, `force`, ...) into an immediate, named 422 instead of a
/// silently ignored parameter.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvideRequest {
    pub producername: String,
    pub openapi_yaml: String,
    /// Declared by the caller: `snapshot` (overwritable) or `ga` (immutable).
    /// The branch→stability translation is Tooling's business; Sanshain only
    /// ever hears the intent.
    pub stability: Stability,
    #[serde(default)]
    pub dry_run: bool,
}

struct ProvideCommon<'a> {
    api_type: ApiType,
    producername: &'a str,
    content: &'a str,
    stability: Stability,
    dry_run: bool,
}

async fn provide_common(
    state: &AppState,
    user: Option<axum::Extension<crate::domain::models::User>>,
    params: ProvideCommon<'_>,
) -> Result<impl IntoResponse + use<>, AppError> {
    let ProvideCommon {
        api_type,
        producername,
        content,
        stability,
        dry_run,
    } = params;
    let actor = if let Some(axum::Extension(ref u)) = user {
        u.username.clone()
    } else {
        "DevMode/Anonymous".to_string()
    };

    let res = services::provide_spec(
        &state.repo,
        services::ProvideSpecParams {
            producername,
            api_type,
            content,
            stability,
            dry_run,
            username: Some(&actor),
        },
    )
    .await?;

    if !dry_run {
        // Skip both the broadcast and the audit entry for a no-op re-upload (no
        // endpoint changes): nothing changed, so there is nothing for a listener
        // to refetch and nothing worth recording. A same-content promotion is
        // the exception — the endpoints are unchanged but the stability
        // flipped, which listeners (and the graph) do care about. Its audit
        // trail is the VERSION_PROMOTED entry the application layer writes.
        if res.promoted || !res.changes.is_empty() {
            let _ = state.spec_updated_tx.send(());
        }
        if !res.changes.is_empty() {
            let version_str = res.version.to_string();
            record_audit_log(
                &state.repo,
                user,
                NewAuditLog {
                    action: "PROVIDE_SPEC",
                    details: &format!(
                        "Provided {:?} spec {} for '{}' as {} (changes: +{}, ~{}, -{})",
                        api_type,
                        res.version,
                        producername,
                        res.stability.as_str(),
                        res.changes.inserts,
                        res.changes.updates,
                        res.changes.deletes
                    ),
                    service: Some(producername),
                    version: Some(&version_str),
                    action_type: Some("WRITE"),
                    diff: None,
                },
            )
            .await?;
        }
    }

    Ok((StatusCode::ACCEPTED, Json(res)))
}

pub async fn provide(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<ProvideRequest>,
) -> Result<impl IntoResponse, AppError> {
    provide_common(
        &state,
        user,
        ProvideCommon {
            api_type: ApiType::OpenApi,
            producername: &payload.producername,
            content: &payload.openapi_yaml,
            stability: payload.stability,
            dry_run: payload.dry_run,
        },
    )
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvideAsyncApiRequest {
    pub producername: String,
    pub asyncapi_yaml: String,
    /// See `ProvideRequest::stability`.
    pub stability: Stability,
    #[serde(default)]
    pub dry_run: bool,
}

pub async fn provide_asyncapi(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<ProvideAsyncApiRequest>,
) -> Result<impl IntoResponse, AppError> {
    provide_common(
        &state,
        user,
        ProvideCommon {
            api_type: ApiType::AsyncApi,
            producername: &payload.producername,
            content: &payload.asyncapi_yaml,
            stability: payload.stability,
            dry_run: payload.dry_run,
        },
    )
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvideProtoRequest {
    pub producername: String,
    pub proto_content: String,
    /// See `ProvideRequest::stability`.
    pub stability: Stability,
    #[serde(default)]
    pub dry_run: bool,
}

pub async fn provide_proto(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<ProvideProtoRequest>,
) -> Result<impl IntoResponse, AppError> {
    provide_common(
        &state,
        user,
        ProvideCommon {
            api_type: ApiType::Proto,
            producername: &payload.producername,
            content: &payload.proto_content,
            stability: payload.stability,
            dry_run: payload.dry_run,
        },
    )
    .await
}

/// The 2.0 Require query: an exact Pin, nothing else. `deny_unknown_fields`
/// turns 1.x parameters (`branch`, `timeout`, `pull_from_branch`, ...) into a
/// named 400 (query-string errors answer 400; JSON body-shape errors 422).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequireQuery {
    pub consumername: String,
    pub producername: String,
    /// The exact pinned version — no ranges, no "latest", no default.
    pub version: SemVer,
    pub path: String,
    pub method: String,
    #[serde(default)]
    pub dry_run: bool,
}

/// Name how a require was answered: the resolution state, the version (always
/// the Pin) and the stability it was actually served from, so a Consumer can
/// see it is building against overwritable content without parsing the body.
fn insert_resolution_headers(headers: &mut HeaderMap, res: &services::RequireResponse) {
    headers.insert(
        "X-Sanshain-Resolution",
        HeaderValue::from_static(res.state.as_str()),
    );
    if let Ok(value) = HeaderValue::from_str(&res.version.to_string()) {
        headers.insert("X-Sanshain-Version", value);
    }
    headers.insert(
        "X-Sanshain-Stability",
        HeaderValue::from_static(res.stability.as_str()),
    );
}

fn require_response(
    request_headers: &HeaderMap,
    res: services::RequireResponse,
) -> axum::response::Response {
    let etag = format!("\"{}\"", hex::encode(Sha256::digest(res.yaml.as_bytes())));
    let etag_value = HeaderValue::from_str(&etag).ok();
    if let Some(ev) = &etag_value
        && request_headers.get("if-none-match") == Some(ev)
    {
        return (StatusCode::NOT_MODIFIED, HeaderMap::new()).into_response();
    }

    let mut headers = HeaderMap::new();
    if let Some(ev) = etag_value {
        headers.insert("ETag", ev);
    }
    if res.deprecated {
        headers.insert("X-Sanshain-Deprecated", HeaderValue::from_static("true"));
    }
    insert_resolution_headers(&mut headers, &res);
    (headers, res.yaml).into_response()
}

async fn require_common(
    state: &AppState,
    user: Option<axum::Extension<crate::domain::models::User>>,
    query: RequireQuery,
    api_type: ApiType,
    request_headers: HeaderMap,
) -> Result<axum::response::Response, AppError> {
    let params = services::RequireEndpointParams {
        consumername: &query.consumername,
        producername: &query.producername,
        version: query.version,
        api_type,
        path: &query.path,
        method: &query.method,
    };
    let res = if query.dry_run {
        services::require_endpoint_dry_run(&state.repo, params).await?
    } else {
        let res = services::require_endpoint(&state.repo, params).await?;
        let version_str = query.version.to_string();
        let _ = record_audit_log(
            &state.repo,
            user,
            NewAuditLog {
                action: "REQUIRE_SPEC",
                details: &format!(
                    "Consumer '{}' required {:?} {} {} of '{}' at version {} ({})",
                    query.consumername,
                    api_type,
                    query.method,
                    query.path,
                    query.producername,
                    query.version,
                    res.stability.as_str()
                ),
                service: Some(&query.producername),
                version: Some(&version_str),
                action_type: Some("READ"),
                diff: None,
            },
        )
        .await;
        res
    };

    Ok(require_response(&request_headers, res))
}

pub async fn require(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Query(query): Query<RequireQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    require_common(&state, user, query, ApiType::OpenApi, headers).await
}

pub async fn require_asyncapi(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Query(query): Query<RequireQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    require_common(&state, user, query, ApiType::AsyncApi, headers).await
}

pub async fn require_proto(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Query(query): Query<RequireQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    require_common(&state, user, query, ApiType::Proto, headers).await
}

#[derive(Deserialize)]
pub struct BundleEndpoint {
    pub path: String,
    pub method: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequireBundleRequest {
    pub consumername: String,
    pub producername: String,
    /// The exact pinned version — see `RequireQuery::version`.
    pub version: SemVer,
    pub api_type: Option<ApiType>,
    pub endpoints: Vec<BundleEndpoint>,
    #[serde(default)]
    pub dry_run: bool,
}

pub async fn require_bundle(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    headers: HeaderMap,
    Json(payload): Json<RequireBundleRequest>,
) -> Result<impl IntoResponse, AppError> {
    let endpoints: Vec<(String, String)> = payload
        .endpoints
        .into_iter()
        .map(|e| (e.path, e.method))
        .collect();

    let params = services::RequireBundleParams {
        consumername: &payload.consumername,
        producername: &payload.producername,
        version: payload.version,
        api_type: payload.api_type.unwrap_or(ApiType::OpenApi),
        endpoints: &endpoints,
    };
    let res = if payload.dry_run {
        services::require_bundle_dry_run(&state.repo, params).await?
    } else {
        let res = services::require_bundle(&state.repo, params).await?;
        let version_str = payload.version.to_string();
        let _ = record_audit_log(
            &state.repo,
            user,
            NewAuditLog {
                action: "REQUIRE_SPEC",
                details: &format!(
                    "Consumer '{}' required a bundle of {} endpoints of '{}' at version {} ({})",
                    payload.consumername,
                    endpoints.len(),
                    payload.producername,
                    payload.version,
                    res.stability.as_str()
                ),
                service: Some(&payload.producername),
                version: Some(&version_str),
                action_type: Some("READ"),
                diff: None,
            },
        )
        .await;
        res
    };

    Ok(require_response(&headers, res))
}

pub async fn report(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    // Not audited: this JSON report is fetched to render the graph view in the
    // UI, so auditing it would turn every graph view into an audit entry.
    // Explicit report exports (markdown/isolation) are still audited below.
    let res = services::generate_report(&state.repo).await?;
    Ok(Json(res))
}

pub async fn report_markdown(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::generate_report(&state.repo).await?;
    let _ = record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "REPORT",
            details: "Generated Markdown dependency report",
            service: None,
            version: None,
            action_type: Some("READ"),
            diff: None,
        },
    )
    .await;
    Ok((
        [(
            axum::http::header::CONTENT_TYPE,
            "text/markdown; charset=utf-8",
        )],
        services::render_report_markdown(&res),
    ))
}

pub async fn report_isolation(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::generate_report(&state.repo).await?;
    let _ = record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "REPORT",
            details: "Generated Isolation report",
            service: None,
            version: None,
            action_type: Some("READ"),
            diff: None,
        },
    )
    .await;
    Ok(services::render_isolation_report(&res))
}

#[derive(Deserialize)]
pub struct ProducerVersionsQuery {
    pub api_type: Option<ApiType>,
}

/// `GET /producers/{producername}/versions` — the version lines of one
/// Producer: version, stability, content hash, timestamps, last provider,
/// endpoint count, and (for snapshots) the use-based expiry. What both the UI
/// views and "what can I upgrade to?" tooling read.
pub async fn producer_versions(
    State(state): State<AppState>,
    Path(producername): Path<String>,
    Query(query): Query<ProducerVersionsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let versions =
        services::list_producer_versions(&state.repo, &producername, query.api_type).await?;
    Ok(Json(versions))
}

#[derive(Deserialize)]
pub struct EndpointHistoryQuery {
    pub service: String,
    pub api_type: Option<ApiType>,
    pub path: String,
    pub method: String,
}

pub async fn endpoint_versions(
    State(state): State<AppState>,
    Query(query): Query<EndpointHistoryQuery>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::get_endpoint_history(
        &state.repo,
        &query.service,
        query.api_type.unwrap_or(ApiType::OpenApi),
        &query.path,
        &query.method,
    )
    .await?;
    Ok(Json(res))
}

#[derive(Deserialize)]
pub struct AuditTimelineQuery {
    pub limit: Option<u32>,
    pub from_date: Option<String>,
    pub to_date: Option<String>,
    pub action_type: Option<String>,
    pub service: Option<String>,
    pub version: Option<String>,
}

pub async fn audit_timeline(
    State(state): State<AppState>,
    Query(query): Query<AuditTimelineQuery>,
) -> Result<impl IntoResponse, AppError> {
    let limit = query.limit.unwrap_or(50);
    let filter = crate::domain::models::AuditLogFilter {
        from_date: query.from_date,
        to_date: query.to_date,
        action_type: query.action_type,
        service_wildcard: query.service,
        version_wildcard: query.version,
        limit,
    };
    let res = state.repo.get_audit_logs(filter).await?;
    Ok(Json(res))
}

/// The free, unauthenticated spec validator behind the landing page: runs
/// exactly the provide-side pipeline (version extraction, splitting) with no
/// persistence and no producer context. Always answers 200 with a verdict.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidateRequest {
    pub api_type: ApiType,
    pub content: String,
}

pub async fn validate(Json(payload): Json<ValidateRequest>) -> impl IntoResponse {
    Json(services::validate_spec(payload.api_type, &payload.content))
}

pub async fn sse_updates(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut receiver = state.spec_updated_tx.subscribe();

    let stream = async_stream::stream! {
        loop {
            match receiver.recv().await {
                Ok(_) => {
                    yield Ok(Event::default().data("updated"));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    };

    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}

pub async fn ws_updates(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let mut rx = state.spec_updated_tx.subscribe();

    loop {
        tokio::select! {
            _ = rx.recv() => {
                if socket.send(Message::Text("updated".into())).await.is_err() {
                    break;
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}
