use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;
use std::net::SocketAddr;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub mod openapi;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
}

#[tokio::main]
pub async fn main() {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "sanshain_service=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Database setup
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

    let state = AppState { db: pool };

    // build our application with a route
    let app = create_app(state);

    // run our app with hyper, listening globally on port 3000
    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    tracing::debug!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

use std::collections::HashMap;

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

async fn provide(
    State(state): State<AppState>,
    Json(payload): Json<ProvidePayload>,
) -> Result<StatusCode, StatusCode> {
    tracing::info!("Providing spec for {}/{}", payload.servicename, payload.branch);

    let endpoints = openapi::split_openapi(&payload.openapi_yaml).map_err(|e| {
        tracing::error!("Failed to parse OpenAPI: {}", e);
        StatusCode::BAD_REQUEST
    })?;

    let mut tx = state.db.begin().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    sqlx::query("INSERT OR IGNORE INTO services (name) VALUES (?)")
        .bind(&payload.servicename)
        .execute(&mut *tx)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let service: (i64,) = sqlx::query_as("SELECT id FROM services WHERE name = ?")
        .bind(&payload.servicename)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    sqlx::query("INSERT OR IGNORE INTO branches (service_id, name) VALUES (?, ?)")
        .bind(service.0)
        .bind(&payload.branch)
        .execute(&mut *tx)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let branch: (i64,) = sqlx::query_as("SELECT id FROM branches WHERE service_id = ? AND name = ?")
        .bind(service.0)
        .bind(&payload.branch)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Fetch existing endpoints for this branch
    let existing_endpoints: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT path, method, yaml_content FROM endpoints WHERE branch_id = ?"
    )
    .bind(branch.0)
    .fetch_all(&mut *tx)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut existing_map: HashMap<(String, String), String> = existing_endpoints
        .into_iter()
        .map(|(p, m, y)| ((p, m), y))
        .collect();

    let mut to_insert = Vec::new();

    for endpoint in endpoints {
        let key = (endpoint.path.clone(), endpoint.method.clone());
        if let Some(existing_yaml) = existing_map.remove(&key) {
            if existing_yaml != endpoint.yaml_content {
                // YAML changed for the same path and method - reject
                tracing::warn!(
                    "Rejected update for {} {}: DTO changed but path remained the same",
                    endpoint.method,
                    endpoint.path
                );
                return Err(StatusCode::CONFLICT);
            }
            // If identical, we do nothing (it's already there)
        } else {
            // New endpoint
            to_insert.push(endpoint);
        }
    }

    // Insert only new endpoints
    for endpoint in to_insert {
        sqlx::query("INSERT INTO endpoints (branch_id, path, method, yaml_content) VALUES (?, ?, ?, ?)")
            .bind(branch.0)
            .bind(endpoint.path)
            .bind(endpoint.method)
            .bind(endpoint.yaml_content)
            .execute(&mut *tx)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    tx.commit().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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

    let mut tx = state.db.begin().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Ensure client exists
    sqlx::query("INSERT OR IGNORE INTO clients (name) VALUES (?)")
        .bind(&params.clientname)
        .execute(&mut *tx)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let client: (i64,) = sqlx::query_as("SELECT id FROM clients WHERE name = ?")
        .bind(&params.clientname)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Ensure service exists (or at least know its ID)
    sqlx::query("INSERT OR IGNORE INTO services (name) VALUES (?)")
        .bind(&params.servicename)
        .execute(&mut *tx)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let service: (i64,) = sqlx::query_as("SELECT id FROM services WHERE name = ?")
        .bind(&params.servicename)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Try to find the endpoint
    let endpoint: Option<(i64, String)> = sqlx::query_as(
        r#"
        SELECT e.id, e.yaml_content
        FROM endpoints e
        JOIN branches b ON e.branch_id = b.id
        WHERE b.service_id = ? AND b.name = ? AND e.path = ? AND e.method = ?
        "#,
    )
    .bind(service.0)
    .bind(&params.branch)
    .bind(&params.path)
    .bind(params.method.to_uppercase())
    .fetch_optional(&mut *tx)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let endpoint_id = endpoint.as_ref().map(|e| e.0);
    let yaml_content = endpoint.as_ref().map(|e| e.1.clone());

    // Record dependency
    sqlx::query(
        r#"
        INSERT INTO dependencies 
        (client_id, endpoint_id, requested_service_id, requested_branch_name, requested_path, requested_method)
        VALUES (?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(client.0)
    .bind(endpoint_id)
    .bind(service.0)
    .bind(&params.branch)
    .bind(&params.path)
    .bind(params.method.to_uppercase())
    .execute(&mut *tx)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    tx.commit().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    match yaml_content {
        Some(yaml) => Ok(yaml),
        None => Err(StatusCode::NOT_FOUND),
    }
}

#[derive(Serialize)]
struct DependencyReport {
    branch: String,
    unused_endpoints: Vec<EndpointInfo>,
    missing_endpoints: Vec<MissingEndpointInfo>,
    dependency_graph: Vec<DependencyInfo>,
}

#[derive(Serialize)]
struct EndpointInfo {
    service: String,
    path: String,
    method: String,
}

#[derive(Serialize)]
struct MissingEndpointInfo {
    client: String,
    service: String,
    path: String,
    method: String,
}

#[derive(Serialize)]
struct DependencyInfo {
    client: String,
    service: String,
    path: String,
    method: String,
}

async fn report(
    State(state): State<AppState>,
    Query(params): Query<ReportParams>,
) -> Result<Json<DependencyReport>, StatusCode> {
    tracing::info!("Generating report for branch {}", params.branch);

    let mut conn = state.db.acquire().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // 1. Dependency Graph (All recorded dependencies on this branch)
    let dependency_rows: Vec<(String, String, String, String)> = sqlx::query_as(
        r#"
        SELECT c.name, s.name, d.requested_path, d.requested_method
        FROM dependencies d
        JOIN clients c ON d.client_id = c.id
        JOIN services s ON d.requested_service_id = s.id
        WHERE d.requested_branch_name = ?
        "#,
    )
    .bind(&params.branch)
    .fetch_all(&mut *conn)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let dependency_graph = dependency_rows
        .into_iter()
        .map(|(client, service, path, method)| DependencyInfo {
            client,
            service,
            path,
            method,
        })
        .collect();

    // 2. Unused Endpoints (Provided in this branch but not in dependencies table for this branch)
    let unused_rows: Vec<(String, String, String)> = sqlx::query_as(
        r#"
        SELECT s.name, e.path, e.method
        FROM endpoints e
        JOIN branches b ON e.branch_id = b.id
        JOIN services s ON b.service_id = s.id
        WHERE b.name = ? AND e.id NOT IN (
            SELECT endpoint_id FROM dependencies 
            WHERE requested_branch_name = ? AND endpoint_id IS NOT NULL
        )
        "#,
    )
    .bind(&params.branch)
    .bind(&params.branch)
    .fetch_all(&mut *conn)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let unused_endpoints = unused_rows
        .into_iter()
        .map(|(service, path, method)| EndpointInfo {
            service,
            path,
            method,
        })
        .collect();

    // 3. Missing Endpoints (In dependencies table for this branch but endpoint_id is NULL)
    let missing_rows: Vec<(String, String, String, String)> = sqlx::query_as(
        r#"
        SELECT c.name, s.name, d.requested_path, d.requested_method
        FROM dependencies d
        JOIN clients c ON d.client_id = c.id
        JOIN services s ON d.requested_service_id = s.id
        WHERE d.requested_branch_name = ? AND d.endpoint_id IS NULL
        "#,
    )
    .bind(&params.branch)
    .fetch_all(&mut *conn)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let missing_endpoints = missing_rows
        .into_iter()
        .map(|(client, service, path, method)| MissingEndpointInfo {
            client,
            service,
            path,
            method,
        })
        .collect();

    Ok(Json(DependencyReport {
        branch: params.branch,
        unused_endpoints,
        missing_endpoints,
        dependency_graph,
    }))
}

async fn report_markdown(
    State(state): State<AppState>,
    Query(params): Query<ReportParams>,
) -> Result<(axum::http::HeaderMap, String), StatusCode> {
    let report_json = report(State(state), Query(params)).await?;
    let report = report_json.0;

    let mut md = format!("# SanShain Dependency Report: Branch `{}`\n\n", report.branch);
    
    md.push_str("## Summary\n");
    md.push_str(&format!("- Total Dependencies: {}\n", report.dependency_graph.len()));
    md.push_str(&format!("- Unused Endpoints: {}\n", report.unused_endpoints.len()));
    md.push_str(&format!("- Missing Requirements: {}\n\n", report.missing_endpoints.len()));

    md.push_str("## Dependency Graph\n");
    if report.dependency_graph.is_empty() {
        md.push_str("No active dependencies recorded for this branch.\n\n");
    } else {
        md.push_str("| Client | Service | Path | Method |\n");
        md.push_str("| --- | --- | --- | --- |\n");
        for dep in report.dependency_graph {
            md.push_str(&format!("| {} | {} | `{}` | `{}` |\n", dep.client, dep.service, dep.path, dep.method));
        }
        md.push_str("\n");
    }

    md.push_str("## Unused Endpoints\n");
    md.push_str("> Endpoints that are provided by a service but have no recorded client requirements.\n\n");
    if report.unused_endpoints.is_empty() {
        md.push_str("All provided endpoints are in use.\n\n");
    } else {
        md.push_str("| Service | Path | Method |\n");
        md.push_str("| --- | --- | --- |\n");
        for ep in report.unused_endpoints {
            md.push_str(&format!("| {} | `{}` | `{}` |\n", ep.service, ep.path, ep.method));
        }
        md.push_str("\n");
    }

    md.push_str("## Missing Requirements\n");
    md.push_str("> Requirements from clients for endpoints that do not exist in this branch.\n\n");
    if report.missing_endpoints.is_empty() {
        md.push_str("No missing requirements identified.\n\n");
    } else {
        md.push_str("| Client | Service | Path | Method |\n");
        md.push_str("| --- | --- | --- | --- |\n");
        for m in report.missing_endpoints {
            md.push_str(&format!("| {} | {} | `{}` | `{}` |\n", m.client, m.service, m.path, m.method));
        }
        md.push_str("\n");
    }

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(axum::http::header::CONTENT_TYPE, "text/markdown; charset=utf-8".parse().unwrap());
    
    Ok((headers, md))
}
