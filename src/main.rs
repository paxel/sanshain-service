use axum::{
    body::Body,
    extract::{Query, State},
    http::StatusCode,
    middleware,
    routing::{get, post},
    Json, Router,
};
use axum_prometheus::PrometheusMetricLayer;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqlitePoolOptions, SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous};
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::collections::{HashMap, VecDeque};
use std::str::FromStr;
use tokio::sync::RwLock;
use chrono::{DateTime, Utc, Duration};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

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
    pub csrf_tokens: Arc<RwLock<HashMap<String, DateTime<Utc>>>>,
    pub instance_id: String,
    pub spec_updated_tx: tokio::sync::broadcast::Sender<()>,
    pub log_buffer: Arc<std::sync::Mutex<VecDeque<domain::models::LogEntry>>>,
    pub business_logic_debug: Arc<AtomicBool>,
    pub admin_user_debug: Arc<AtomicBool>,
    pub requests_total: Arc<AtomicU64>,
    pub failures_total: Arc<AtomicU64>,
    pub process_start_time: DateTime<Utc>,
    pub prometheus_handle: metrics_exporter_prometheus::PrometheusHandle,
}

struct LogVisitor<'a> {
    message: &'a mut String,
}

impl<'a> tracing::field::Visit for LogVisitor<'a> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            *self.message = format!("{:?}", value);
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            *self.message = value.to_string();
        }
    }
}

struct LogCaptureLayer {
    buffer: Arc<std::sync::Mutex<VecDeque<domain::models::LogEntry>>>,
    business_logic_debug: Arc<AtomicBool>,
    admin_user_debug: Arc<AtomicBool>,
}

impl<S> tracing_subscriber::Layer<S> for LogCaptureLayer
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let metadata = event.metadata();
        let target = metadata.target();
        let level_str = metadata.level().to_string();

        // Check if debug flags permit this message
        if level_str == "DEBUG" {
            if target.starts_with("sanshain_service::application") {
                if !self.business_logic_debug.load(Ordering::Relaxed) {
                    return;
                }
            } else if !self.admin_user_debug.load(Ordering::Relaxed) {
                // For all other targets (including main.rs/presentation)
                return;
            }
        }

        let mut message = String::new();
        let mut visitor = LogVisitor { message: &mut message };
        event.record(&mut visitor);

        if message.is_empty() {
            // Some events might not have a message field, skip them or use a placeholder
            return;
        }

        let entry = domain::models::LogEntry {
            timestamp: Utc::now().to_rfc3339(),
            level: level_str,
            target: target.to_string(),
            message,
        };

        if let Ok(mut buf) = self.buffer.lock() {
            if buf.len() >= 100 {
                buf.pop_front();
            }
            buf.push_back(entry);
        }
    }
}

#[tokio::main]
pub async fn main() {
    let business_logic_debug = Arc::new(AtomicBool::new(false));
    let admin_user_debug = Arc::new(AtomicBool::new(false));
    let requests_total = Arc::new(AtomicU64::new(0));
    let failures_total = Arc::new(AtomicU64::new(0));
    let log_buffer = Arc::new(std::sync::Mutex::new(VecDeque::with_capacity(101)));

    let capture_layer = LogCaptureLayer {
        buffer: log_buffer.clone(),
        business_logic_debug: business_logic_debug.clone(),
        admin_user_debug: admin_user_debug.clone(),
    };

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "sanshain_service=info,tower_http=info".into());
    
    let registry = tracing_subscriber::registry()
        .with(filter)
        .with(capture_layer);

    if std::env::var("LOG_FORMAT").unwrap_or_default() == "json" {
        registry.with(tracing_subscriber::fmt::layer().json()).init();
    } else {
        registry.with(tracing_subscriber::fmt::layer()).init();
    }

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
        let connection_options = SqliteConnectOptions::from_str(&db_connection_str)
            .expect("invalid database URL")
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(5))
            .synchronous(SqliteSynchronous::Normal);

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(connection_options)
            .await
            .expect("can't connect to SQLite database");
        let sqlite_repo = SqliteSpecRepository::new(pool);
        sqlite_repo.run_migrations().await.expect("can't run SQLite migrations");
        tracing::info!("Using SQLite database backend");
        DatabaseRepo::Sqlite(sqlite_repo)
    };

    // Ensure initial admin user exists
    services::ensure_initial_admin(&repo).await.expect("can't create initial admin");

    let instance_id = uuid::Uuid::new_v4().to_string();
    tracing::info!("Instance ID: {}", instance_id);

    let (prometheus_layer, prometheus_handle) = PrometheusMetricLayer::pair();
    let (spec_updated_tx, _) = tokio::sync::broadcast::channel(100);
    let state = AppState {
        repo,
        db_url: db_connection_str,
        csrf_tokens: Arc::new(RwLock::new(HashMap::new())),
        instance_id,
        spec_updated_tx,
        log_buffer,
        business_logic_debug,
        admin_user_debug,
        requests_total,
        failures_total,
        process_start_time: Utc::now(),
        prometheus_handle,
    };

    // Spawn background branch cleanup task (runs every hour)
    let cleanup_repo = state.repo.clone();
    let cleanup_csrf = state.csrf_tokens.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            interval.tick().await;
            match services::cleanup_stale_branches(&cleanup_repo).await {
                Ok(0) => {},
                Ok(n) => tracing::info!("Branch cleanup: deleted {} stale branches", n),
                Err(e) => tracing::warn!("Branch cleanup failed: {:?}", e),
            }
            match services::cleanup_stale_dependencies(&cleanup_repo).await {
                Ok(0) => {},
                Ok(n) => tracing::info!("Dependency cleanup: pruned {} stale dependencies", n),
                Err(e) => tracing::warn!("Dependency cleanup failed: {:?}", e),
            }

            // Prune expired CSRF tokens (older than 24 hours)
            {
                let mut tokens = cleanup_csrf.write().await;
                let now = Utc::now();
                let max_age = Duration::hours(24);
                let before_count = tokens.len();
                tokens.retain(|_, created_at| now - *created_at < max_age);
                let after_count = tokens.len();
                if before_count > after_count {
                    tracing::info!("CSRF cleanup: pruned {} expired tokens", before_count - after_count);
                }
            }
        }
    });

    let app = create_app(state.clone())
        .layer(prometheus_layer)
        .route("/metrics", get(move || {
            let handle = state.prometheus_handle.clone();
            async move { handle.render() }
        }));

    let bind_address = std::env::var("BIND_ADDRESS").unwrap_or_else(|_| "0.0.0.0:3000".into());
    let addr: SocketAddr = bind_address.parse().expect("invalid BIND_ADDRESS");
    tracing::debug!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
    tracing::info!("Server shut down gracefully");
}

async fn shutdown_signal() {
    use tokio::signal;
    let ctrl_c = async {
        signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => { tracing::info!("Received SIGINT, shutting down..."); },
        _ = terminate => { tracing::info!("Received SIGTERM, shutting down..."); },
    }
}

use tower_http::services::ServeDir;
use axum::http::header::HeaderValue;

/// Resolve a user from either a session token or a san_ API token.
async fn resolve_user(repo: &DatabaseRepo, token: &str) -> Result<Option<domain::models::User>, StatusCode> {
    if token.starts_with("san_") {
        // API token
        let user = services::validate_api_token(repo, token)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if let Some(u) = user
            && u.approved
        {
            return Ok(Some(u));
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
             script-src 'self' 'unsafe-inline' https://cdn.tailwindcss.com https://cdn.jsdelivr.net https://unpkg.com; \
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
        // Bypass CSRF for requests using Authorization headers (API tokens or manual sessions).
        // If an Authorization header is present, the client is not relying on browser-automatic
        // cookie transmission, which is the vector for CSRF.
        if req.headers().contains_key(axum::http::header::AUTHORIZATION) {
            return Ok(next.run(req).await);
        }

        // In dev mode, skip CSRF validation to allow unauthenticated API access
        let dev_mode = services::get_dev_mode(&state.repo)
            .await
            .unwrap_or(false);
        if !dev_mode {
            let csrf_token = req
                .headers()
                .get("X-CSRF-Token")
                .and_then(|v| v.to_str().ok());

            match csrf_token {
                Some(token) => {
                    let tokens = state.csrf_tokens.read().await;
                    if let Some(created_at) = tokens.get(token) {
                        let now = Utc::now();
                        let max_age = Duration::hours(24);
                        if now - *created_at < max_age {
                            // Token is valid and not expired
                            drop(tokens);
                            return Ok(next.run(req).await);
                        }
                    }
                    return Err(StatusCode::FORBIDDEN);
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
        .route("/services/{name}/branches/{branch}/endpoints", get(admin_list_service_endpoints))
        .route("/endpoint-yaml", get(admin_get_endpoint_yaml))
        .route("/endpoint-versions", get(admin_get_endpoint_versions))
        .route("/settings/dev-mode", get(get_dev_mode).post(set_dev_mode))
        .route("/settings/local-users", get(get_local_users).post(set_local_users))
        .route("/settings/database", get(get_database_info))
        .route("/auth-config", get(get_auth_config).put(set_auth_config))
        .route("/auth-config/test", post(test_auth_config))
        .route("/settings/branch-max-age", get(get_branch_max_age).post(set_branch_max_age))
        .route("/settings/branch-cleanup", post(trigger_branch_cleanup))
        .route("/settings/dependency-max-age", get(get_dependency_max_age).post(set_dependency_max_age))
        .route("/settings/dependency-cleanup", post(trigger_dependency_cleanup))
        .route("/users", get(admin_list_users))
        .route("/observability/logs", get(admin_get_logs))
        .route("/observability/stats", get(admin_get_stats))
        .route("/observability/debug-config", get(admin_get_debug_config).post(admin_set_debug_config))
        .route("/users/{id}/approve", post(admin_approve_user))
        .route("/users/{id}", axum::routing::delete(admin_delete_user_handler))
        .route_layer(middleware::from_fn_with_state(state.clone(), admin_auth));

    let fragment_routes = Router::new()
        .route("/admin/users", get(fragment_users))
        .route("/admin/users/{id}/approve", post(fragment_approve_user))
        .route("/admin/users/{id}", axum::routing::delete(fragment_delete_user))
        .route("/admin/services", get(fragment_services))
        .route("/admin/services/{name}", axum::routing::delete(fragment_delete_service))
        .route("/admin/services/{name}/fallback-branch", post(fragment_set_fallback_branch))
        .route("/admin/services/{name}/branches", get(fragment_branches))
        .route("/admin/services/{name}/branches/{branch}", axum::routing::delete(fragment_delete_branch))
        .route("/admin/clients", get(fragment_clients))
        .route("/admin/clients/{name}", axum::routing::delete(fragment_delete_client))
        .route("/admin/dev-mode", get(fragment_dev_mode))
        .route("/admin/dev-mode/toggle", post(fragment_dev_mode_toggle))
        .route("/admin/local-users", get(fragment_local_users))
        .route("/admin/local-users/toggle", post(fragment_local_users_toggle))
        .route("/admin/database-info", get(fragment_database_info))
        .route("/admin/protected-branches", get(fragment_protected_branches).post(fragment_add_protected_branch))
        .route("/admin/protected-branches/{pattern}", axum::routing::delete(fragment_delete_protected_branch))
        .route("/admin/auth-config", get(fragment_auth_config))
        .route("/admin/branch-max-age", get(fragment_branch_max_age).post(fragment_set_branch_max_age))
        .route("/admin/branch-cleanup", post(fragment_branch_cleanup))
        .route("/admin/dependency-max-age", get(fragment_dependency_max_age).post(fragment_set_dependency_max_age))
        .route("/admin/dependency-cleanup", post(fragment_dependency_cleanup))
        .route_layer(middleware::from_fn_with_state(state.clone(), admin_auth));

    let api_routes = Router::new()
        .route("/provide", post(provide))
        .route("/require", get(require))
        .route("/require-bundle", post(require_bundle))
        .route("/report", get(report))
        .route("/report/markdown", get(report_markdown))
        .route("/endpoint-versions", get(endpoint_versions))
        .route_layer(middleware::from_fn_with_state(state.clone(), api_auth));

    Router::new()
        .route("/", get(index_page))
        .route("/index.html", get(index_page))
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
        .route("/admin.html", get(admin_page))
        .route("/account", get(account_page))
        .route("/services", get(services_page))
        .merge(api_routes)
        .nest("/admin", admin_routes)
        .nest("/fragments", fragment_routes)
        .fallback_service(
            tower::ServiceBuilder::new()
                .layer(tower_http::set_header::SetResponseHeaderLayer::if_not_present(
                    axum::http::header::CACHE_CONTROL,
                    HeaderValue::from_static("no-cache, must-revalidate"),
                ))
                .service(ServeDir::new("static"))
        )
        .layer(middleware::from_fn_with_state(state.clone(), request_counter))
        .layer(middleware::from_fn_with_state(state.clone(), csrf_protection))
        .layer(middleware::from_fn(security_headers))
        .layer(tower_http::decompression::RequestDecompressionLayer::new())
        .layer(tower_http::compression::CompressionLayer::new())
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
}

#[derive(Deserialize)]
struct ProvidePayload {
    servicename: String,
    branch: String,
    openapi_yaml: String,
    #[serde(default)]
    dry_run: bool,
}

#[derive(Deserialize)]
struct RequireParams {
    clientname: String,
    servicename: String,
    branch: String,
    path: String,
    method: String,
    timeout: Option<u64>,
    #[serde(default)]
    dry_run: Option<bool>,
}

#[derive(Deserialize)]
struct RequireBundleEndpoint {
    path: String,
    method: String,
}

#[derive(Deserialize)]
struct RequireBundlePayload {
    clientname: String,
    servicename: String,
    branch: String,
    endpoints: Vec<RequireBundleEndpoint>,
    timeout: Option<u64>,
    #[serde(default)]
    dry_run: bool,
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
    if let Some(at_pos) = url.find('@')
        && let Some(scheme_end) = url.find("://")
    {
        let prefix = &url[..scheme_end + 3];
        let user_pass = &url[scheme_end + 3..at_pos];
        let rest = &url[at_pos..];
        if let Some(colon) = user_pass.find(':') {
            let user = &user_pass[..colon];
            return format!("{}{}:****{}", prefix, user, rest);
        }
    }
    url.to_string()
}

#[derive(Serialize)]
struct VersionResponse {
    version: &'static str,
    instance_id: String,
}

async fn version(
    State(state): State<AppState>,
) -> Json<VersionResponse> {
    Json(VersionResponse {
        version: env!("CARGO_PKG_VERSION"),
        instance_id: state.instance_id.clone(),
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
    state.csrf_tokens.write().await.insert(token.clone(), Utc::now());
    Json(CsrfTokenResponse { csrf_token: token })
}

fn app_error_to_status(e: AppError) -> StatusCode {
    match e {
        AppError::BadRequest(ref msg) => {
            tracing::warn!("Bad request: {}", msg);
            StatusCode::BAD_REQUEST
        }
        AppError::Conflict(ref msg) => {
            tracing::warn!("Conflict: {}", msg);
            StatusCode::CONFLICT
        }
        AppError::NotFound(ref msg) => {
            tracing::warn!("Not found: {}", msg);
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

fn app_error_to_status_with_body(e: AppError) -> (StatusCode, String) {
    let body = match &e {
        AppError::BadRequest(msg) => msg.clone(),
        AppError::NotFound(msg) => msg.clone(),
        AppError::Internal(msg) => msg.clone(),
        AppError::Conflict(msg) => msg.clone(),
        AppError::Unauthorized => "Unauthorized".to_string(),
        AppError::Forbidden => "Forbidden".to_string(),
    };
    (app_error_to_status(e), body)
}

async fn provide(
    State(state): State<AppState>,
    Json(payload): Json<ProvidePayload>,
) -> Result<StatusCode, (StatusCode, String)> {
    let result = if payload.dry_run {
        services::provide_spec_dry_run(&state.repo, &payload.servicename, &payload.branch, &payload.openapi_yaml).await
    } else {
        services::provide_spec(&state.repo, &payload.servicename, &payload.branch, &payload.openapi_yaml).await
    };

    if result.is_ok() && !payload.dry_run {
        let _ = state.spec_updated_tx.send(());
    }

    result
        .map(|_| StatusCode::ACCEPTED)
        .map_err(app_error_to_status_with_body)
}

async fn require(
    State(state): State<AppState>,
    Query(params): Query<RequireParams>,
) -> Result<String, (StatusCode, String)> {
    let dry_run = params.dry_run.unwrap_or(false);
    let notifier = Some(state.spec_updated_tx.subscribe());
    let result = if dry_run {
        services::require_endpoint_dry_run(
            &state.repo,
            notifier,
            &params.clientname,
            &params.servicename,
            &params.branch,
            &params.path,
            &params.method,
            params.timeout,
        ).await
    } else {
        services::require_endpoint(
            &state.repo,
            notifier,
            &params.clientname,
            &params.servicename,
            &params.branch,
            &params.path,
            &params.method,
            params.timeout,
        ).await
    };
    result.map_err(app_error_to_status_with_body)
}

async fn require_bundle(
    State(state): State<AppState>,
    Json(payload): Json<RequireBundlePayload>,
) -> Result<String, (StatusCode, String)> {
    let endpoints: Vec<(String, String)> = payload.endpoints
        .into_iter()
        .map(|e| (e.path, e.method))
        .collect();
    let notifier = Some(state.spec_updated_tx.subscribe());
    let result = if payload.dry_run {
        services::require_bundle_dry_run(
            &state.repo,
            notifier,
            &payload.clientname,
            &payload.servicename,
            &payload.branch,
            &endpoints,
            payload.timeout,
        ).await
    } else {
        services::require_bundle(
            &state.repo,
            notifier,
            &payload.clientname,
            &payload.servicename,
            &payload.branch,
            &endpoints,
            payload.timeout,
        ).await
    };
    result.map_err(app_error_to_status_with_body)
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

#[derive(Deserialize)]
struct EndpointVersionsParams {
    servicename: String,
    branch: String,
    path: String,
    method: String,
}

async fn endpoint_versions(
    State(state): State<AppState>,
    Query(params): Query<EndpointVersionsParams>,
) -> Result<Json<Vec<domain::models::EndpointVersion>>, (StatusCode, String)> {
    services::get_endpoint_version_history(
        &state.repo,
        &params.servicename,
        &params.branch,
        &params.path,
        &params.method,
    )
    .await
    .map(Json)
    .map_err(app_error_to_status_with_body)
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

// --- Read-only UI endpoints (no side effects) ---

#[derive(Deserialize)]
struct AdminEndpointYamlParams {
    servicename: String,
    branch: String,
    path: String,
    method: String,
}

async fn admin_get_endpoint_yaml(
    State(state): State<AppState>,
    Query(params): Query<AdminEndpointYamlParams>,
) -> Result<String, (StatusCode, String)> {
    services::get_endpoint_yaml(
        &state.repo,
        &params.servicename,
        &params.branch,
        &params.path,
        &params.method,
    )
    .await
    .map_err(app_error_to_status_with_body)
}

#[derive(Serialize)]
struct AdminEndpointInfo {
    path: String,
    method: String,
}

async fn admin_list_service_endpoints(
    State(state): State<AppState>,
    axum::extract::Path((name, branch)): axum::extract::Path<(String, String)>,
) -> Result<Json<Vec<AdminEndpointInfo>>, StatusCode> {
    services::list_service_endpoints(&state.repo, &name, &branch)
        .await
        .map(|eps| Json(eps.into_iter().map(|e| AdminEndpointInfo { path: e.path, method: e.method }).collect()))
        .map_err(app_error_to_status)
}

async fn admin_get_endpoint_versions(
    State(state): State<AppState>,
    Query(params): Query<EndpointVersionsParams>,
) -> Result<Json<Vec<domain::models::EndpointVersion>>, (StatusCode, String)> {
    services::get_endpoint_version_history(
        &state.repo,
        &params.servicename,
        &params.branch,
        &params.path,
        &params.method,
    )
    .await
    .map(Json)
    .map_err(app_error_to_status_with_body)
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
    let mode: AuthMode = payload.auth_mode.parse()
        .map_err(|_| StatusCode::BAD_REQUEST)?;

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

#[derive(Serialize)]
struct BranchMaxAgeResponse {
    days: u64,
}

#[derive(Deserialize)]
struct BranchMaxAgePayload {
    days: u64,
}

async fn get_branch_max_age(
    State(state): State<AppState>,
) -> Result<Json<BranchMaxAgeResponse>, StatusCode> {
    let days = services::get_branch_max_age_days(&state.repo)
        .await
        .map_err(app_error_to_status)?;
    Ok(Json(BranchMaxAgeResponse { days }))
}

async fn set_branch_max_age(
    State(state): State<AppState>,
    Json(payload): Json<BranchMaxAgePayload>,
) -> Result<StatusCode, StatusCode> {
    services::set_branch_max_age_days(&state.repo, payload.days)
        .await
        .map_err(app_error_to_status)?;
    Ok(StatusCode::OK)
}

async fn trigger_branch_cleanup(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let deleted = services::cleanup_stale_branches(&state.repo)
        .await
        .map_err(app_error_to_status)?;
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}

#[derive(Serialize)]
struct DependencyMaxAgeResponse {
    days: u64,
}

#[derive(Deserialize)]
struct DependencyMaxAgePayload {
    days: u64,
}

async fn get_dependency_max_age(
    State(state): State<AppState>,
) -> Result<Json<DependencyMaxAgeResponse>, StatusCode> {
    let days = services::get_dependency_max_age_days(&state.repo)
        .await
        .map_err(app_error_to_status)?;
    Ok(Json(DependencyMaxAgeResponse { days }))
}

async fn set_dependency_max_age(
    State(state): State<AppState>,
    Json(payload): Json<DependencyMaxAgePayload>,
) -> Result<StatusCode, StatusCode> {
    services::set_dependency_max_age_days(&state.repo, payload.days)
        .await
        .map_err(app_error_to_status)?;
    Ok(StatusCode::OK)
}

async fn trigger_dependency_cleanup(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let deleted = services::cleanup_stale_dependencies(&state.repo)
        .await
        .map_err(app_error_to_status)?;
    Ok(Json(serde_json::json!({ "deleted": deleted })))
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

// --- Askama page templates ---

#[derive(askama::Template)]
#[template(path = "index.html")]
struct IndexTemplate {}

async fn index_page() -> Result<axum::response::Response, StatusCode> {
    let tmpl = IndexTemplate {};
    let html = tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(axum::response::Html(html).into_response())
}

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

// --- Static page redirects (serve .html files at clean URLs) ---

async fn account_page() -> impl IntoResponse {
    axum::response::Redirect::permanent("/account.html")
}

async fn services_page() -> impl IntoResponse {
    axum::response::Redirect::permanent("/service.html")
}

// --- Admin page (htmx-powered, served from Askama template) ---

#[derive(Template)]
#[template(path = "admin.html")]
struct AdminPageTemplate {}

async fn admin_page() -> Result<axum::response::Response, StatusCode> {
    let tmpl = AdminPageTemplate {};
    let html = tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(axum::response::Html(html).into_response())
}

// --- Fragment handlers (return HTML snippets for htmx) ---

#[derive(Template)]
#[template(path = "fragments/admin/users.html")]
struct FragmentUsers {
    users: Vec<domain::models::User>,
}

async fn fragment_users(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, StatusCode> {
    let users = services::list_users(&state.repo).await.map_err(app_error_to_status)?;
    let tmpl = FragmentUsers { users };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

async fn fragment_approve_user(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<axum::response::Html<String>, StatusCode> {
    services::approve_user(&state.repo, id).await.map_err(app_error_to_status)?;
    let users = services::list_users(&state.repo).await.map_err(app_error_to_status)?;
    let tmpl = FragmentUsers { users };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

async fn fragment_delete_user(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<axum::response::Html<String>, StatusCode> {
    services::admin_delete_user(&state.repo, id).await.map_err(app_error_to_status)?;
    let users = services::list_users(&state.repo).await.map_err(app_error_to_status)?;
    let tmpl = FragmentUsers { users };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

#[derive(Template)]
#[template(path = "fragments/admin/services.html")]
struct FragmentServices {
    services: Vec<domain::models::ServiceSummary>,
}

async fn fragment_services(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, StatusCode> {
    let services_list = services::list_services_detailed(&state.repo).await.map_err(app_error_to_status)?;
    let tmpl = FragmentServices { services: services_list };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

async fn fragment_delete_service(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<axum::response::Html<String>, StatusCode> {
    services::delete_service(&state.repo, &name).await.map_err(app_error_to_status)?;
    let services_list = services::list_services_detailed(&state.repo).await.map_err(app_error_to_status)?;
    let tmpl = FragmentServices { services: services_list };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

#[derive(Template)]
#[template(path = "fragments/admin/branches.html")]
struct FragmentBranches {
    service_name: String,
    branches: Vec<String>,
}

async fn fragment_branches(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<axum::response::Html<String>, StatusCode> {
    let branches = services::list_branches(&state.repo, &name).await.map_err(app_error_to_status)?;
    let tmpl = FragmentBranches { service_name: name, branches };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

async fn fragment_delete_branch(
    State(state): State<AppState>,
    axum::extract::Path((name, branch)): axum::extract::Path<(String, String)>,
) -> Result<axum::response::Html<String>, StatusCode> {
    services::delete_branch(&state.repo, &name, &branch).await.map_err(app_error_to_status)?;
    // Return the full services list since the target is #services-list
    let services_list = services::list_services_detailed(&state.repo).await.map_err(app_error_to_status)?;
    let tmpl = FragmentServices { services: services_list };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

#[derive(Deserialize)]
struct SetFallbackBranchParams {
    branch: String,
}

async fn fragment_set_fallback_branch(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
    axum::extract::Form(params): axum::extract::Form<SetFallbackBranchParams>,
) -> Result<axum::response::Html<String>, StatusCode> {
    let branch = if params.branch.trim().is_empty() {
        None
    } else {
        Some(params.branch.trim())
    };
    services::set_fallback_branch(&state.repo, &name, branch).await.map_err(app_error_to_status)?;
    
    let services_list = services::list_services_detailed(&state.repo).await.map_err(app_error_to_status)?;
    let tmpl = FragmentServices { services: services_list };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

#[derive(Template)]
#[template(path = "fragments/admin/clients.html")]
struct FragmentClients {
    clients: Vec<String>,
}

async fn fragment_clients(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, StatusCode> {
    let clients = services::list_clients(&state.repo).await.map_err(app_error_to_status)?;
    let tmpl = FragmentClients { clients };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

async fn fragment_delete_client(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<axum::response::Html<String>, StatusCode> {
    services::delete_client(&state.repo, &name).await.map_err(app_error_to_status)?;
    let clients = services::list_clients(&state.repo).await.map_err(app_error_to_status)?;
    let tmpl = FragmentClients { clients };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

#[derive(Template)]
#[template(path = "fragments/admin/dev_mode.html")]
struct FragmentDevMode {
    enabled: bool,
}

async fn fragment_dev_mode(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, StatusCode> {
    let enabled = services::get_dev_mode(&state.repo).await.map_err(app_error_to_status)?;
    let tmpl = FragmentDevMode { enabled };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

async fn fragment_dev_mode_toggle(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, StatusCode> {
    let current = services::get_dev_mode(&state.repo).await.map_err(app_error_to_status)?;
    services::set_dev_mode(&state.repo, !current).await.map_err(app_error_to_status)?;
    let tmpl = FragmentDevMode { enabled: !current };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

#[derive(Template)]
#[template(path = "fragments/admin/local_users.html")]
struct FragmentLocalUsers {
    enabled: bool,
}

async fn fragment_local_users(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, StatusCode> {
    let enabled = services::get_local_users_enabled(&state.repo).await.map_err(app_error_to_status)?;
    let tmpl = FragmentLocalUsers { enabled };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

async fn fragment_local_users_toggle(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, StatusCode> {
    let current = services::get_local_users_enabled(&state.repo).await.map_err(app_error_to_status)?;
    services::set_local_users_enabled(&state.repo, !current).await.map_err(app_error_to_status)?;
    let tmpl = FragmentLocalUsers { enabled: !current };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

#[derive(Template)]
#[template(path = "fragments/admin/database_info.html")]
struct FragmentDatabaseInfo {
    backend: String,
    url: String,
}

async fn fragment_database_info(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, StatusCode> {
    let masked_url = mask_database_url(&state.db_url);
    let tmpl = FragmentDatabaseInfo {
        backend: state.repo.backend_name().to_string(),
        url: masked_url,
    };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

#[derive(Template)]
#[template(path = "fragments/admin/protected_branches.html")]
struct FragmentProtectedBranches {
    patterns: Vec<String>,
}

async fn fragment_protected_branches(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, StatusCode> {
    let patterns = services::list_protected_branches(&state.repo).await.map_err(app_error_to_status)?;
    let tmpl = FragmentProtectedBranches { patterns };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

#[derive(Deserialize)]
struct FragmentAddBranchPattern {
    pattern: String,
}

async fn fragment_add_protected_branch(
    State(state): State<AppState>,
    axum::extract::Form(payload): axum::extract::Form<FragmentAddBranchPattern>,
) -> Result<axum::response::Html<String>, StatusCode> {
    if !payload.pattern.is_empty() {
        services::add_protected_branch(&state.repo, &payload.pattern).await.map_err(app_error_to_status)?;
    }
    let patterns = services::list_protected_branches(&state.repo).await.map_err(app_error_to_status)?;
    let tmpl = FragmentProtectedBranches { patterns };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

async fn fragment_delete_protected_branch(
    State(state): State<AppState>,
    axum::extract::Path(pattern): axum::extract::Path<String>,
) -> Result<axum::response::Html<String>, StatusCode> {
    services::remove_protected_branch(&state.repo, &pattern).await.map_err(app_error_to_status)?;
    let patterns = services::list_protected_branches(&state.repo).await.map_err(app_error_to_status)?;
    let tmpl = FragmentProtectedBranches { patterns };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

#[derive(Template)]
#[template(path = "fragments/admin/auth_config.html")]
struct FragmentAuthConfig {
    auth_mode: String,
    ldap_server_url: String,
    ldap_bind_dn: String,
    ldap_bind_password: String,
    ldap_base_dn: String,
    ldap_user_filter: String,
    ldap_admin_group: String,
}

async fn fragment_auth_config(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, StatusCode> {
    let mode = services::get_auth_mode(&state.repo).await.map_err(app_error_to_status)?;
    let ldap = services::get_ldap_config(&state.repo).await.map_err(app_error_to_status)?;
    let tmpl = FragmentAuthConfig {
        auth_mode: mode.as_str().to_string(),
        ldap_server_url: ldap.as_ref().map(|c| c.server_url.clone()).unwrap_or_default(),
        ldap_bind_dn: ldap.as_ref().map(|c| c.bind_dn.clone()).unwrap_or_default(),
        ldap_bind_password: ldap.as_ref().and_then(|c| c.bind_password.as_ref()).map(|_| "****".to_string()).unwrap_or_default(),
        ldap_base_dn: ldap.as_ref().map(|c| c.base_dn.clone()).unwrap_or_default(),
        ldap_user_filter: ldap.as_ref().map(|c| c.user_filter.clone()).unwrap_or_default(),
        ldap_admin_group: ldap.as_ref().map(|c| c.admin_group.clone()).unwrap_or_default(),
    };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

#[derive(Template)]
#[template(path = "fragments/admin/branch_max_age.html")]
struct FragmentBranchMaxAge {
    days: u64,
}

async fn fragment_branch_max_age(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, StatusCode> {
    let days = services::get_branch_max_age_days(&state.repo).await.map_err(app_error_to_status)?;
    let tmpl = FragmentBranchMaxAge { days };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

#[derive(Deserialize)]
struct FragmentMaxAgeDays {
    days: u64,
}

async fn fragment_set_branch_max_age(
    State(state): State<AppState>,
    axum::extract::Form(payload): axum::extract::Form<FragmentMaxAgeDays>,
) -> Result<axum::response::Html<String>, StatusCode> {
    services::set_branch_max_age_days(&state.repo, payload.days).await.map_err(app_error_to_status)?;
    let tmpl = FragmentBranchMaxAge { days: payload.days };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

async fn fragment_branch_cleanup(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, StatusCode> {
    let deleted = services::cleanup_stale_branches(&state.repo).await.map_err(app_error_to_status)?;
    Ok(axum::response::Html(format!("Deleted {} stale branches", deleted)))
}

#[derive(Template)]
#[template(path = "fragments/admin/dependency_max_age.html")]
struct FragmentDependencyMaxAge {
    days: u64,
}

async fn fragment_dependency_max_age(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, StatusCode> {
    let days = services::get_dependency_max_age_days(&state.repo).await.map_err(app_error_to_status)?;
    let tmpl = FragmentDependencyMaxAge { days };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

async fn fragment_set_dependency_max_age(
    State(state): State<AppState>,
    axum::extract::Form(payload): axum::extract::Form<FragmentMaxAgeDays>,
) -> Result<axum::response::Html<String>, StatusCode> {
    services::set_dependency_max_age_days(&state.repo, payload.days).await.map_err(app_error_to_status)?;
    let tmpl = FragmentDependencyMaxAge { days: payload.days };
    Ok(axum::response::Html(tmpl.render().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?))
}

async fn fragment_dependency_cleanup(
    State(state): State<AppState>,
) -> Result<axum::response::Html<String>, StatusCode> {
    let deleted = services::cleanup_stale_dependencies(&state.repo).await.map_err(app_error_to_status)?;
    Ok(axum::response::Html(format!("Deleted {} stale dependencies", deleted)))
}

async fn request_counter(
    State(state): State<AppState>,
    req: axum::http::Request<Body>,
    next: middleware::Next,
) -> axum::response::Response {
    let path = req.uri().path();
    let is_business = path == "/provide"
        || path == "/require"
        || path == "/require-bundle"
        || path == "/report"
        || path == "/report/markdown"
        || path == "/endpoint-versions";

    if is_business {
        state.requests_total.fetch_add(1, Ordering::Relaxed);
    }

    let response = next.run(req).await;
    if is_business && response.status().is_server_error() {
        state.failures_total.fetch_add(1, Ordering::Relaxed);
    }
    response
}

async fn admin_get_logs(State(state): State<AppState>) -> Json<Vec<domain::models::LogEntry>> {
    let logs = state.log_buffer.lock().unwrap();
    Json(logs.iter().cloned().collect())
}

async fn admin_get_stats(State(state): State<AppState>) -> Json<domain::models::SystemStats> {
    let mut sys = System::new_with_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::everything())
            .with_memory(MemoryRefreshKind::everything()),
    );
    sys.refresh_all();

    Json(domain::models::SystemStats {
        cpu_usage: sys.global_cpu_usage(),
        memory_used: sys.used_memory(),
        memory_total: sys.total_memory(),
        system_uptime: System::uptime(),
        process_uptime: (Utc::now() - state.process_start_time).num_seconds() as u64,
        requests_total: state.requests_total.load(Ordering::Relaxed),
        failures_total: state.failures_total.load(Ordering::Relaxed),
    })
}

async fn admin_get_debug_config(State(state): State<AppState>) -> Json<domain::models::DebugConfig> {
    Json(domain::models::DebugConfig {
        business_logic_debug: state.business_logic_debug.load(Ordering::Relaxed),
        admin_user_debug: state.admin_user_debug.load(Ordering::Relaxed),
    })
}

async fn admin_set_debug_config(
    State(state): State<AppState>,
    Json(payload): Json<domain::models::DebugConfig>,
) -> StatusCode {
    state.business_logic_debug.store(payload.business_logic_debug, Ordering::Relaxed);
    state.admin_user_debug.store(payload.admin_user_debug, Ordering::Relaxed);
    StatusCode::OK
}
