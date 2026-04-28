use crate::AppState;
use crate::application::services::{self, AppError};
use crate::domain::models::ApiType;
use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
};
use sha2::{Digest, Sha256};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct ProvideRequest {
    pub servicename: String,
    pub branch: String,
    pub openapi_yaml: String,
    pub base_version: Option<i32>,
    #[serde(default)]
    pub dry_run: bool,
}

pub async fn provide(
    State(state): State<AppState>,
    Json(payload): Json<ProvideRequest>,
) -> Result<impl IntoResponse, AppError> {
    let res = if payload.dry_run {
        services::provide_spec_dry_run(
            &state.repo,
            &payload.servicename,
            &payload.branch,
            ApiType::OpenApi,
            &payload.openapi_yaml,
        )
        .await?
    } else {
        services::provide_spec(
            &state.repo,
            &payload.servicename,
            &payload.branch,
            ApiType::OpenApi,
            &payload.openapi_yaml,
            payload.base_version,
        )
        .await?
    };

    if !payload.dry_run {
        let _ = state.spec_updated_tx.send(());
    }

    Ok((StatusCode::ACCEPTED, Json(res)))
}

#[derive(Deserialize)]
pub struct ProvideAsyncApiRequest {
    pub servicename: String,
    pub branch: String,
    pub asyncapi_yaml: String,
    pub base_version: Option<i32>,
}

pub async fn provide_asyncapi(
    State(state): State<AppState>,
    Json(payload): Json<ProvideAsyncApiRequest>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::provide_spec(
        &state.repo,
        &payload.servicename,
        &payload.branch,
        ApiType::AsyncApi,
        &payload.asyncapi_yaml,
        payload.base_version,
    )
    .await?;
    let _ = state.spec_updated_tx.send(());
    Ok((StatusCode::ACCEPTED, Json(res)))
}

#[derive(Deserialize)]
pub struct ProvideProtoRequest {
    pub servicename: String,
    pub branch: String,
    pub proto_content: String,
    pub base_version: Option<i32>,
}

pub async fn provide_proto(
    State(state): State<AppState>,
    Json(payload): Json<ProvideProtoRequest>,
) -> Result<impl IntoResponse, AppError> {
    let res = services::provide_spec(
        &state.repo,
        &payload.servicename,
        &payload.branch,
        ApiType::Proto,
        &payload.proto_content,
        payload.base_version,
    )
    .await?;
    let _ = state.spec_updated_tx.send(());
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

    let etag = format!("\"{}\"", hex::encode(Sha256::digest(res.as_bytes())));
    if let Some(if_none_match) = headers.get("if-none-match") {
        if if_none_match == etag.as_str() {
            return Ok((StatusCode::NOT_MODIFIED, HeaderMap::new()).into_response());
        }
    }

    let mut headers = HeaderMap::new();
    headers.insert("ETag", HeaderValue::from_str(&etag).unwrap());
    Ok((headers, res).into_response())
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

    let etag = format!("\"{}\"", hex::encode(Sha256::digest(res.as_bytes())));
    if let Some(if_none_match) = headers.get("if-none-match") {
        if if_none_match == etag.as_str() {
            return Ok((StatusCode::NOT_MODIFIED, HeaderMap::new()).into_response());
        }
    }

    let mut headers = HeaderMap::new();
    headers.insert("ETag", HeaderValue::from_str(&etag).unwrap());
    Ok((headers, res).into_response())
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

    let etag = format!("\"{}\"", hex::encode(Sha256::digest(res.as_bytes())));
    if let Some(if_none_match) = headers.get("if-none-match") {
        if if_none_match == etag.as_str() {
            return Ok((StatusCode::NOT_MODIFIED, HeaderMap::new()).into_response());
        }
    }

    let mut headers = HeaderMap::new();
    headers.insert("ETag", HeaderValue::from_str(&etag).unwrap());
    Ok((headers, res).into_response())
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

    let etag = format!("\"{}\"", hex::encode(Sha256::digest(res.as_bytes())));
    if let Some(if_none_match) = headers.get("if-none-match") {
        if if_none_match == etag.as_str() {
            return Ok((StatusCode::NOT_MODIFIED, HeaderMap::new()).into_response());
        }
    }

    let mut headers = HeaderMap::new();
    headers.insert("ETag", HeaderValue::from_str(&etag).unwrap());
    Ok((headers, res).into_response())
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
