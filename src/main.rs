use axum_prometheus::PrometheusMetricLayer;
use chrono::Utc;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use std::collections::{HashMap, VecDeque};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use sanshain_service::application::services;
use sanshain_service::infrastructure::cached_repository::CachedSpecRepository;
use sanshain_service::infrastructure::database::DatabaseRepo;
use sanshain_service::infrastructure::postgres_repository::PostgresSpecRepository;
use sanshain_service::infrastructure::sqlite_repository::SqliteSpecRepository;
use sanshain_service::infrastructure::telemetry;
use sanshain_service::{AppState, LogCaptureLayer, create_app};

/// Read a numeric setting, refusing to start when the value is not one.
///
/// These were all parsed with `.ok().unwrap_or(default)`, so `LOG_BUFFER_SIZE=1OO`
/// silently became 100 and the operator believed a setting was active that the
/// service had ignored — the exact complaint behind `ai/improvements.md` #15. A
/// container that will not start is far cheaper to diagnose than one running
/// with settings nobody chose. Called before tracing is initialised, so the
/// message goes to stderr where a container runtime will surface it.
fn env_number<T>(name: &str, default: T) -> T
where
    T: std::str::FromStr + std::fmt::Display,
{
    match std::env::var(name) {
        Err(_) => default,
        Ok(raw) => match raw.trim().parse::<T>() {
            Ok(value) => value,
            Err(_) => {
                eprintln!(
                    "CONFIGURATION ERROR: {name}={raw:?} is not a valid number. \
                     Correct it or unset it to use the default ({default})."
                );
                std::process::exit(1);
            }
        },
    }
}

/// Same contract for a setting with a fixed vocabulary: an unrecognised value
/// is refused rather than quietly treated as "not the special one".
fn env_choice(name: &str, allowed: &[&str], default: &str) -> String {
    match std::env::var(name) {
        Err(_) => default.to_string(),
        Ok(raw) => {
            let value = raw.trim().to_ascii_lowercase();
            if allowed.contains(&value.as_str()) {
                value
            } else {
                eprintln!(
                    "CONFIGURATION ERROR: {name}={raw:?} is not recognised. \
                     Valid values: {}. Unset it to use the default ({default}).",
                    allowed.join(", ")
                );
                std::process::exit(1);
            }
        }
    }
}

#[tokio::main]
pub async fn main() {
    let log_buffer_size = env_number::<usize>("LOG_BUFFER_SIZE", 100);

    let business_logic_debug = Arc::new(AtomicBool::new(false));
    let admin_user_debug = Arc::new(AtomicBool::new(false));
    let requests_total = Arc::new(AtomicU64::new(0));
    let failures_total = Arc::new(AtomicU64::new(0));
    let error_buffer = Arc::new(std::sync::Mutex::new(VecDeque::with_capacity(
        log_buffer_size + 1,
    )));
    let warn_buffer = Arc::new(std::sync::Mutex::new(VecDeque::with_capacity(
        log_buffer_size + 1,
    )));
    let info_buffer = Arc::new(std::sync::Mutex::new(VecDeque::with_capacity(
        log_buffer_size + 1,
    )));
    let debug_buffer = Arc::new(std::sync::Mutex::new(VecDeque::with_capacity(
        log_buffer_size + 1,
    )));

    let capture_layer = LogCaptureLayer {
        max_size: log_buffer_size,
        error_buffer: error_buffer.clone(),
        warn_buffer: warn_buffer.clone(),
        info_buffer: info_buffer.clone(),
        debug_buffer: debug_buffer.clone(),
        business_logic_debug: business_logic_debug.clone(),
        admin_user_debug: admin_user_debug.clone(),
    };

    use tracing_subscriber::Layer;
    let stdout_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "sanshain_service=info,tower_http=warn".into());

    // We want the capture layer to see DEBUG logs so they can be toggled at runtime,
    // but we keep stdout at INFO by default to avoid noise.
    let capture_log_filter = std::env::var("CAPTURE_LOG_FILTER")
        .unwrap_or_else(|_| "sanshain_service=debug,tower_http=debug".into());
    let capture_filter = tracing_subscriber::EnvFilter::new(capture_log_filter);

    let otel_enabled = env_choice("OTEL_ENABLED", &["true", "false"], "false") == "true";
    let (otel_layer, otel_provider) = if otel_enabled {
        match telemetry::init_tracer() {
            Ok((layer, provider)) => (Some(layer), Some(provider)),
            Err(e) => {
                // The tracing subscriber is not initialized yet, so stderr is
                // the only reliable channel here.
                eprintln!("ERROR: Failed to initialize OpenTelemetry tracing: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        (None, None)
    };

    let registry = tracing_subscriber::registry()
        .with(capture_layer.with_filter(capture_filter))
        .with(otel_layer);

    if env_choice("LOG_FORMAT", &["json", "text"], "text") == "json" {
        registry
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_filter(stdout_filter),
            )
            .init();
    } else {
        registry
            .with(tracing_subscriber::fmt::layer().with_filter(stdout_filter))
            .init();
    }

    // Build the outbound TLS trust store, including any CA certificates the
    // operator mounted. Runs before the listener binds and so before any LDAPS
    // connection, and is fatal on failure: starting with a trust store that
    // silently lacks the mounted certificates is the failure this prevents.
    if let Err(e) = sanshain_service::infrastructure::tls::init_from_env() {
        tracing::error!("Can't build the TLS trust store: {}", e);
        eprintln!("ERROR: Can't build the TLS trust store: {}", e);
        std::process::exit(1);
    }

    let db_connection_str =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:sanshain.db?mode=rwc".into());

    let repo = if db_connection_str.starts_with("postgres://")
        || db_connection_str.starts_with("postgresql://")
    {
        let max_connections = env_number::<u32>("MAX_POSTGRES_CONNECTIONS", 20);

        let pool = match PgPoolOptions::new()
            .max_connections(max_connections)
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
        let sqlite_busy_timeout_ms = env_number::<u64>("SQLITE_BUSY_TIMEOUT_MS", 5000);

        let connection_options = match SqliteConnectOptions::from_str(&db_connection_str) {
            Ok(opts) => opts,
            Err(e) => {
                tracing::error!("Invalid database URL: {}", e);
                eprintln!("ERROR: Invalid database URL: {}", e);
                std::process::exit(1);
            }
        }
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_millis(sqlite_busy_timeout_ms))
        .synchronous(SqliteSynchronous::Normal);

        // WAL mode supports concurrent readers beside one writer; a pool of 1
        // would serialize every query. Write transactions use BEGIN IMMEDIATE
        // (see SqliteSpecRepository::write_tx) so writers queue on busy_timeout
        // rather than fail once the pool exceeds one connection.
        let max_connections = env_number::<u32>("MAX_SQLITE_CONNECTIONS", 5);

        let pool = match SqlitePoolOptions::new()
            .max_connections(max_connections)
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

    // Wrap with in-memory cache layer
    let cache_memory_mb = env_number::<u64>("CACHE_MEMORY_MB", 256);
    let repo = CachedSpecRepository::new(repo, cache_memory_mb);
    if cache_memory_mb > 0 {
        tracing::info!("In-memory cache enabled with {} MB limit", cache_memory_mb);
    } else {
        tracing::info!("In-memory cache disabled (CACHE_MEMORY_MB=0)");
    }

    // Ensure initial admin user exists
    if let Err(e) = services::ensure_initial_admin(&repo).await {
        tracing::error!("Can't create initial admin: {}", e);
        eprintln!("ERROR: Can't create initial admin: {}", e);
        std::process::exit(1);
    }

    // Carry a previously configured directory admin group into the group
    // mapping. Best effort: an instance that has never used the directory has
    // nothing to migrate, and failing to migrate must not stop the service from
    // starting — it would leave the operator with neither the old behaviour nor
    // a running instance.
    if let Err(e) = services::migrate_stored_admin_group(&repo).await {
        tracing::warn!(
            "Could not carry the configured directory admin group into the group mapping: {}",
            e
        );
    }

    // Dev mode lets any caller reach protected endpoints without a token and
    // must never be left enabled in production. It only takes effect when
    // explicitly requested AND permitted by the `ALLOW_INSECURE_DEV_MODE` safety
    // gate; a requested-but-ungated dev mode fails closed so configuration drift
    // cannot silently disable authentication.
    let dev_mode_requested = services::is_dev_mode_requested(&repo)
        .await
        .unwrap_or(false);
    let dev_user = if dev_mode_requested && services::dev_mode_gate_open() {
        tracing::warn!(
            "SECURITY WARNING: dev_mode is ENABLED - API endpoints accept unauthenticated requests. Disable it in production."
        );
        eprintln!(
            "SECURITY WARNING: dev_mode is ENABLED - API endpoints accept unauthenticated requests. Disable it in production."
        );
        match services::ensure_dev_user(&repo).await {
            Ok(user) => Some(user),
            Err(e) => {
                tracing::warn!("Could not ensure dev_user in database: {}", e);
                None
            }
        }
    } else {
        if dev_mode_requested {
            tracing::error!(
                "SECURITY: dev_mode is requested (SANSHAIN_DEV_MODE or persisted dev_mode setting) but the ALLOW_INSECURE_DEV_MODE safety gate is not set to 'true'. Refusing to enable dev mode - authentication stays enforced. Set ALLOW_INSECURE_DEV_MODE=true only on a trusted local machine to enable it."
            );
            eprintln!(
                "SECURITY: dev_mode is requested but the ALLOW_INSECURE_DEV_MODE safety gate is not set to 'true'. Refusing to enable dev mode - authentication stays enforced."
            );
        }
        None
    };

    let instance_id =
        std::env::var("INSTANCE_ID").unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
    tracing::info!("Instance ID: {}", instance_id);

    let (prometheus_layer, prometheus_handle) = PrometheusMetricLayer::pair();

    let max_body_bytes = env_number::<usize>(
        "MAX_SPEC_BODY_BYTES",
        sanshain_service::DEFAULT_MAX_BODY_BYTES,
    );
    if max_body_bytes == 0 {
        eprintln!("CONFIGURATION ERROR: MAX_SPEC_BODY_BYTES=0 would reject every request.");
        std::process::exit(1);
    }
    tracing::info!("Maximum request body size: {} bytes", max_body_bytes);

    let spec_updated_channel_size = env_number::<usize>("SPEC_UPDATED_CHANNEL_SIZE", 100);

    // Root is resolved from configuration and never stored, so no stored grant
    // can revoke it. Falling back to the bootstrap administrator's name keeps an
    // existing deployment with a root account without being reconfigured.
    let root_users =
        std::sync::Arc::new(sanshain_service::domain::permissions::RootUsers::resolve(
            std::env::var("SANSHAIN_ROOT_USERS").ok().as_deref(),
            std::env::var("INITIAL_ADMIN_USERNAME").ok().as_deref(),
        ));
    tracing::info!(
        "Root users (configuration-held, cannot be revoked in-app): {}",
        root_users.usernames().collect::<Vec<_>>().join(", ")
    );
    // Root authority is a username match, so every root name must be claimed
    // by an account before anyone else can claim it.
    if let Err(e) = services::claim_root_usernames(&repo, &root_users).await {
        tracing::error!("Can't claim root usernames: {}", e);
        eprintln!("ERROR: Can't claim root usernames: {}", e);
        std::process::exit(1);
    }

    let (spec_updated_tx, _) = tokio::sync::broadcast::channel(spec_updated_channel_size);
    let mut system = sysinfo::System::new_all();
    system.refresh_all();

    let state = AppState {
        repo,
        db_url: db_connection_str,
        dev_user,
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
        system: Arc::new(std::sync::Mutex::new(system)),
        max_body_bytes,
        root_users,
        directory_roles:
            sanshain_service::application::directory_roles::DirectoryRoleCache::from_env(),
    };

    // Spawn background cleanup task (snapshots, dependencies, CSRF tokens)
    let cleanup_repo = state.repo.clone();
    let cleanup_csrf = state.csrf_tokens.clone();

    let cleanup_interval_secs = env_number::<u64>("CLEANUP_INTERVAL_SECS", 3600);

    let csrf_max_age_hours = env_number::<i64>("CSRF_MAX_AGE_HOURS", 24);

    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(cleanup_interval_secs));
        loop {
            interval.tick().await;
            match services::cleanup_expired_snapshots(&cleanup_repo).await {
                Ok(0) => {}
                Ok(n) => tracing::info!("Snapshot cleanup: deleted {} unused snapshots", n),
                Err(e) => tracing::warn!("Snapshot cleanup failed: {:?}", e),
            }
            match services::cleanup_stale_dependencies(&cleanup_repo).await {
                Ok(0) => {}
                Ok(n) => tracing::info!("Dependency cleanup: pruned {} stale dependencies", n),
                Err(e) => tracing::warn!("Dependency cleanup failed: {:?}", e),
            }
            match services::cleanup_stale_trunk_data(&cleanup_repo).await {
                Ok(0) => {}
                Ok(n) => tracing::info!("Trunk cleanup: closed {} stale trunk pins", n),
                Err(e) => tracing::warn!("Trunk cleanup failed: {:?}", e),
            }
            match services::cleanup_expired_credentials(&cleanup_repo).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(
                    "Credential cleanup: removed {} expired sessions and API tokens",
                    n
                ),
                Err(e) => tracing::warn!("Credential cleanup failed: {:?}", e),
            }

            // Prune expired CSRF tokens
            {
                let mut tokens = cleanup_csrf.write().await;
                let now = Utc::now();
                let max_age = chrono::Duration::hours(csrf_max_age_hours);
                let before_count = tokens.len();
                tokens.retain(|_, created_at| now - *created_at < max_age);
                let after_count = tokens.len();
                if before_count > after_count {
                    tracing::info!(
                        "CSRF cleanup: pruned {} expired tokens",
                        before_count - after_count
                    );
                }
            }
        }
    });

    let app = create_app(state).layer(prometheus_layer);

    let bind_address = std::env::var("BIND_ADDRESS").unwrap_or_else(|_| "0.0.0.0:3000".into());
    if bind_address.starts_with("0.0.0.0") || bind_address.starts_with("[::]") {
        tracing::warn!(
            "SECURITY WARNING: binding to all interfaces ({}). Set BIND_ADDRESS=127.0.0.1:3000 to restrict access to localhost.",
            bind_address
        );
        eprintln!(
            "SECURITY WARNING: binding to all interfaces ({}). Set BIND_ADDRESS=127.0.0.1:3000 to restrict access to localhost.",
            bind_address
        );
    }
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
                eprintln!(
                    "ERROR: Address {} already in use. Another instance might be running.",
                    addr
                );
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

    if let Some(provider) = otel_provider {
        telemetry::shutdown_telemetry(provider);
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
