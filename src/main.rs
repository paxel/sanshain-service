use axum::{
    body::Body,
    extract::{Query, State},
    http::StatusCode,
    middleware,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePoolOptions;
use std::net::SocketAddr;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub mod openapi;
pub mod domain;
pub mod application;
pub mod infrastructure;

use application::services::{self, AppError};
use infrastructure::sqlite_repository::SqliteSpecRepository;

#[derive(Clone)]
pub struct AppState {
    pub repo: SqliteSpecRepository,
}

#[tokio::main]
pub async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "sanshain_service=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let db_connection_str = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite:sanshain.db?mode=rwc".into());

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&db_connection_str)
        .await
        .expect("can't connect to database");

    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.expect("can't run migrations");

    // Ensure initial admin user exists
    services::ensure_initial_admin(&repo).await.expect("can't create initial admin");

    let state = AppState { repo };

    let app = create_app(state);

    let bind_address = std::env::var("BIND_ADDRESS").unwrap_or_else(|_| "0.0.0.0:3000".into());
    let addr: SocketAddr = bind_address.parse().expect("invalid BIND_ADDRESS");
    tracing::debug!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

use tower_http::services::ServeDir;
use axum::response::Redirect;

/// Middleware: admin endpoints always require a valid admin session token.
async fn admin_auth(
    State(state): State<AppState>,
    req: axum::http::Request<Body>,
    next: middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
    let token = extract_bearer_token(&req).ok_or(StatusCode::UNAUTHORIZED)?;
    let (user, _session) = services::validate_session(&state.repo, &token)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(next.run(req).await)
}

/// Middleware: non-admin API endpoints require dev_mode=true OR a valid session token.
async fn api_auth(
    State(state): State<AppState>,
    req: axum::http::Request<Body>,
    next: middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
    let dev_mode = services::get_dev_mode(&state.repo)
        .await
        .unwrap_or(false);
    if dev_mode {
        return Ok(next.run(req).await);
    }

    // Dev mode off: require valid session
    let token = extract_bearer_token(&req).ok_or(StatusCode::FORBIDDEN)?;
    let _valid = services::validate_session(&state.repo, &token)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    Ok(next.run(req).await)
}

fn extract_bearer_token(req: &axum::http::Request<Body>) -> Option<String> {
    let header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    header.strip_prefix("Bearer ").map(|s| s.to_string())
}

pub fn create_app(state: AppState) -> Router {
    let admin_routes = Router::new()
        .route("/protected-branches", get(list_protected_branches).post(add_protected_branch))
        .route("/protected-branches/:pattern", axum::routing::delete(delete_protected_branch))
        .route("/services", get(admin_list_services))
        .route("/services/:name", axum::routing::delete(admin_delete_service))
        .route("/services/:name/branches", get(admin_list_branches))
        .route("/services/:name/branches/:branch", axum::routing::delete(admin_delete_branch))
        .route("/clients", get(admin_list_clients))
        .route("/clients/:name", axum::routing::delete(admin_delete_client))
        .route("/settings/dev-mode", get(get_dev_mode).post(set_dev_mode))
        .route_layer(middleware::from_fn_with_state(state.clone(), admin_auth));

    let api_routes = Router::new()
        .route("/provide", post(provide))
        .route("/require", get(require))
        .route("/report", get(report))
        .route("/report/markdown", get(report_markdown))
        .route_layer(middleware::from_fn_with_state(state.clone(), api_auth));

    Router::new()
        .route("/", get(|| async { Redirect::permanent("/index.html") }))
        .route("/health", get(health))
        .route("/auth/login", post(auth_login))
        .route("/auth/logout", post(auth_logout))
        .route("/auth/me", get(auth_me))
        .route("/auth/change-password", post(auth_change_password))
        .merge(api_routes)
        .nest("/admin", admin_routes)
        .fallback_service(ServeDir::new("static"))
        .with_state(state)
}

#[derive(Deserialize)]
struct ProvidePayload {
    servicename: String,
    branch: String,
    openapi_yaml: String,
}

#[derive(Deserialize)]
struct RequireParams {
    clientname: String,
    servicename: String,
    branch: String,
    path: String,
    method: String,
    timeout: Option<u64>,
}

#[derive(Deserialize)]
struct ReportParams {
    branch: String,
}

async fn health() -> StatusCode {
    StatusCode::OK
}

fn app_error_to_status(e: AppError) -> StatusCode {
    match e {
        AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
        AppError::Conflict => StatusCode::CONFLICT,
        AppError::NotFound => StatusCode::NOT_FOUND,
        AppError::Unauthorized => StatusCode::UNAUTHORIZED,
        AppError::Forbidden => StatusCode::FORBIDDEN,
        AppError::Internal(msg) => {
            tracing::error!("Internal error: {}", msg);
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

async fn provide(
    State(state): State<AppState>,
    Json(payload): Json<ProvidePayload>,
) -> Result<StatusCode, StatusCode> {
    services::provide_spec(&state.repo, &payload.servicename, &payload.branch, &payload.openapi_yaml)
        .await
        .map(|_| StatusCode::ACCEPTED)
        .map_err(app_error_to_status)
}

async fn require(
    State(state): State<AppState>,
    Query(params): Query<RequireParams>,
) -> Result<String, StatusCode> {
    services::require_endpoint(
        &state.repo,
        &params.clientname,
        &params.servicename,
        &params.branch,
        &params.path,
        &params.method,
        params.timeout,
    )
    .await
    .map_err(app_error_to_status)
}

async fn report(
    State(state): State<AppState>,
    Query(params): Query<ReportParams>,
) -> Result<Json<domain::models::DependencyReport>, StatusCode> {
    services::generate_report(&state.repo, &params.branch)
        .await
        .map(Json)
        .map_err(app_error_to_status)
}

async fn report_markdown(
    State(state): State<AppState>,
    Query(params): Query<ReportParams>,
) -> Result<(axum::http::HeaderMap, String), StatusCode> {
    let report = services::generate_report(&state.repo, &params.branch)
        .await
        .map_err(app_error_to_status)?;
    let md = services::render_report_markdown(&report);
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        "text/markdown; charset=utf-8".parse().unwrap(),
    );
    Ok((headers, md))
}

// --- Auth endpoints ---

#[derive(Deserialize)]
struct LoginPayload {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct LoginResponse {
    token: String,
    expires_at: String,
}

#[derive(Serialize)]
struct MeResponse {
    username: String,
    is_admin: bool,
}

async fn auth_login(
    State(state): State<AppState>,
    Json(payload): Json<LoginPayload>,
) -> Result<Json<LoginResponse>, StatusCode> {
    let session = services::login(&state.repo, &payload.username, &payload.password)
        .await
        .map_err(app_error_to_status)?;
    Ok(Json(LoginResponse {
        token: session.token,
        expires_at: session.expires_at,
    }))
}

async fn auth_logout(
    State(state): State<AppState>,
    req: axum::http::Request<Body>,
) -> Result<StatusCode, StatusCode> {
    let token = extract_bearer_token(&req).ok_or(StatusCode::UNAUTHORIZED)?;
    services::logout(&state.repo, &token)
        .await
        .map_err(app_error_to_status)?;
    Ok(StatusCode::OK)
}

async fn auth_me(
    State(state): State<AppState>,
    req: axum::http::Request<Body>,
) -> Result<Json<MeResponse>, StatusCode> {
    let token = extract_bearer_token(&req).ok_or(StatusCode::UNAUTHORIZED)?;
    let (user, _session) = services::validate_session(&state.repo, &token)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    Ok(Json(MeResponse {
        username: user.username,
        is_admin: user.is_admin,
    }))
}

#[derive(Deserialize)]
struct ChangePasswordPayload {
    old_password: String,
    new_password: String,
}

async fn auth_change_password(
    State(state): State<AppState>,
    req: axum::http::Request<Body>,
) -> Result<StatusCode, StatusCode> {
    let token = extract_bearer_token(&req).ok_or(StatusCode::UNAUTHORIZED)?;
    let (user, _session) = services::validate_session(&state.repo, &token)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // Need to parse body manually since we already consumed headers
    let body_bytes = axum::body::to_bytes(req.into_body(), 1024 * 1024)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let payload: ChangePasswordPayload = serde_json::from_slice(&body_bytes)
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    services::change_password(&state.repo, &user, &payload.old_password, &payload.new_password)
        .await
        .map_err(app_error_to_status)?;
    Ok(StatusCode::OK)
}

// --- Admin endpoints ---

async fn list_protected_branches(
    State(state): State<AppState>,
) -> Result<Json<Vec<String>>, StatusCode> {
    services::list_protected_branches(&state.repo)
        .await
        .map(Json)
        .map_err(app_error_to_status)
}

#[derive(Deserialize)]
struct ProtectedBranchPayload {
    pattern: String,
}

async fn add_protected_branch(
    State(state): State<AppState>,
    Json(payload): Json<ProtectedBranchPayload>,
) -> Result<StatusCode, StatusCode> {
    services::add_protected_branch(&state.repo, &payload.pattern)
        .await
        .map(|_| StatusCode::CREATED)
        .map_err(app_error_to_status)
}

async fn delete_protected_branch(
    State(state): State<AppState>,
    axum::extract::Path(pattern): axum::extract::Path<String>,
) -> Result<StatusCode, StatusCode> {
    let removed = services::remove_protected_branch(&state.repo, &pattern)
        .await
        .map_err(app_error_to_status)?;
    if removed {
        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn admin_list_services(
    State(state): State<AppState>,
) -> Result<Json<Vec<String>>, StatusCode> {
    services::list_services(&state.repo)
        .await
        .map(Json)
        .map_err(app_error_to_status)
}

async fn admin_delete_service(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<StatusCode, StatusCode> {
    let removed = services::delete_service(&state.repo, &name)
        .await
        .map_err(app_error_to_status)?;
    if removed {
        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn admin_list_branches(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<Json<Vec<String>>, StatusCode> {
    services::list_branches(&state.repo, &name)
        .await
        .map(Json)
        .map_err(app_error_to_status)
}

async fn admin_delete_branch(
    State(state): State<AppState>,
    axum::extract::Path((name, branch)): axum::extract::Path<(String, String)>,
) -> Result<StatusCode, StatusCode> {
    let removed = services::delete_branch(&state.repo, &name, &branch)
        .await
        .map_err(app_error_to_status)?;
    if removed {
        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn admin_list_clients(
    State(state): State<AppState>,
) -> Result<Json<Vec<String>>, StatusCode> {
    services::list_clients(&state.repo)
        .await
        .map(Json)
        .map_err(app_error_to_status)
}

async fn admin_delete_client(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<StatusCode, StatusCode> {
    let removed = services::delete_client(&state.repo, &name)
        .await
        .map_err(app_error_to_status)?;
    if removed {
        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

// --- Dev mode admin endpoints ---

#[derive(Serialize)]
struct DevModeResponse {
    dev_mode: bool,
}

async fn get_dev_mode(
    State(state): State<AppState>,
) -> Result<Json<DevModeResponse>, StatusCode> {
    let enabled = services::get_dev_mode(&state.repo)
        .await
        .map_err(app_error_to_status)?;
    Ok(Json(DevModeResponse { dev_mode: enabled }))
}

#[derive(Deserialize)]
struct DevModePayload {
    enabled: bool,
}

async fn set_dev_mode(
    State(state): State<AppState>,
    Json(payload): Json<DevModePayload>,
) -> Result<StatusCode, StatusCode> {
    services::set_dev_mode(&state.repo, payload.enabled)
        .await
        .map_err(app_error_to_status)?;
    if payload.enabled {
        tracing::warn!("Dev mode ENABLED — all API endpoints are now open without authentication.");
    } else {
        tracing::info!("Dev mode DISABLED — API endpoints require authentication.");
    }
    Ok(StatusCode::OK)
}
