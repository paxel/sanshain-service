use axum_prometheus::PrometheusMetricLayer;
use sqlx::sqlite::{SqlitePoolOptions, SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::collections::{HashMap, VecDeque};
use std::str::FromStr;
use chrono::Utc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use sanshain_service::{create_app, AppState, LogCaptureLayer};
use sanshain_service::application::services;
use sanshain_service::infrastructure::database::DatabaseRepo;
use sanshain_service::infrastructure::sqlite_repository::SqliteSpecRepository;
use sanshain_service::infrastructure::postgres_repository::PostgresSpecRepository;

#[tokio::main]
pub async fn main() {
    let business_logic_debug = Arc::new(AtomicBool::new(false));
    let admin_user_debug = Arc::new(AtomicBool::new(false));
    let requests_total = Arc::new(AtomicU64::new(0));
    let failures_total = Arc::new(AtomicU64::new(0));
    let error_buffer = Arc::new(std::sync::Mutex::new(VecDeque::with_capacity(101)));
    let warn_buffer = Arc::new(std::sync::Mutex::new(VecDeque::with_capacity(101)));
    let info_buffer = Arc::new(std::sync::Mutex::new(VecDeque::with_capacity(101)));
    let debug_buffer = Arc::new(std::sync::Mutex::new(VecDeque::with_capacity(101)));

    let capture_layer = LogCaptureLayer {
        error_buffer: error_buffer.clone(),
        warn_buffer: warn_buffer.clone(),
        info_buffer: info_buffer.clone(),
        debug_buffer: debug_buffer.clone(),
        business_logic_debug: business_logic_debug.clone(),
        admin_user_debug: admin_user_debug.clone(),
    };

    use tracing_subscriber::Layer;
    let stdout_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "sanshain_service=info,tower_http=info".into());
    
    // We want the capture layer to see DEBUG logs so they can be toggled at runtime,
    // but we keep stdout at INFO by default to avoid noise.
    let capture_filter = tracing_subscriber::EnvFilter::new("sanshain_service=debug,tower_http=debug");

    let registry = tracing_subscriber::registry()
        .with(capture_layer.with_filter(capture_filter));

    if std::env::var("LOG_FORMAT").unwrap_or_default() == "json" {
        registry.with(tracing_subscriber::fmt::layer().json().with_filter(stdout_filter)).init();
    } else {
        registry.with(tracing_subscriber::fmt::layer().with_filter(stdout_filter)).init();
    }

    let db_connection_str = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite:sanshain.db?mode=rwc".into());

    let repo = if db_connection_str.starts_with("postgres://") || db_connection_str.starts_with("postgresql://") {
        let pool = match PgPoolOptions::new()
            .max_connections(5)
            .connect(&db_connection_str)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("Can't connect to PostgreSQL database: {}", e);
                eprintln!("ERROR: Can't connect to PostgreSQL database: {}", e);
                std::process::exit(1);
            }
        };
        let pg_repo = PostgresSpecRepository::new(pool);
        if let Err(e) = pg_repo.run_migrations().await {
            tracing::error!("Can't run PostgreSQL migrations: {}", e);
            eprintln!("ERROR: Can't run PostgreSQL migrations: {}", e);
            std::process::exit(1);
        }
        tracing::info!("Using PostgreSQL database backend");
        DatabaseRepo::Postgres(pg_repo)
    } else {
        let connection_options = match SqliteConnectOptions::from_str(&db_connection_str) {
            Ok(opts) => opts,
            Err(e) => {
                tracing::error!("Invalid database URL: {}", e);
                eprintln!("ERROR: Invalid database URL: {}", e);
                std::process::exit(1);
            }
        }
        .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(5))
            .synchronous(SqliteSynchronous::Normal);

        let pool = match SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(connection_options)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("Can't connect to SQLite database: {}", e);
                eprintln!("ERROR: Can't connect to SQLite database: {}", e);
                std::process::exit(1);
            }
        };
        let sqlite_repo = SqliteSpecRepository::new(pool);
        if let Err(e) = sqlite_repo.run_migrations().await {
            tracing::error!("Can't run SQLite migrations: {}", e);
            eprintln!("ERROR: Can't run SQLite migrations: {}", e);
            std::process::exit(1);
        }
        tracing::info!("Using SQLite database backend");
        DatabaseRepo::Sqlite(sqlite_repo)
    };

    // Ensure initial admin user exists
    if let Err(e) = services::ensure_initial_admin(&repo).await {
        tracing::error!("Can't create initial admin: {}", e);
        eprintln!("ERROR: Can't create initial admin: {}", e);
        std::process::exit(1);
    }

    let instance_id = uuid::Uuid::new_v4().to_string();
    tracing::info!("Instance ID: {}", instance_id);

    let (prometheus_layer, prometheus_handle) = PrometheusMetricLayer::pair();
    let (spec_updated_tx, _) = tokio::sync::broadcast::channel(100);
    let state = AppState {
        repo,
        db_url: db_connection_str,
        csrf_tokens: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        instance_id,
        spec_updated_tx,
        error_buffer,
        warn_buffer,
        info_buffer,
        debug_buffer,
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
                let max_age = chrono::Duration::hours(24);
                let before_count = tokens.len();
                tokens.retain(|_, created_at| now - *created_at < max_age);
                let after_count = tokens.len();
                if before_count > after_count {
                    tracing::info!("CSRF cleanup: pruned {} expired tokens", before_count - after_count);
                }
            }
        }
    });

    let app = create_app(state).layer(prometheus_layer);

    let bind_address = std::env::var("BIND_ADDRESS").unwrap_or_else(|_| "0.0.0.0:3000".into());
    let addr: std::net::SocketAddr = match bind_address.parse() {
        Ok(a) => a,
        Err(e) => {
            tracing::error!("Invalid bind address '{}': {}", bind_address, e);
            eprintln!("ERROR: Invalid bind address '{}': {}", bind_address, e);
            std::process::exit(1);
        }
    };
    tracing::info!("Listening on {}", addr);
    
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to bind to {}: {}", addr, e);
            if e.kind() == std::io::ErrorKind::AddrInUse {
                eprintln!("ERROR: Address {} already in use. Another instance might be running.", addr);
            } else {
                eprintln!("ERROR: Failed to bind to {}: {}", addr, e);
            }
            std::process::exit(1);
        }
    };

    if let Err(e) = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
    {
        tracing::error!("Server error: {}", e);
        eprintln!("ERROR: Server error: {}", e);
        std::process::exit(1);
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!("failed to install Ctrl+C handler: {}", e);
            eprintln!("ERROR: failed to install Ctrl+C handler: {}", e);
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => {
                tracing::error!("failed to install signal handler: {}", e);
                eprintln!("ERROR: failed to install signal handler: {}", e);
                // Fallback to pending if we can't install the handler
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("signal received, starting graceful shutdown");
}
