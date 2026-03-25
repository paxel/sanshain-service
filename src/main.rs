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
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use std::sync::Arc;
use std::collections::HashSet;
use tokio::sync::RwLock;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub mod openapi;
pub mod domain;
pub mod application;
pub mod infrastructure;

use application::services::{self, AppError};
use infrastructure::database::DatabaseRepo;
use infrastructure::sqlite_repository::SqliteSpecRepository;
use infrastructure::postgres_repository::PostgresSpecRepository;
use infrastructure::ldap_provider::LdapAuthProvider;
use domain::models::{AuthMode, LdapConfig};

#[derive(Clone)]
pub struct AppState {
    pub repo: DatabaseRepo,
    pub db_url: String,
    pub csrf_tokens: Arc<RwLock<HashSet<String>>>,
}

#[tokio::main]
pub async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "sanshain_service=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let db_connection_str = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite:sanshain.db?mode=rwc".into());

    let repo = if db_connection_str.starts_with("postgres://") || db_connection_str.starts_with("postgresql://") {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&db_connection_str)
            .await
            .expect("can't connect to PostgreSQL database");
        let pg_repo = PostgresSpecRepository::new(pool);
        pg_repo.run_migrations().await.expect("can't run PostgreSQL migrations");
        tracing::info!("Using PostgreSQL database backend");
        DatabaseRepo::Postgres(pg_repo)
    } else {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&db_connection_str)
            .await
            .expect("can't connect to SQLite database");
        let sqlite_repo = SqliteSpecRepository::new(pool);
        sqlite_repo.run_migrations().await.expect("can't run SQLite migrations");
        tracing::info!("Using SQLite database backend");
        DatabaseRepo::Sqlite(sqlite_repo)
    };

    // Ensure initial admin user exists
    services::ensure_initial_admin(&repo).await.expect("can't create initial admin");

    let state = AppState {
        repo,
        db_url: db_connection_str,
        csrf_tokens: Arc::new(RwLock::new(HashSet::new())),
    };

    let app = create_app(state);

    let bind_address = std::env::var("BIND_ADDRESS").unwrap_or_else(|_| "0.0.0.0:3000".into());
    let addr: SocketAddr = bind_address.parse().expect("invalid BIND_ADDRESS");
    tracing::debug!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

use tower_http::services::ServeDir;
use axum::response::Redirect;
use axum::http::header::HeaderValue;

/// Resolve a user from either a session token or a san_ API token.
async fn resolve_user(repo: &DatabaseRepo, token: &str) -> Result<Option<domain::models::User>, StatusCode> {
    if token.starts_with("san_") {
        // API token
        let user = services::validate_api_token(repo, token)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if let Some(u) = user {
            if u.approved {
                return Ok(Some(u));
            }
        }
        Ok(None)
    } else {
        // Session token
        let result = services::validate_session(repo, token)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        Ok(result.map(|(u, _)| u))
    }
}

/// Middleware: admin endpoints always require a valid admin session token or API token.
async fn admin_auth(
    State(state): State<AppState>,
    req: axum::http::Request<Body>,
    next: middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
    let token = extract_bearer_token(&req).ok_or(StatusCode::UNAUTHORIZED)?;
    let user = resolve_user(&state.repo, &token).await?.ok_or(StatusCode::UNAUTHORIZED)?;
    if !user.is_admin {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(next.run(req).await)
}

/// Middleware: non-admin API endpoints require dev_mode=true OR a valid session/API token.
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

    // Dev mode off: require valid session or API token
    let token = extract_bearer_token(&req).ok_or(StatusCode::FORBIDDEN)?;
    let _user = resolve_user(&state.repo, &token).await?.ok_or(StatusCode::UNAUTHORIZED)?;
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

/// Middleware: adds security headers to all responses.
async fn security_headers(
    req: axum::http::Request<Body>,
    next: middleware::Next,
) -> axum::response::Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert(
        axum::http::header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; \
             script-src 'self' 'unsafe-inline' https://cdn.tailwindcss.com https://cdn.jsdelivr.net; \
             style-src 'self' 'unsafe-inline'; \
             img-src 'self' data:; \
             connect-src 'self'; \
             font-src 'self'; \
             frame-ancestors 'none'"
        ),
    );
    headers.insert(
        axum::http::header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        axum::http::header::X_FRAME_OPTIONS,
        HeaderValue::from_static("DENY"),
    );
    headers.insert(
        axum::http::HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        axum::http::HeaderName::from_static("x-xss-protection"),
        HeaderValue::from_static("1; mode=block"),
    );
    response
}

/// Middleware: validates CSRF token on state-changing requests (POST, DELETE, PUT, PATCH).
/// Skipped when dev_mode is enabled so that external tools (e.g. curl) can reach the API.
async fn csrf_protection(
    State(state): State<AppState>,
    req: axum::http::Request<Body>,
    next: middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
    let method = req.method().clone();
    if method == axum::http::Method::POST
        || method == axum::http::Method::DELETE
        || method == axum::http::Method::PUT
        || method == axum::http::Method::PATCH
    {
        // In dev mode, skip CSRF validation to allow unauthenticated API access
        let dev_mode = services::get_dev_mode(&state.repo)
            .await
            .unwrap_or(false);
        if !dev_mode {
            let csrf_token = req
                .headers()
                .get("X-CSRF-Token")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            match csrf_token {
                Some(token) => {
                    let valid = state.csrf_tokens.read().await.contains(&token);
                    if !valid {
                        return Err(StatusCode::FORBIDDEN);
                    }
                }
                None => {
                    return Err(StatusCode::FORBIDDEN);
                }
            }
        }
    }
    Ok(next.run(req).await)
}

pub fn create_app(state: AppState) -> Router {
    let admin_routes = Router::new()
        .route("/protected-branches", get(list_protected_branches).post(add_protected_branch))
        .route("/protected-branches/{pattern}", axum::routing::delete(delete_protected_branch))
        .route("/services", get(admin_list_services))
        .route("/services/{name}", axum::routing::delete(admin_delete_service))
        .route("/services/{name}/branches", get(admin_list_branches))
        .route("/services/{name}/branches/{branch}", axum::routing::delete(admin_delete_branch))
        .route("/clients", get(admin_list_clients))
        .route("/clients/{name}", axum::routing::delete(admin_delete_client))
        .route("/clients/{name}/branches", get(admin_list_client_branches))
        .route("/clients/{name}/branches/{branch}/endpoints", get(admin_list_client_endpoints))
        .route("/settings/dev-mode", get(get_dev_mode).post(set_dev_mode))
        .route("/settings/local-users", get(get_local_users).post(set_local_users))
        .route("/settings/database", get(get_database_info))
        .route("/auth-config", get(get_auth_config).put(set_auth_config))
        .route("/auth-config/test", post(test_auth_config))
        .route("/users", get(admin_list_users))
        .route("/users/{id}/approve", post(admin_approve_user))
        .route("/users/{id}", axum::routing::delete(admin_delete_user_handler))
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
        .route("/version", get(version))
        .route("/csrf-token", get(generate_csrf_token))
        .route("/auth/login", post(auth_login))
        .route("/auth/logout", post(auth_logout))
        .route("/auth/me", get(auth_me))
        .route("/auth/change-password", post(auth_change_password))
        .route("/auth/register", post(auth_register))
        .route("/auth/tokens", get(list_tokens).post(create_token))
        .route("/auth/tokens/{id}", axum::routing::delete(revoke_token))
        .route("/dashboard", get(dashboard_page))
        .merge(api_routes)
        .nest("/admin", admin_routes)
        .fallback_service(ServeDir::new("static"))
        .layer(middleware::from_fn_with_state(state.clone(), csrf_protection))
        .layer(middleware::from_fn(security_headers))
        .layer(tower_http::trace::TraceLayer::new_for_http())
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

#[derive(Serialize)]
struct DatabaseInfoResponse {
    backend: String,
    url: String,
}

async fn get_database_info(
    State(state): State<AppState>,
) -> Json<DatabaseInfoResponse> {
    // Mask credentials in the URL for display
    let masked_url = mask_database_url(&state.db_url);
    Json(DatabaseInfoResponse {
        backend: state.repo.backend_name().to_string(),
        url: masked_url,
    })
}

fn mask_database_url(url: &str) -> String {
    // For postgres URLs, mask the password
    if let Some(at_pos) = url.find('@') {
        if let Some(scheme_end) = url.find("://") {
            let prefix = &url[..scheme_end + 3];
            let user_pass = &url[scheme_end + 3..at_pos];
            let rest = &url[at_pos..];
            if let Some(colon) = user_pass.find(':') {
                let user = &user_pass[..colon];
                return format!("{}{}:****{}", prefix, user, rest);
            }
        }
    }
    url.to_string()
}

#[derive(Serialize)]
struct VersionResponse {
    version: &'static str,
}

async fn version() -> Json<VersionResponse> {
    Json(VersionResponse {
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[derive(Serialize)]
struct CsrfTokenResponse {
    csrf_token: String,
}

async fn generate_csrf_token(
    State(state): State<AppState>,
) -> Json<CsrfTokenResponse> {
    use rand::Rng;
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    let token: String = hex::encode(bytes);
    state.csrf_tokens.write().await.insert(token.clone());
    Json(CsrfTokenResponse { csrf_token: token })
}

fn app_error_to_status(e: AppError) -> StatusCode {
    match e {
        AppError::BadRequest(ref msg) => {
            tracing::warn!("Bad request: {}", msg);
            StatusCode::BAD_REQUEST
        }
        AppError::Conflict => {
            tracing::warn!("Conflict");
            StatusCode::CONFLICT
        }
        AppError::NotFound => {
            tracing::warn!("Not found");
            StatusCode::NOT_FOUND
        }
        AppError::Unauthorized => {
            tracing::warn!("Unauthorized");
            StatusCode::UNAUTHORIZED
        }
        AppError::Forbidden => {
            tracing::warn!("Forbidden");
            StatusCode::FORBIDDEN
        }
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
    approved: bool,
}

async fn auth_login(
    State(state): State<AppState>,
    Json(payload): Json<LoginPayload>,
) -> Result<Json<LoginResponse>, StatusCode> {
    let auth_mode = services::get_auth_mode(&state.repo).await.map_err(app_error_to_status)?;
    let session = match auth_mode {
        AuthMode::Ldap => {
            let ldap_config = services::get_ldap_config(&state.repo).await
                .map_err(app_error_to_status)?
                .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
            let provider = LdapAuthProvider::new(ldap_config);
            services::login_with_provider(&state.repo, &provider, &payload.username, &payload.password)
                .await
                .map_err(app_error_to_status)?
        }
        AuthMode::Local => {
            // Use existing local login
            services::login(&state.repo, &payload.username, &payload.password)
                .await
                .map_err(app_error_to_status)?
        }
        AuthMode::Dev => {
            // Dev mode — use local login (auth middleware skips checks anyway)
            services::login(&state.repo, &payload.username, &payload.password)
                .await
                .map_err(app_error_to_status)?
        }
    };
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
        approved: user.approved,
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

async fn admin_list_client_branches(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<Json<Vec<String>>, StatusCode> {
    services::list_client_branches(&state.repo, &name)
        .await
        .map(Json)
        .map_err(app_error_to_status)
}

async fn admin_list_client_endpoints(
    State(state): State<AppState>,
    axum::extract::Path((name, branch)): axum::extract::Path<(String, String)>,
) -> Result<Json<Vec<domain::models::ClientEndpointInfo>>, StatusCode> {
    services::list_client_endpoints(&state.repo, &name, &branch)
        .await
        .map(Json)
        .map_err(app_error_to_status)
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

// --- Registration endpoint ---

#[derive(Deserialize)]
struct RegisterPayload {
    username: String,
    password: String,
}

async fn auth_register(
    State(state): State<AppState>,
    Json(payload): Json<RegisterPayload>,
) -> Result<StatusCode, StatusCode> {
    services::register_user(&state.repo, &payload.username, &payload.password)
        .await
        .map(|_| StatusCode::CREATED)
        .map_err(app_error_to_status)
}

// --- Local users setting ---

#[derive(Serialize)]
struct LocalUsersResponse {
    local_users_enabled: bool,
}

#[derive(Deserialize)]
struct LocalUsersPayload {
    enabled: bool,
}

async fn get_local_users(
    State(state): State<AppState>,
) -> Result<Json<LocalUsersResponse>, StatusCode> {
    let enabled = services::get_local_users_enabled(&state.repo)
        .await
        .map_err(app_error_to_status)?;
    Ok(Json(LocalUsersResponse { local_users_enabled: enabled }))
}

async fn set_local_users(
    State(state): State<AppState>,
    Json(payload): Json<LocalUsersPayload>,
) -> Result<StatusCode, StatusCode> {
    services::set_local_users_enabled(&state.repo, payload.enabled)
        .await
        .map_err(app_error_to_status)?;
    Ok(StatusCode::OK)
}

// --- Auth Config endpoints ---

#[derive(Serialize, Deserialize)]
struct AuthConfigResponse {
    auth_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    ldap_config: Option<LdapConfig>,
}

#[derive(Deserialize)]
struct AuthConfigPayload {
    auth_mode: String,
    #[serde(default)]
    ldap_config: Option<LdapConfig>,
}

async fn get_auth_config(
    State(state): State<AppState>,
) -> Result<Json<AuthConfigResponse>, StatusCode> {
    let mode = services::get_auth_mode(&state.repo).await.map_err(app_error_to_status)?;
    let ldap = services::get_ldap_config(&state.repo).await.map_err(app_error_to_status)?;
    // Redact bind password in response
    let ldap_redacted = ldap.map(|mut c| {
        if c.bind_password.is_some() {
            c.bind_password = Some("****".to_string());
        }
        c
    });
    Ok(Json(AuthConfigResponse {
        auth_mode: mode.as_str().to_string(),
        ldap_config: ldap_redacted,
    }))
}

async fn set_auth_config(
    State(state): State<AppState>,
    Json(payload): Json<AuthConfigPayload>,
) -> Result<StatusCode, StatusCode> {
    let mode = AuthMode::from_str(&payload.auth_mode)
        .ok_or(StatusCode::BAD_REQUEST)?;

    if mode == AuthMode::Ldap {
        let config = payload.ldap_config.ok_or(StatusCode::BAD_REQUEST)?;
        // If password is "****", keep the existing one
        let final_config = if config.bind_password.as_deref() == Some("****") {
            let existing = services::get_ldap_config(&state.repo).await.map_err(app_error_to_status)?;
            LdapConfig {
                bind_password: existing.and_then(|e| e.bind_password),
                ..config
            }
        } else {
            config
        };
        services::set_ldap_config(&state.repo, &final_config).await.map_err(app_error_to_status)?;
    }

    services::set_auth_mode(&state.repo, &mode).await.map_err(app_error_to_status)?;
    Ok(StatusCode::OK)
}

async fn test_auth_config(
    State(state): State<AppState>,
    Json(config): Json<LdapConfig>,
) -> Result<StatusCode, StatusCode> {
    // If password is "****", use existing stored password
    let final_config = if config.bind_password.as_deref() == Some("****") {
        let existing = services::get_ldap_config(&state.repo).await.map_err(app_error_to_status)?;
        LdapConfig {
            bind_password: existing.and_then(|e| e.bind_password),
            ..config
        }
    } else {
        config
    };
    final_config.validate().map_err(|_| StatusCode::BAD_REQUEST)?;
    let provider = LdapAuthProvider::new(final_config);
    services::test_ldap_connection(&provider).await.map_err(app_error_to_status)?;
    Ok(StatusCode::OK)
}

// --- User management admin endpoints ---

#[derive(Serialize)]
struct UserResponse {
    id: i64,
    username: String,
    is_admin: bool,
    approved: bool,
}

async fn admin_list_users(
    State(state): State<AppState>,
) -> Result<Json<Vec<UserResponse>>, StatusCode> {
    let users = services::list_users(&state.repo)
        .await
        .map_err(app_error_to_status)?;
    Ok(Json(users.into_iter().map(|u| UserResponse {
        id: u.id,
        username: u.username,
        is_admin: u.is_admin,
        approved: u.approved,
    }).collect()))
}

async fn admin_approve_user(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<StatusCode, StatusCode> {
    let approved = services::approve_user(&state.repo, id)
        .await
        .map_err(app_error_to_status)?;
    if approved {
        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn admin_delete_user_handler(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<StatusCode, StatusCode> {
    let deleted = services::admin_delete_user(&state.repo, id)
        .await
        .map_err(app_error_to_status)?;
    if deleted {
        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

// --- API Token Management ---

#[derive(Deserialize)]
struct CreateTokenPayload {
    name: String,
    expires_in_days: Option<u64>,
}

#[derive(Serialize)]
struct CreateTokenResponse {
    id: String,
    token: String,
    name: String,
    expires_in_days: u64,
}

#[derive(Serialize)]
struct TokenListItem {
    id: String,
    name: String,
    created_at: String,
    expires_at: String,
    last_used_at: Option<String>,
}

async fn create_token(
    State(state): State<AppState>,
    req: axum::http::Request<Body>,
) -> Result<Json<CreateTokenResponse>, StatusCode> {
    let token = extract_bearer_token(&req).ok_or(StatusCode::UNAUTHORIZED)?;
    let user = resolve_user(&state.repo, &token).await?.ok_or(StatusCode::UNAUTHORIZED)?;

    let body_bytes = axum::body::to_bytes(req.into_body(), 1024 * 64)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let payload: CreateTokenPayload = serde_json::from_slice(&body_bytes)
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let expires_in_days = payload.expires_in_days.unwrap_or(365);
    let (id, raw_token) = services::create_api_token(&state.repo, user.id, &payload.name, expires_in_days)
        .await
        .map_err(app_error_to_status)?;

    Ok(Json(CreateTokenResponse {
        id,
        token: raw_token,
        name: payload.name,
        expires_in_days,
    }))
}

async fn list_tokens(
    State(state): State<AppState>,
    req: axum::http::Request<Body>,
) -> Result<Json<Vec<TokenListItem>>, StatusCode> {
    let token = extract_bearer_token(&req).ok_or(StatusCode::UNAUTHORIZED)?;
    let user = resolve_user(&state.repo, &token).await?.ok_or(StatusCode::UNAUTHORIZED)?;

    let tokens = services::list_api_tokens(&state.repo, user.id)
        .await
        .map_err(app_error_to_status)?;

    Ok(Json(tokens.into_iter().map(|t| TokenListItem {
        id: t.id,
        name: t.name,
        created_at: t.created_at,
        expires_at: t.expires_at,
        last_used_at: t.last_used_at,
    }).collect()))
}

async fn revoke_token(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    req: axum::http::Request<Body>,
) -> Result<StatusCode, StatusCode> {
    let token = extract_bearer_token(&req).ok_or(StatusCode::UNAUTHORIZED)?;
    let user = resolve_user(&state.repo, &token).await?.ok_or(StatusCode::UNAUTHORIZED)?;

    let deleted = services::revoke_api_token(&state.repo, &id, user.id)
        .await
        .map_err(app_error_to_status)?;

    if deleted {
        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

// --- Dashboard (Askama template) ---

#[derive(askama::Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    username: String,
    is_admin: bool,
}

async fn dashboard_page(
    State(state): State<AppState>,
    req: axum::http::Request<Body>,
) -> Result<axum::response::Response, StatusCode> {
    let token = extract_bearer_token(&req);
    match token {
        Some(t) => {
            let user = resolve_user(&state.repo, &t).await?.ok_or(StatusCode::UNAUTHORIZED)?;
            let tmpl = DashboardTemplate {
                username: user.username,
                is_admin: user.is_admin,
            };
            let html = tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            Ok(axum::response::Html(html).into_response())
        }
        None => {
            // No auth — show login redirect page
            Ok(axum::response::Redirect::temporary("/service.html").into_response())
        }
    }
}

use axum::response::IntoResponse;
use askama::Template;
