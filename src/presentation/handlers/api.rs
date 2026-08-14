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

use super::record_audit_log;

/// The 2.0 Provide payload. The version is *not* here — it lives in the spec
/// document itself. `deny_unknown_fields` turns a 1.x-shaped request (`branch`,
/// `base_version`, `force`, ...) into an immediate, named 422 instead of a
/// silently ignored parameter.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvideRequest {
    pub producername: String,
    /// Absent only on a retire — see [`ProvideRequest::retired`].
    pub openapi_yaml: Option<String>,
    /// Declared by the caller: `snapshot` (overwritable) or `ga` (immutable).
    /// The branch→stability translation is Tooling's business; Sanshain only
    /// ever hears the intent. Absent only on a retire, which publishes nothing
    /// and so has no stability to declare.
    pub stability: Option<Stability>,
    #[serde(default)]
    pub dry_run: bool,
    /// ADR-0004: marks this build as the trunk stream's — the version entry
    /// becomes "trunk's current version". Orthogonal to stability.
    #[serde(default)]
    pub trunk: bool,
    /// ADR-0005: marks this build as a sanshain-branch's (hotfix) — the
    /// producer's member version within that branch. Exclusive with `trunk`.
    #[serde(default)]
    pub tag: Option<String>,
    /// The Producer no longer provides this family: retire the capability
    /// rather than publish a version. Carried on the provide call because the
    /// endpoint already names the family, and because dropping a protocol is
    /// something a build knows and a human should not have to remember.
    ///
    /// Exclusive with everything a publish needs — a body, a `stability`, a
    /// stream. Retiring is not a build of anything, so a request that says both
    /// is a contradiction rather than a request with defaults to fill in.
    #[serde(default)]
    pub retired: bool,
}

struct ProvideCommon<'a> {
    api_type: ApiType,
    producername: &'a str,
    content: Option<&'a str>,
    /// The name of the body field on *this* endpoint's payload, so a request
    /// that omits it is refused in its own vocabulary rather than a generic one.
    content_field: &'static str,
    stability: Option<Stability>,
    dry_run: bool,
    trunk: bool,
    tag: Option<&'a str>,
    retired: bool,
}

/// The rejection a missing field produced when the deserializer still required
/// it: same `422`, same wording. `retired` made two fields conditional, and
/// conditional fields have to be checked by hand — but a Producer that simply
/// forgot one should not be able to tell that anything about the payload
/// changed.
fn missing_field(field: &str) -> axum::response::Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        format!(
            "Failed to deserialize the JSON body into the target type: missing field `{field}`"
        ),
    )
        .into_response()
}

/// The id behind a `tag`, for the audit row's branch identity. The stream was
/// already validated upstream, so a miss here means the branch vanished between
/// the write and this lookup — the row then keeps only its recorded label.
async fn audit_branch_id(state: &AppState, tag: Option<&str>) -> Option<i64> {
    match tag {
        Some(tag) => state
            .repo
            .find_branch(tag)
            .await
            .ok()
            .flatten()
            .map(|b| b.id),
        None => None,
    }
}

async fn provide_common(
    state: &AppState,
    user: Option<axum::Extension<crate::domain::models::User>>,
    caller: Option<axum::Extension<crate::domain::permissions::Actor>>,
    params: ProvideCommon<'_>,
) -> Result<axum::response::Response, AppError> {
    let ProvideCommon {
        api_type,
        producername,
        content,
        content_field,
        stability,
        dry_run,
        trunk,
        tag,
        retired,
    } = params;

    if retired {
        return retire_common(
            state,
            user,
            caller,
            RetireCommon {
                api_type,
                producername,
                content,
                stability,
                dry_run,
                trunk,
                tag,
            },
        )
        .await;
    }

    // Absent only on a retire, which returned above. Both fields were required
    // by the deserializer until `retired` made them conditional, and a caller
    // that forgets one is in exactly the situation it was before — so it keeps
    // the answer it had, naming the field the way serde did. Widening a payload
    // must not quietly re-code an error every Producer's CI already handles.
    let (content, stability) = match (content, stability) {
        (Some(content), Some(stability)) => (content, stability),
        (None, _) => return Ok(missing_field(content_field)),
        (_, None) => return Ok(missing_field("stability")),
    };

    let res = services::provide_spec(
        &state.repo,
        services::ProvideSpecParams {
            producername,
            api_type,
            content,
            stability,
            dry_run,
            trunk,
            tag,
            caller: caller.map(|axum::Extension(a)| a),
            require_prior_content_match: false,
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
            // ADR-0005: the audit entry names the declared stream, so a
            // release graph's history is one filtered query.
            let stream = if trunk { Some("trunk") } else { tag };
            let branch_id = audit_branch_id(state, tag).await;
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
                    stream,
                    branch_id,
                },
            )
            .await?;
        } else if let Some(tag) = tag {
            // A no-op provide changes no endpoint, but a *tagged* one still
            // marks the producer's member version in that release graph — a
            // real mutation of the branch, and the one ADR-0005 wants findable
            // by `stream`. (The trunk-marker refresh above stays unaudited on
            // purpose: nightly trunk CI would write a row per producer per
            // night for content nobody changed.)
            let version_str = res.version.to_string();
            record_audit_log(
                &state.repo,
                user,
                NewAuditLog {
                    action: "BRANCH_MEMBER_TAGGED",
                    details: &format!(
                        "Tagged {:?} {} of '{}' into sanshain-branch '{}'",
                        api_type, res.version, producername, tag
                    ),
                    service: Some(producername),
                    version: Some(&version_str),
                    action_type: Some("WRITE"),
                    diff: None,
                    stream: Some(tag),
                    branch_id: audit_branch_id(state, Some(tag)).await,
                },
            )
            .await?;
        }
    }

    Ok((StatusCode::ACCEPTED, Json(res)).into_response())
}

/// The retire half of a provide call, split out so the publish path above reads
/// as one story. Everything a publish carries is refused here rather than
/// ignored: a caller that sends a body *and* `retired` has two different
/// intentions in one request, and silently honouring one of them is how a
/// pipeline retires a family it meant to publish.
struct RetireCommon<'a> {
    api_type: ApiType,
    producername: &'a str,
    content: Option<&'a str>,
    stability: Option<Stability>,
    dry_run: bool,
    trunk: bool,
    tag: Option<&'a str>,
}

async fn retire_common(
    state: &AppState,
    user: Option<axum::Extension<crate::domain::models::User>>,
    caller: Option<axum::Extension<crate::domain::permissions::Actor>>,
    params: RetireCommon<'_>,
) -> Result<axum::response::Response, AppError> {
    let RetireCommon {
        api_type,
        producername,
        content,
        stability,
        dry_run,
        trunk,
        tag,
    } = params;

    let contradiction = if content.is_some() {
        Some("a specification body — a retire publishes nothing")
    } else if stability.is_some() {
        Some("`stability` — a retire publishes nothing to declare a stability for")
    } else if trunk {
        Some("`trunk` — a retire closes the family's trunk pins on every stream at once")
    } else if tag.is_some() {
        Some("`tag` — a retire is a fact about the Producer, not about one sanshain-branch")
    } else {
        None
    };
    if let Some(what) = contradiction {
        return Err(AppError::BadRequest(format!(
            "`retired: true` cannot be combined with {what}"
        )));
    }

    let actor = caller.map(|axum::Extension(a)| a);
    let res = services::retire_protocol_family(
        &state.repo,
        producername,
        api_type,
        actor.as_ref(),
        dry_run,
    )
    .await?;

    if !dry_run {
        let _ = state.spec_updated_tx.send(());
        record_audit_log(
            &state.repo,
            user,
            NewAuditLog {
                action: "RETIRE_PROTOCOL",
                details: &format!(
                    "Retired {} for producer '{}'",
                    api_type.as_str(),
                    producername
                ),
                service: Some(producername),
                action_type: Some("WRITE"),
                ..Default::default()
            },
        )
        .await?;
    }

    Ok((StatusCode::ACCEPTED, Json(res)).into_response())
}

pub async fn provide(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    caller: Option<axum::Extension<crate::domain::permissions::Actor>>,
    Json(payload): Json<ProvideRequest>,
) -> Result<impl IntoResponse, AppError> {
    provide_common(
        &state,
        user,
        caller,
        ProvideCommon {
            api_type: ApiType::OpenApi,
            producername: &payload.producername,
            content: payload.openapi_yaml.as_deref(),
            content_field: "openapi_yaml",
            stability: payload.stability,
            dry_run: payload.dry_run,
            trunk: payload.trunk,
            tag: payload.tag.as_deref(),
            retired: payload.retired,
        },
    )
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvideAsyncApiRequest {
    pub producername: String,
    /// See `ProvideRequest::openapi_yaml`.
    pub asyncapi_yaml: Option<String>,
    /// See `ProvideRequest::stability`.
    pub stability: Option<Stability>,
    #[serde(default)]
    pub dry_run: bool,
    /// See `ProvideRequest::trunk`.
    #[serde(default)]
    pub trunk: bool,
    /// See `ProvideRequest::tag`.
    #[serde(default)]
    pub tag: Option<String>,
    /// See `ProvideRequest::retired`.
    #[serde(default)]
    pub retired: bool,
}

pub async fn provide_asyncapi(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    caller: Option<axum::Extension<crate::domain::permissions::Actor>>,
    Json(payload): Json<ProvideAsyncApiRequest>,
) -> Result<impl IntoResponse, AppError> {
    provide_common(
        &state,
        user,
        caller,
        ProvideCommon {
            api_type: ApiType::AsyncApi,
            producername: &payload.producername,
            content: payload.asyncapi_yaml.as_deref(),
            content_field: "asyncapi_yaml",
            stability: payload.stability,
            dry_run: payload.dry_run,
            trunk: payload.trunk,
            tag: payload.tag.as_deref(),
            retired: payload.retired,
        },
    )
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvideProtoRequest {
    pub producername: String,
    /// See `ProvideRequest::openapi_yaml`.
    pub proto_content: Option<String>,
    /// See `ProvideRequest::stability`.
    pub stability: Option<Stability>,
    #[serde(default)]
    pub dry_run: bool,
    /// See `ProvideRequest::trunk`.
    #[serde(default)]
    pub trunk: bool,
    /// See `ProvideRequest::tag`.
    #[serde(default)]
    pub tag: Option<String>,
    /// See `ProvideRequest::retired`.
    #[serde(default)]
    pub retired: bool,
}

pub async fn provide_proto(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    caller: Option<axum::Extension<crate::domain::permissions::Actor>>,
    Json(payload): Json<ProvideProtoRequest>,
) -> Result<impl IntoResponse, AppError> {
    provide_common(
        &state,
        user,
        caller,
        ProvideCommon {
            api_type: ApiType::Proto,
            producername: &payload.producername,
            content: payload.proto_content.as_deref(),
            content_field: "proto_content",
            stability: payload.stability,
            dry_run: payload.dry_run,
            trunk: payload.trunk,
            tag: payload.tag.as_deref(),
            retired: payload.retired,
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
    /// ADR-0004: record this pin in the append-only trunk store too.
    #[serde(default)]
    pub trunk: bool,
    /// ADR-0005: the pin updates this sanshain-branch instead of trunk.
    #[serde(default)]
    pub tag: Option<String>,
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
        trunk: query.trunk,
        tag: query.tag.as_deref(),
    };
    let res = if query.dry_run {
        services::require_endpoint_dry_run(&state.repo, params).await?
    } else {
        let res = services::require_endpoint(&state.repo, params).await?;
        let version_str = query.version.to_string();
        let stream = if query.trunk {
            Some("trunk")
        } else {
            query.tag.as_deref()
        };
        let branch_id = audit_branch_id(state, query.tag.as_deref()).await;
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
                stream,
                branch_id,
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
    /// See `RequireQuery::trunk`.
    #[serde(default)]
    pub trunk: bool,
    /// See `RequireQuery::tag`.
    #[serde(default)]
    pub tag: Option<String>,
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
        trunk: payload.trunk,
        tag: payload.tag.as_deref(),
    };
    let res = if payload.dry_run {
        services::require_bundle_dry_run(&state.repo, params).await?
    } else {
        let res = services::require_bundle(&state.repo, params).await?;
        let version_str = payload.version.to_string();
        let stream = if payload.trunk {
            Some("trunk")
        } else {
            payload.tag.as_deref()
        };
        let branch_id = audit_branch_id(&state, payload.tag.as_deref()).await;
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
                stream,
                branch_id,
            },
        )
        .await;
        res
    };

    Ok(require_response(&headers, res))
}

#[derive(Deserialize)]
pub struct ReportQuery {
    /// `dev` (default), `main`, or `<branch>[@rfc3339]` (ADR-0005).
    pub scope: Option<String>,
}

pub async fn report(
    State(state): State<AppState>,
    Query(query): Query<ReportQuery>,
) -> Result<impl IntoResponse, AppError> {
    // Not audited: this JSON report is fetched to render the graph view in the
    // UI, so auditing it would turn every graph view into an audit entry.
    // Explicit report exports (markdown/isolation) are still audited below.
    let scope = services::ReportScope::parse(query.scope.as_deref())?;
    let res = services::generate_scoped_report(&state.repo, scope).await?;
    Ok(Json(res))
}

pub async fn report_markdown(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Query(query): Query<ReportQuery>,
) -> Result<impl IntoResponse, AppError> {
    let scope = services::ReportScope::parse(query.scope.as_deref())?;
    let res = services::generate_scoped_report(&state.repo, scope).await?;
    let _ = record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "REPORT",
            details: "Generated Markdown dependency report",
            action_type: Some("READ"),
            ..Default::default()
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
    Query(query): Query<ReportQuery>,
) -> Result<impl IntoResponse, AppError> {
    let scope = services::ReportScope::parse(query.scope.as_deref())?;
    let res = services::generate_scoped_report(&state.repo, scope).await?;
    let _ = record_audit_log(
        &state.repo,
        user,
        NewAuditLog {
            action: "REPORT",
            details: "Generated Isolation report",
            action_type: Some("READ"),
            ..Default::default()
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
    /// Exact stream match (ADR-0005): `trunk` or a tag name.
    pub stream: Option<String>,
}

pub async fn audit_timeline(
    State(state): State<AppState>,
    Query(query): Query<AuditTimelineQuery>,
) -> Result<impl IntoResponse, AppError> {
    let limit = query.limit.unwrap_or(50);
    // Resolve a branch name to its identity so a renamed branch answers under
    // its current name; 'trunk' and unknown names keep matching the label.
    let branch_id = match query.stream.as_deref() {
        Some(name) if name != "trunk" => state
            .repo
            .find_branch(name)
            .await
            .ok()
            .flatten()
            .map(|b| b.id),
        _ => None,
    };
    let filter = crate::domain::models::AuditLogFilter {
        from_date: query.from_date,
        to_date: query.to_date,
        action_type: query.action_type,
        service_wildcard: query.service,
        version_wildcard: query.version,
        stream: query.stream,
        branch_id,
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

pub async fn validate(Json(payload): Json<ValidateRequest>) -> Result<impl IntoResponse, AppError> {
    Ok(Json(
        services::validate_spec_async(payload.api_type, payload.content).await?,
    ))
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
