use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
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

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("can't run migrations");

    let state = AppState {
        repo: SqliteSpecRepository::new(pool),
    };

    let app = create_app(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    tracing::debug!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

use tower_http::services::ServeDir;
use axum::response::Redirect;

pub fn create_app(state: AppState) -> Router {
    Router::new()
        .route("/", get(|| async { Redirect::permanent("/index.html") }))
        .route("/provide", post(provide))
        .route("/require", get(require))
        .route("/report", get(report))
        .route("/report/markdown", get(report_markdown))
        .route("/admin/protected-branches", get(list_protected_branches).post(add_protected_branch))
        .route("/admin/protected-branches/:pattern", axum::routing::delete(delete_protected_branch))
        .route("/admin/services", get(admin_list_services))
        .route("/admin/services/:name", axum::routing::delete(admin_delete_service))
        .route("/admin/services/:name/branches", get(admin_list_branches))
        .route("/admin/services/:name/branches/:branch", axum::routing::delete(admin_delete_branch))
        .route("/admin/clients", get(admin_list_clients))
        .route("/admin/clients/:name", axum::routing::delete(admin_delete_client))
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

fn app_error_to_status(e: AppError) -> StatusCode {
    match e {
        AppError::BadRequest(msg) => {
            tracing::error!("Bad request: {}", msg);
            StatusCode::BAD_REQUEST
        }
        AppError::Conflict => StatusCode::CONFLICT,
        AppError::NotFound => StatusCode::NOT_FOUND,
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
    tracing::info!("Providing spec for {}/{}", payload.servicename, payload.branch);

    services::provide_spec(&state.repo, &payload.servicename, &payload.branch, &payload.openapi_yaml)
        .await
        .map_err(app_error_to_status)?;

    Ok(StatusCode::ACCEPTED)
}

async fn require(
    State(state): State<AppState>,
    Query(params): Query<RequireParams>,
) -> Result<String, StatusCode> {
    tracing::info!(
        "Requiring {} {} from {}/{} for client {}",
        params.method,
        params.path,
        params.servicename,
        params.branch,
        params.clientname
    );

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
    tracing::info!("Generating report for branch {}", params.branch);

    let report = services::generate_report(&state.repo, &params.branch)
        .await
        .map_err(app_error_to_status)?;

    Ok(Json(report))
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
    headers.insert("content-type", "text/markdown; charset=utf-8".parse().unwrap());
    Ok((headers, md))
}

async fn list_protected_branches(
    State(state): State<AppState>,
) -> Result<Json<Vec<String>>, StatusCode> {
    let branches = services::list_protected_branches(&state.repo)
        .await
        .map_err(app_error_to_status)?;
    Ok(Json(branches))
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
        .map_err(app_error_to_status)?;
    Ok(StatusCode::CREATED)
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
    let names = services::list_services(&state.repo)
        .await
        .map_err(app_error_to_status)?;
    Ok(Json(names))
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
    let branches = services::list_branches(&state.repo, &name)
        .await
        .map_err(app_error_to_status)?;
    Ok(Json(branches))
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
    let names = services::list_clients(&state.repo)
        .await
        .map_err(app_error_to_status)?;
    Ok(Json(names))
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
