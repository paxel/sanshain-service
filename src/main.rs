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
