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
        csrf_tokens: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
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
    let addr: std::net::SocketAddr = bind_address.parse().expect("invalid bind address");
    tracing::info!("Listening on {}", addr);
    
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("signal received, starting graceful shutdown");
}
