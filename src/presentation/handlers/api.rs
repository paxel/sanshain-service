use crate::AppState;
use crate::application::services::{self, AppError};
use crate::domain::models::{ApiType, redact_username};
use axum::{
    Json,
    extract::{Query, State, ws::{WebSocketUpgrade, WebSocket, Message}},
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
    action: &str,
    details: &str,
) -> Result<(), AppError> {
    let actor = if let Some(axum::Extension(u)) = user {
        redact_username(&u.username)
    } else {
        "DevMode/Anonymous".to_string()
    };
    repo.insert_audit_log(&actor, action, details)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))
}

#[derive(Deserialize)]
pub struct ProvideRequest {
    pub servicename: String,
    pub branch: String,
    pub openapi_yaml: String,
    pub base_version: Option<i32>,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub force: bool,
}

pub async fn provide(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<ProvideRequest>,
) -> Result<impl IntoResponse, AppError> {
    let actor = if let Some(axum::Extension(ref u)) = user {
        redact_username(&u.username)
    } else {
        "DevMode/Anonymous".to_string()
    };

    let res = if payload.dry_run {
        services::provide_spec_dry_run(
            &state.repo,
            &payload.servicename,
            &payload.branch,
            ApiType::OpenApi,
            &payload.openapi_yaml,
            payload.force,
        )
        .await?
    } else {
        services::provide_spec_with_actor(
            &state.repo,
            &payload.servicename,
            &payload.branch,
            ApiType::OpenApi,
            &payload.openapi_yaml,
            payload.base_version,
            payload.force,
            Some(&actor),
        )
        .await?
    };

    if !payload.dry_run {
        let _ = state.spec_updated_tx.send(());
        record_audit_log(
            &state.repo,
            user,
            "PROVIDE_SPEC",
            &format!(
                "Uploaded OpenApi spec for service '{}' on branch '{}' (version {}, content hash {})",
                payload.servicename, payload.branch, res.version, res.content_hash
            ),
        )
        .await?;
    }

    Ok((StatusCode::ACCEPTED, Json(res)))
}

#[derive(Deserialize)]
pub struct ProvideAsyncApiRequest {
    pub servicename: String,
    pub branch: String,
    pub asyncapi_yaml: String,
    pub base_version: Option<i32>,
    #[serde(default)]
    pub force: bool,
}

pub async fn provide_asyncapi(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<ProvideAsyncApiRequest>,
) -> Result<impl IntoResponse, AppError> {
    let actor = if let Some(axum::Extension(ref u)) = user {
        redact_username(&u.username)
    } else {
        "DevMode/Anonymous".to_string()
    };

    let res = services::provide_spec_with_actor(
        &state.repo,
        &payload.servicename,
        &payload.branch,
        ApiType::AsyncApi,
        &payload.asyncapi_yaml,
        payload.base_version,
        payload.force,
        Some(&actor),
    )
    .await?;
    let _ = state.spec_updated_tx.send(());
    record_audit_log(
        &state.repo,
        user,
        "PROVIDE_SPEC",
        &format!(
            "Uploaded AsyncApi spec for service '{}' on branch '{}' (version {}, content hash {})",
            payload.servicename, payload.branch, res.version, res.content_hash
        ),
    )
    .await?;
    Ok((StatusCode::ACCEPTED, Json(res)))
}

#[derive(Deserialize)]
pub struct ProvideProtoRequest {
    pub servicename: String,
    pub branch: String,
    pub proto_content: String,
    pub base_version: Option<i32>,
    #[serde(default)]
    pub force: bool,
}

pub async fn provide_proto(
    State(state): State<AppState>,
    user: Option<axum::Extension<crate::domain::models::User>>,
    Json(payload): Json<ProvideProtoRequest>,
) -> Result<impl IntoResponse, AppError> {
    let actor = if let Some(axum::Extension(ref u)) = user {
        redact_username(&u.username)
    } else {
        "DevMode/Anonymous".to_string()
    };

    let res = services::provide_spec_with_actor(
        &state.repo,
        &payload.servicename,
        &payload.branch,
        ApiType::Proto,
        &payload.proto_content,
        payload.base_version,
        payload.force,
        Some(&actor),
    )
    .await?;
    let _ = state.spec_updated_tx.send(());
    record_audit_log(
        &state.repo,
        user,
        "PROVIDE_SPEC",
        &format!(
            "Uploaded Proto spec for service '{}' on branch '{}' (version {}, content hash {})",
            payload.servicename, payload.branch, res.version, res.content_hash
        ),
    )
    .await?;
    Ok((StatusCode::ACCEPTED, Json(res)))
}

#[derive(Deserialize)]
pub struct RequireQuery {
    pub clientname: String,
    pub servicename: String,
    pub branch: String,
    pub path: String,
    pub method: String,
    pub timeout: Option<u64>,
    #[serde(default)]
    pub dry_run: bool,
}

pub async fn require(
    State(state): State<AppState>,
    Query(query): Query<RequireQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let params = services::RequireEndpointParams {
        clientname: &query.clientname,
        servicename: &query.servicename,
        branch: &query.branch,
        api_type: ApiType::OpenApi,
        path: &query.path,
        method: &query.method,
        timeout_secs: query.timeout,
    };
    let res = if query.dry_run {
        services::require_endpoint_dry_run(
            &state.repo,
            Some(state.spec_updated_tx.subscribe()),
            params,
        )
        .await?
    } else {
        services::require_endpoint(&state.repo, Some(state.spec_updated_tx.subscribe()), params)
            .await?
    };

    let etag = format!("\"{}\"", hex::encode(Sha256::digest(res.yaml.as_bytes())));
    if headers.get("if-none-match") == Some(&HeaderValue::from_str(&etag).unwrap()) {
        return Ok((StatusCode::NOT_MODIFIED, HeaderMap::new()).into_response());
    }

    let mut headers = HeaderMap::new();
    headers.insert("ETag", HeaderValue::from_str(&etag).unwrap());
    if res.deprecated {
        headers.insert("X-Sanshain-Deprecated", HeaderValue::from_static("true"));
    }
    Ok((headers, res.yaml).into_response())
}

pub async fn require_asyncapi(
    State(state): State<AppState>,
    Query(query): Query<RequireQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let res = services::require_endpoint(
        &state.repo,
        Some(state.spec_updated_tx.subscribe()),
        services::RequireEndpointParams {
            clientname: &query.clientname,
            servicename: &query.servicename,
            branch: &query.branch,
            api_type: ApiType::AsyncApi,
            path: &query.path,
            method: &query.method,
            timeout_secs: query.timeout,
        },
    )
    .await?;

    let etag = format!("\"{}\"", hex::encode(Sha256::digest(res.yaml.as_bytes())));
    if headers.get("if-none-match") == Some(&HeaderValue::from_str(&etag).unwrap()) {
        return Ok((StatusCode::NOT_MODIFIED, HeaderMap::new()).into_response());
    }

    let mut headers = HeaderMap::new();
    headers.insert("ETag", HeaderValue::from_str(&etag).unwrap());
    if res.deprecated {
        headers.insert("X-Sanshain-Deprecated", HeaderValue::from_static("true"));
    }
    Ok((headers, res.yaml).into_response())
}

pub async fn require_proto(
    State(state): State<AppState>,
    Query(query): Query<RequireQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let res = services::require_endpoint(
        &state.repo,
        Some(state.spec_updated_tx.subscribe()),
        services::RequireEndpointParams {
            clientname: &query.clientname,
            servicename: &query.servicename,
            branch: &query.branch,
            api_type: ApiType::Proto,
            path: &query.path,
            method: &query.method,
            timeout_secs: query.timeout,
        },
    )
    .await?;

    let etag = format!("\"{}\"", hex::encode(Sha256::digest(res.yaml.as_bytes())));
    if headers.get("if-none-match") == Some(&HeaderValue::from_str(&etag).unwrap()) {
        return Ok((StatusCode::NOT_MODIFIED, HeaderMap::new()).into_response());
    }

    let mut headers = HeaderMap::new();
    headers.insert("ETag", HeaderValue::from_str(&etag).unwrap());
    if res.deprecated {
        headers.insert("X-Sanshain-Deprecated", HeaderValue::from_static("true"));
    }
    Ok((headers, res.yaml).into_response())
}

#[derive(Deserialize)]
pub struct BundleEndpoint {
    pub path: String,
    pub method: String,
}

#[derive(Deserialize)]
pub struct RequireBundleRequest {
    pub clientname: String,
    pub servicename: String,
    pub branch: String,
    pub api_type: Option<ApiType>,
    pub endpoints: Vec<BundleEndpoint>,
    pub timeout: Option<u64>,
}

pub async fn require_bundle(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<RequireBundleRequest>,
) -> Result<impl IntoResponse, AppError> {
    let endpoints: Vec<(String, String)> = payload
        .endpoints
        .into_iter()
        .map(|e| (e.path, e.method))
        .collect();

    let res = services::require_bundle(
        &state.repo,
        Some(state.spec_updated_tx.subscribe()),
        services::RequireBundleParams {
            clientname: &payload.clientname,
            servicename: &payload.servicename,
            branch: &payload.branch,
            api_type: payload.api_type.unwrap_or(ApiType::OpenApi),
            endpoints: &endpoints,
            timeout_secs: payload.timeout,
        },
    )
    .await?;

    let etag = format!("\"{}\"", hex::encode(Sha256::digest(res.yaml.as_bytes())));
    if headers.get("if-none-match") == Some(&HeaderValue::from_str(&etag).unwrap()) {
        return Ok((StatusCode::NOT_MODIFIED, HeaderMap::new()).into_response());
    }

    let mut headers = HeaderMap::new();
    headers.insert("ETag", HeaderValue::from_str(&etag).unwrap());
    if res.deprecated {
        headers.insert("X-Sanshain-Deprecated", HeaderValue::from_static("true"));
    }
    Ok((headers, res.yaml).into_response())
}

#[derive(Deserialize)]
pub struct ReportQuery {
    pub branch: String,
}

pub async fn report(
    State(state): State<AppState>,
    Query(query): Query<ReportQuery>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::generate_report(&state.repo, &query.branch).await?;
    Ok(Json(res))
}

pub async fn report_markdown(
    State(state): State<AppState>,
    Query(query): Query<ReportQuery>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::generate_report(&state.repo, &query.branch).await?;
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
    Query(query): Query<ReportQuery>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::generate_report(&state.repo, &query.branch).await?;
    Ok(services::render_isolation_report(&res))
}

#[derive(Deserialize)]
pub struct MergedReportQuery {
    pub branch: String,
    pub target: String,
}

pub async fn report_merged(
    State(state): State<AppState>,
    Query(query): Query<MergedReportQuery>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::generate_merged_report(&state.repo, &query.branch, &query.target).await?;
    Ok(Json(res))
}

pub async fn list_protected_branches_public(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::list_protected_branches(&state.repo).await?;
    Ok(Json(res))
}

pub async fn list_branches_metadata(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::list_branches_with_metadata(&state.repo).await?;
    Ok(Json(res))
}

#[derive(Deserialize)]
pub struct VersionsQuery {
    pub service: String,
    pub branch: String,
    pub api_type: Option<ApiType>,
    pub path: String,
    pub method: String,
}

pub async fn endpoint_versions(
    State(state): State<AppState>,
    Query(query): Query<VersionsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::get_endpoint_version_history(
        &state.repo,
        &query.service,
        &query.branch,
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
}

pub async fn audit_timeline(
    State(state): State<AppState>,
    Query(query): Query<AuditTimelineQuery>,
) -> Result<impl IntoResponse, AppError> {
    let limit = query.limit.unwrap_or(50);
    let res = services::get_audit_timeline(&state.repo, limit).await?;
    Ok(Json(res))
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

pub async fn ws_updates(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
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
