pub mod application;
pub mod asyncapi;
pub mod domain;
pub mod infrastructure;
pub mod openapi;
pub mod presentation;
pub mod proto;

use axum::{
    Router,
    extract::State,
    http::{HeaderValue, header},
    middleware::from_fn_with_state,
    routing::{delete, get, post},
};
use chrono::{DateTime, Utc};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};
use tokio::sync::RwLock;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

use domain::models::LogEntry;
use infrastructure::cached_repository::CachedSpecRepository;

pub use presentation::handlers::{admin, api, auth, pages};
pub use presentation::middleware::{
    LogCaptureLayer, admin_auth, api_auth, authenticated_auth, validate_csrf,
};

#[derive(Clone)]
pub struct AppState {
    pub repo: CachedSpecRepository,
    pub db_url: String,
    pub csrf_tokens: Arc<RwLock<HashMap<String, DateTime<Utc>>>>,
    pub instance_id: String,
    pub spec_updated_tx: tokio::sync::broadcast::Sender<()>,
    pub error_buffer: Arc<std::sync::Mutex<VecDeque<LogEntry>>>,
    pub warn_buffer: Arc<std::sync::Mutex<VecDeque<LogEntry>>>,
    pub info_buffer: Arc<std::sync::Mutex<VecDeque<LogEntry>>>,
    pub debug_buffer: Arc<std::sync::Mutex<VecDeque<LogEntry>>>,
    pub business_logic_debug: Arc<AtomicBool>,
    pub admin_user_debug: Arc<AtomicBool>,
    pub requests_total: Arc<AtomicU64>,
    pub failures_total: Arc<AtomicU64>,
    pub process_start_time: DateTime<Utc>,
    pub prometheus_handle: metrics_exporter_prometheus::PrometheusHandle,
    pub system: Arc<std::sync::Mutex<sysinfo::System>>,
}

pub fn create_app(state: AppState) -> Router {
    Router::new()
        // High-priority unique paths
        .route("/tokens", get(auth::list_tokens).post(auth::create_token).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/tokens/{id}", delete(auth::revoke_token).layer(from_fn_with_state(state.clone(), authenticated_auth)))

        // API
        .route("/provide", post(api::provide).layer(from_fn_with_state(state.clone(), api_auth)))
        .route("/provide/asyncapi", post(api::provide_asyncapi).layer(from_fn_with_state(state.clone(), api_auth)))
        .route("/provide/grpc", post(api::provide_proto).layer(from_fn_with_state(state.clone(), api_auth)))
        .route("/require", get(api::require).layer(from_fn_with_state(state.clone(), api_auth)))
        .route("/require/asyncapi", get(api::require_asyncapi).layer(from_fn_with_state(state.clone(), api_auth)))
        .route("/require/grpc", get(api::require_proto).layer(from_fn_with_state(state.clone(), api_auth)))
        .route("/require-bundle", post(api::require_bundle).layer(from_fn_with_state(state.clone(), api_auth)))
        .route("/report", get(api::report).layer(from_fn_with_state(state.clone(), api_auth)))
        .route("/report/markdown", get(api::report_markdown).layer(from_fn_with_state(state.clone(), api_auth)))
        .route("/report/isolation", get(api::report_isolation).layer(from_fn_with_state(state.clone(), api_auth)))
        .route("/report/merged", get(api::report_merged).layer(from_fn_with_state(state.clone(), api_auth)))
        .route("/branches/protected", get(api::list_protected_branches_public).layer(from_fn_with_state(state.clone(), api_auth)))
        .route("/endpoint-versions", get(api::endpoint_versions).layer(from_fn_with_state(state.clone(), api_auth)))

        // Admin (Flat list to avoid double nesting issues)
        .route("/admin/protected-branches", get(admin::list_protected_branches).post(admin::add_protected_branch).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/protected-branches/{pattern}", delete(admin::delete_protected_branch).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/services", get(admin::admin_list_services).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/admin/services/{name}", delete(admin::admin_delete_service).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/services/{name}/branches", get(admin::admin_list_branches).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/admin/services/{name}/branches/{branch}", delete(admin::admin_delete_branch).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/services/{name}/branches/{branch}/reset-history", post(admin::admin_reset_branch_history).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/services/{name}/branches/{branch}/endpoints", get(admin::admin_list_service_endpoints).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/admin/clients", get(admin::admin_list_clients).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/admin/clients/{name}", delete(admin::admin_delete_client).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/clients/{name}/branches", get(admin::admin_list_client_branches).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/admin/clients/{name}/branches/{branch}/endpoints", get(admin::admin_list_client_endpoints).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/admin/endpoint-yaml", get(admin::admin_get_endpoint_yaml).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/admin/endpoint-versions", get(admin::admin_get_endpoint_versions).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/admin/shared-contract", get(admin::admin_get_shared_contract).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/admin/settings/dev-mode", get(admin::get_dev_mode).post(admin::set_dev_mode).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/settings/local-users", get(admin::get_local_users).post(admin::set_local_users).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/settings/auto-approve", get(admin::get_auto_approve_users).post(admin::set_auto_approve_users).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/auth-config", get(admin::get_auth_config).put(admin::set_auth_config).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/auth-config/test", post(admin::test_auth_config).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/nuke/database", post(admin::admin_nuke_database).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/nuke/services", post(admin::admin_nuke_services).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/nuke/clients", post(admin::admin_nuke_clients).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/nuke/users", post(admin::admin_nuke_users).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/nuke/branch/{branch}", post(admin::admin_nuke_branch).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/settings/branch-max-age", get(admin::get_branch_max_age).post(admin::set_branch_max_age).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/cleanup/branches", post(admin::trigger_branch_cleanup).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/settings/branch-cleanup", post(admin::trigger_branch_cleanup).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/settings/dependency-max-age", get(admin::get_dependency_max_age).post(admin::set_dependency_max_age).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/cleanup/dependencies", post(admin::trigger_dependency_cleanup).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/settings/dependency-cleanup", post(admin::trigger_dependency_cleanup).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/users", get(admin::admin_list_users).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/users/{id}/approve", post(admin::admin_approve_user).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/users/{id}", delete(admin::admin_delete_user_handler).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/settings/cache", get(admin::get_cache_config).post(admin::set_cache_config).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/cache/clear", post(admin::clear_cache).layer(from_fn_with_state(state.clone(), admin_auth)))

        // Observability (GET routes accessible to any authenticated user, POST requires admin)
        .route("/admin/observability/stats", get(admin::get_observability_stats).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/admin/observability/logs", get(admin::get_observability_logs).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/admin/observability/debug-config", get(admin::get_debug_config).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/admin/observability/debug-config-update", post(admin::set_debug_config).layer(from_fn_with_state(state.clone(), admin_auth)))
        .route("/admin/observability/audit-logs", get(admin::get_observability_audit_logs).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/admin/observability/audit-logs/export", get(admin::export_audit_logs_csv).layer(from_fn_with_state(state.clone(), authenticated_auth)))

        // Auth (Mixed prefix)
        .route("/auth/login", post(auth::auth_login))
        .route("/auth/logout", post(auth::auth_logout))
        .route("/auth/register", post(auth::auth_register))
        .route("/auth/me", get(auth::auth_me).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/auth/change-password", post(auth::auth_change_password).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/auth/tokens", get(auth::list_tokens).post(auth::create_token).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/auth/tokens/{id}", delete(auth::revoke_token).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/csrf-token", get(auth::get_csrf_token))

        // Pages / Root
        .route("/", get(pages::index_page))
        .route("/dashboard", get(pages::dashboard_page).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/health", get(pages::health))
        .route("/metrics", get(pages::metrics))
        .route("/LICENSE", get(pages::license_text))
        .route("/version", get(|State(s): State<AppState>| async move { axum::Json(serde_json::json!({"version": env!("CARGO_PKG_VERSION"), "instance_id": s.instance_id})) }))

        .fallback_service(ServeDir::new(std::env::var("STATIC_DIR").unwrap_or_else(|_| "static".to_string())))
        .layer(from_fn_with_state(state.clone(), validate_csrf))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .layer(tower_http::compression::CompressionLayer::new())
        .layer(tower_http::decompression::RequestDecompressionLayer::new())
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("SAMEORIGIN"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'self'; script-src 'self' 'unsafe-inline' https://cdn.tailwindcss.com https://cdn.jsdelivr.net; style-src 'self' 'unsafe-inline' https://cdn.tailwindcss.com https://cdn.jsdelivr.net; img-src 'self' data:;"),
        ))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    /// Ensures every `/admin/` and `/fragments/admin/` route in `create_app` has
    /// either `admin_auth` or `authenticated_auth` middleware.
    ///
    /// This test reads the source of `lib.rs` at compile time and parses each
    /// `.route(...)` call. If a developer adds a new admin route without attaching
    /// auth middleware, this test will fail — making security-by-default enforceable.
    #[test]
    fn all_admin_routes_have_auth_middleware() {
        let source = include_str!("lib.rs");
        let mut unprotected = Vec::new();

        for line in source.lines() {
            let trimmed = line.trim();
            if !trimmed.starts_with(".route(\"") {
                continue;
            }
            // Extract the path from .route("/some/path", ...)
            let path = trimmed
                .strip_prefix(".route(\"")
                .and_then(|s| s.split('"').next())
                .unwrap_or("");

            let is_admin_route = path.starts_with("/admin/");
            if !is_admin_route {
                continue;
            }

            let has_auth = trimmed.contains("admin_auth") || trimmed.contains("authenticated_auth");
            if !has_auth {
                unprotected.push(path.to_string());
            }
        }

        assert!(
            unprotected.is_empty(),
            "The following admin routes are missing auth middleware: {:?}\n\
             Every /admin/* route must use `admin_auth` or `authenticated_auth`.",
            unprotected
        );
    }

    #[test]
    fn metrics_route_is_registered() {
        let source = include_str!("lib.rs");

        assert!(
            source.contains(".route(\"/metrics\", get(pages::metrics))"),
            "The app router must expose the Prometheus metrics endpoint at /metrics.",
        );
    }
}
