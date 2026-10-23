pub mod application;
pub mod asyncapi;
pub mod backstage;
pub mod descriptions;
pub mod domain;
pub mod infrastructure;
pub mod openapi;
pub mod presentation;
pub mod proto;
pub mod structure;

use axum::{
    Router,
    extract::State,
    http::{HeaderValue, header},
    middleware::from_fn_with_state,
    routing::{delete, get, post, put},
};
use chrono::{DateTime, Utc};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};
use tokio::sync::RwLock;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

use domain::models::LogEntry;
use domain::permissions::Permission;
use infrastructure::cached_repository::CachedSpecRepository;

pub use presentation::handlers::{admin, api, auth, pages, roles, webhooks};
pub use presentation::middleware::{
    LogCaptureLayer, RouteGuard, api_auth, api_auth_write, authenticated_auth, require,
    require_with_api_tokens, validate_csrf,
};

/// Default maximum request body size in bytes (4 MiB). Overridable at startup
/// via the `MAX_SPEC_BODY_BYTES` environment variable.
pub const DEFAULT_MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub repo: CachedSpecRepository,
    pub db_url: String,
    pub dev_user: Option<domain::models::User>,
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
    pub max_body_bytes: usize,
    /// Set while a database snapshot (#50) is being produced or streamed, so
    /// only one runs at a time; a second request is answered `409`.
    pub snapshot_in_progress: Arc<AtomicBool>,
    /// Usernames holding every permission by configuration. Read once at
    /// startup and never stored, so nothing in the database can revoke root.
    pub root_users: Arc<domain::permissions::RootUsers>,
    /// Caches directory group membership, so a change in the directory takes
    /// effect within the cache lifetime rather than at the next login.
    pub directory_roles: application::directory_roles::DirectoryRoleCache,
    /// The MCP endpoint's configuration (https://paxel.github.io/sanshain-service/developer/mcp.html).
    pub mcp: presentation::mcp::McpConfig,
}

/// Content-Security-Policy served with every response (ai/improvements.md #11).
///
/// `script-src 'self'` forbids inline scripts and inline `on*=` event handlers;
/// all behaviour is delegated through the dispatcher in `common.js`. Every
/// third-party library is vendored under `/static/vendor`, so no CDN host is
/// listed. `style-src` retains `'unsafe-inline'` because the vendored Mermaid
/// injects `<style>` elements at diagram-render time — that stays until those
/// can be nonced or hashed. `frame-ancestors 'none'` forbids framing.
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; frame-ancestors 'none';";

pub fn create_app(state: AppState) -> Router {
    Router::new()
        // High-priority unique paths
        .route("/tokens", get(auth::list_tokens).post(auth::create_token).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/tokens/{id}", delete(auth::revoke_token).layer(from_fn_with_state(state.clone(), authenticated_auth)))

        // API. Routes that change the catalog carry `api_auth_write`, which
        // refuses read-only tokens; `write_api_routes_refuse_read_only_tokens`
        // below keeps that list honest.
        .route("/provide", post(api::provide).layer(from_fn_with_state(state.clone(), api_auth_write)))
        .route("/provide/asyncapi", post(api::provide_asyncapi).layer(from_fn_with_state(state.clone(), api_auth_write)))
        .route("/provide/grpc", post(api::provide_proto).layer(from_fn_with_state(state.clone(), api_auth_write)))
        .route("/require", get(api::require).layer(from_fn_with_state(state.clone(), api_auth_write)))
        .route("/require/asyncapi", get(api::require_asyncapi).layer(from_fn_with_state(state.clone(), api_auth_write)))
        .route("/require/grpc", get(api::require_proto).layer(from_fn_with_state(state.clone(), api_auth_write)))
        .route("/require-bundle", post(api::require_bundle).layer(from_fn_with_state(state.clone(), api_auth_write)))
        // #46: a build pushes its Backstage catalog-info.yaml; whoever may
        // require may describe its own periphery.
        .route("/catalog/{systemname}", put(api::push_catalog).layer(from_fn_with_state(state.clone(), api_auth_write)))
        .route("/report", get(api::report).layer(from_fn_with_state(state.clone(), api_auth)))
        .route("/report/markdown", get(api::report_markdown).layer(from_fn_with_state(state.clone(), api_auth)))
        .route("/report/isolation", get(api::report_isolation).layer(from_fn_with_state(state.clone(), api_auth)))
        .route("/producers/{producername}/versions", get(api::producer_versions).layer(from_fn_with_state(state.clone(), api_auth)))
        // Deliberately unauthenticated: the free validator is stateless,
        // touches no data, and is bounded by the body-size limit.
        .route("/validate", post(api::validate))
        .route("/endpoint-versions", get(api::endpoint_versions).layer(from_fn_with_state(state.clone(), api_auth)))
        // The audit trail records the Actor behind every change, across every
        // Producer, so it needs `ViewAudit`. `permission_auth` validates sessions
        // only, so API tokens cannot read it — deliberate, and safe because this
        // route is outside `api.yaml`.
        .route("/api/audit/timeline", get(api::audit_timeline).layer(require(state.clone(), RouteGuard::Global(Permission::ViewAudit))))
        // The landing page's dashboard: numbers for everyone, names only
        // for a signed-in caller.
        .route("/api/dashboard", get(api::dashboard_stats))
        .route("/api/dashboard/recent", get(api::dashboard_recent).layer(from_fn_with_state(state.clone(), api_auth)))
        .route("/api/sse/updates", get(api::sse_updates).layer(from_fn_with_state(state.clone(), api_auth)))
        .route("/api/ws/updates", get(api::ws_updates).layer(from_fn_with_state(state.clone(), api_auth)))

        // Admin (Flat list to avoid double nesting issues)
        .route("/admin/roles", get(roles::list_roles).layer(require(state.clone(), RouteGuard::Global(Permission::ManageRoles))))
        .route("/admin/users/{id}/roles", get(roles::list_user_roles).post(roles::grant_user_role).layer(require(state.clone(), RouteGuard::Global(Permission::ManageRoles))))
        .route("/admin/users/{id}/roles/{role}", delete(roles::revoke_user_role).layer(require(state.clone(), RouteGuard::Global(Permission::ManageRoles))))
        .route("/admin/groups", get(roles::list_groups).post(roles::create_group).layer(require(state.clone(), RouteGuard::Global(Permission::ManageRoles))))
        .route("/admin/groups/{id}", put(roles::update_group).delete(roles::delete_group).layer(require(state.clone(), RouteGuard::Global(Permission::ManageRoles))))
        .route("/admin/groups/{id}/members", post(roles::add_group_member).layer(require(state.clone(), RouteGuard::Global(Permission::ManageRoles))))
        .route("/admin/groups/{id}/members/{user_id}", delete(roles::remove_group_member).layer(require(state.clone(), RouteGuard::Global(Permission::ManageRoles))))
        .route("/admin/maintainers", get(roles::list_all_maintainers).layer(require(state.clone(), RouteGuard::Global(Permission::ManageRoles))))
        // Split so a maintainer can see who shares responsibility for their own
        // Producer, while changing the assignment stays who-may-do-what work.
        .route("/admin/producers/{name}/maintainers", get(roles::list_maintainers).layer(require(state.clone(), RouteGuard::Producer(Permission::ManageProducers))))
        .route("/admin/producers/{name}/maintainers", post(roles::assign_maintainer).layer(require(state.clone(), RouteGuard::Global(Permission::ManageRoles))))
        .route("/admin/producers/{name}/maintainers/users/{user_id}", delete(roles::unassign_user_maintainer).layer(require(state.clone(), RouteGuard::Global(Permission::ManageRoles))))
        .route("/admin/producers/{name}/maintainers/groups/{group_id}", delete(roles::unassign_group_maintainer).layer(require(state.clone(), RouteGuard::Global(Permission::ManageRoles))))
        .route("/admin/users/{id}/maintains", get(roles::list_maintained_producers).layer(require(state.clone(), RouteGuard::Global(Permission::ManageRoles))))
        .route("/admin/producers", get(admin::admin_list_producers).layer(require(state.clone(), RouteGuard::Authenticated)))
        // #48: the Producer's context from its catalog-info, for its page.
        .route("/admin/producers/{name}", get(admin::admin_get_producer).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/producers/{name}", delete(admin::admin_delete_producer).layer(require(state.clone(), RouteGuard::Producer(Permission::ManageProducers))))
        .route("/admin/producers/metadata", post(admin::admin_update_producer_metadata).layer(require(state.clone(), RouteGuard::Global(Permission::ManageProducers))))
        .route("/admin/producers/{name}/versions", get(admin::admin_list_producer_versions).layer(require(state.clone(), RouteGuard::Authenticated)))
        // Delete-version is the sole escape hatch from GA immutability
        // (ADR-0003): admins anywhere, Maintainers for their own Producers.
        .route("/admin/producers/{name}/versions/{api_type}/{version}", delete(admin::admin_delete_version).layer(require(state.clone(), RouteGuard::Producer(Permission::ManageProducers))))
        // Promote declares Authenticated, not Global(ReleaseGa): the release_ga
        // check runs inside the shared GA gate so an unauthorized click gets the
        // instructive 403 + ga_requires_releaser telemetry, which a guard-level
        // refusal could not produce.
        .route("/admin/producers/{name}/versions/{api_type}/{version}/promote", post(admin::admin_promote_version).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/producers/{name}/versions/{api_type}/{version}/dependents", get(admin::admin_get_version_dependents).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/producers/{name}/endpoints", get(admin::admin_list_producer_endpoints).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/producers/{name}/full-spec", get(admin::admin_get_full_spec).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/producers/{name}/diff", get(admin::admin_diff_versions).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/producers/{name}/blame", get(admin::admin_blame_version).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/producers/{name}/structure", get(admin::admin_structure_version).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/producers/{name}/branch-memberships", get(admin::admin_producer_branch_memberships).layer(require(state.clone(), RouteGuard::Authenticated)))
        // Sanshain-branches (ADR-0005): creation is release work (the cut
        // script's endpoint) — releaser-guarded; reading is open to any
        // authenticated caller like the rest of the graph surface.
        .route("/admin/branches", get(admin::admin_list_branches).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/branches", post(admin::admin_create_branch).layer(require(state.clone(), RouteGuard::Global(Permission::ReleaseGa))))
        // Rename and delete sit at the admin bar (ADR-0005), deliberately
        // above creation: they rewrite/remove a record others reference.
        .route("/admin/branches/{name}", put(admin::admin_rename_branch).layer(require(state.clone(), RouteGuard::Global(Permission::ManageProducers))))
        .route("/admin/branches/{name}", delete(admin::admin_delete_branch).layer(require(state.clone(), RouteGuard::Global(Permission::ManageProducers))))
        .route("/admin/branches/{name}/graph", get(admin::admin_branch_graph).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/branches/{name}/timeline", get(admin::admin_branch_timeline).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/trunk/graph", get(admin::admin_trunk_graph).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/trunk/timeline", get(admin::admin_trunk_timeline).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/graph/diff", get(admin::admin_graph_diff).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/periphery", get(admin::admin_periphery).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/graph/scope", get(admin::admin_graph_scope).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/consumers", get(admin::admin_list_consumers).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/consumers/{name}", get(admin::admin_get_consumer).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/consumers/{name}", delete(admin::admin_delete_consumer).layer(require(state.clone(), RouteGuard::Global(Permission::ManageConsumers))))
        .route("/admin/consumers/{name}/endpoints", get(admin::admin_list_consumer_endpoints).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/endpoint-yaml", get(admin::admin_get_endpoint_yaml).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/endpoint-versions", get(admin::admin_get_endpoint_versions).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/settings/registration", post(admin::set_registration_enabled).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/settings/auto-approve", get(admin::get_auto_approve_users).post(admin::set_auto_approve_users).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/auth-config", get(admin::get_auth_config).put(admin::set_auth_config).layer(require(state.clone(), RouteGuard::Global(Permission::ManageAuthConfig))))
        .route("/admin/auth-config/test", post(admin::test_auth_config).layer(require(state.clone(), RouteGuard::Global(Permission::ManageAuthConfig))))
        .route("/admin/nuke/database", post(admin::admin_nuke_database).layer(require(state.clone(), RouteGuard::Global(Permission::RunDestructiveOperations))))
        .route("/admin/nuke/producers", post(admin::admin_nuke_producers).layer(require(state.clone(), RouteGuard::Global(Permission::RunDestructiveOperations))))
        .route("/admin/nuke/consumers", post(admin::admin_nuke_consumers).layer(require(state.clone(), RouteGuard::Global(Permission::RunDestructiveOperations))))
        .route("/admin/nuke/users", post(admin::admin_nuke_users).layer(require(state.clone(), RouteGuard::Global(Permission::RunDestructiveOperations))))
        // #50: the backup CronJob calls this with an API token, so it admits
        // a full-scope token beside a session; read-only tokens are refused.
        .route("/admin/backup/sqlite", get(admin::download_sqlite_snapshot).layer(require_with_api_tokens(state.clone(), RouteGuard::Global(Permission::RunDestructiveOperations))))
        .route("/admin/settings/snapshot-max-age", get(admin::get_snapshot_max_age).post(admin::set_snapshot_max_age).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        // The ticket URL template is set under settings (ManageSettings) and read by every
        // signed-in user from its own path: the pages link ticket ids with it.
        .route("/admin/settings/ticket-url-template", post(admin::set_ticket_url_template).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/ticket-url-template", get(admin::get_ticket_url_template).layer(require(state.clone(), RouteGuard::Authenticated)))
        // The harmless settings as values for everyone signed in; changing each stays behind ManageSettings.
        .route("/admin/instance-settings", get(admin::get_instance_settings).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/cleanup/snapshots", post(admin::trigger_snapshot_cleanup).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/settings/dependency-max-age", get(admin::get_dependency_max_age).post(admin::set_dependency_max_age).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/cleanup/dependencies", post(admin::trigger_dependency_cleanup).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/settings/trunk-max-age", get(admin::get_trunk_max_age).post(admin::set_trunk_max_age).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/cleanup/trunk", post(admin::trigger_trunk_cleanup).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/users", get(admin::admin_list_users).post(admin::admin_create_service_account).layer(require(state.clone(), RouteGuard::Global(Permission::ManageUsers))))
        .route("/admin/users/{id}/approve", post(admin::admin_approve_user).layer(require(state.clone(), RouteGuard::Global(Permission::ManageUsers))))
        .route("/admin/users/{id}", delete(admin::admin_delete_user_handler).layer(require(state.clone(), RouteGuard::Global(Permission::ManageUsers))))
        // Webhooks (#47): global hooks and the egress allow-list are instance
        // configuration; a system's hooks are its Maintainers' to manage (a
        // Consumer's by a Maintainer of the Producer of the same name).
        .route("/admin/webhooks", get(webhooks::list_global).post(webhooks::create_global).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/webhooks/{id}", get(webhooks::get_global).patch(webhooks::patch_global).delete(webhooks::delete_global).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/webhooks/{id}/test", post(webhooks::test_global).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/webhooks/{id}/rotate-secret", post(webhooks::rotate_global).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/webhooks/{id}/deliveries", get(webhooks::deliveries_global).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/webhooks/{id}/deliveries/{delivery_id}/redeliver", post(webhooks::redeliver_global).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/producers/{name}/webhooks", get(webhooks::list_producer).post(webhooks::create_producer).layer(require(state.clone(), RouteGuard::Producer(Permission::ManageProducers))))
        .route("/admin/producers/{name}/webhooks/{id}", get(webhooks::get_producer).patch(webhooks::patch_producer).delete(webhooks::delete_producer).layer(require(state.clone(), RouteGuard::Producer(Permission::ManageProducers))))
        .route("/admin/producers/{name}/webhooks/{id}/test", post(webhooks::test_producer).layer(require(state.clone(), RouteGuard::Producer(Permission::ManageProducers))))
        .route("/admin/producers/{name}/webhooks/{id}/rotate-secret", post(webhooks::rotate_producer).layer(require(state.clone(), RouteGuard::Producer(Permission::ManageProducers))))
        .route("/admin/producers/{name}/webhooks/{id}/deliveries", get(webhooks::deliveries_producer).layer(require(state.clone(), RouteGuard::Producer(Permission::ManageProducers))))
        .route("/admin/producers/{name}/webhooks/{id}/deliveries/{delivery_id}/redeliver", post(webhooks::redeliver_producer).layer(require(state.clone(), RouteGuard::Producer(Permission::ManageProducers))))
        .route("/admin/consumers/{name}/webhooks", get(webhooks::list_consumer).post(webhooks::create_consumer).layer(require(state.clone(), RouteGuard::Producer(Permission::ManageProducers))))
        .route("/admin/consumers/{name}/webhooks/{id}", get(webhooks::get_consumer).patch(webhooks::patch_consumer).delete(webhooks::delete_consumer).layer(require(state.clone(), RouteGuard::Producer(Permission::ManageProducers))))
        .route("/admin/consumers/{name}/webhooks/{id}/test", post(webhooks::test_consumer).layer(require(state.clone(), RouteGuard::Producer(Permission::ManageProducers))))
        .route("/admin/consumers/{name}/webhooks/{id}/rotate-secret", post(webhooks::rotate_consumer).layer(require(state.clone(), RouteGuard::Producer(Permission::ManageProducers))))
        .route("/admin/consumers/{name}/webhooks/{id}/deliveries", get(webhooks::deliveries_consumer).layer(require(state.clone(), RouteGuard::Producer(Permission::ManageProducers))))
        .route("/admin/consumers/{name}/webhooks/{id}/deliveries/{delivery_id}/redeliver", post(webhooks::redeliver_consumer).layer(require(state.clone(), RouteGuard::Producer(Permission::ManageProducers))))
        .route("/admin/settings/webhook-allowed-hosts", get(webhooks::get_allowed_hosts).post(webhooks::set_allowed_hosts).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/settings/cache", get(admin::get_cache_config).post(admin::set_cache_config).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/settings/database", get(admin::get_database_info).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/cache/clear", post(admin::clear_cache).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))

        // Observability (GET routes accessible to any authenticated user, POST requires admin)
        .route("/admin/observability/stats", get(admin::get_observability_stats).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/observability/logs", get(admin::get_observability_logs).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/observability/debug-config", get(admin::get_debug_config).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/observability/debug-config-update", post(admin::set_debug_config).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/observability/audit-logs", get(admin::get_observability_audit_logs).layer(require(state.clone(), RouteGuard::Global(Permission::ViewAudit))))
        .route("/admin/observability/audit-logs/export", get(admin::export_audit_logs_csv).layer(require(state.clone(), RouteGuard::Global(Permission::ViewAudit))))

        // Auth (Mixed prefix)
        .route("/auth/login", post(auth::auth_login))
        .route("/auth/options", get(auth::auth_options))
        .route("/auth/oidc/login", get(auth::oidc_login))
        .route("/auth/oidc/callback", get(auth::oidc_callback))
        .route("/auth/logout", post(auth::auth_logout))
        .route("/auth/register", post(auth::auth_register))
        .route("/auth/me", get(auth::auth_me).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/auth/change-password", post(auth::auth_change_password).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/auth/tokens", get(auth::list_tokens).post(auth::create_token).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/auth/tokens/{id}", delete(auth::revoke_token).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/auth/favorites", get(auth::get_favorites).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/auth/favorites/{item_type}/{item_name}", post(auth::add_favorite).delete(auth::remove_favorite).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/csrf-token", get(auth::get_csrf_token))

        // Pages / Root
        .route("/", get(pages::index_page))
        .route("/dashboard", get(pages::dashboard_page).layer(from_fn_with_state(state.clone(), authenticated_auth)))
        .route("/health", get(pages::health))
        .route("/ready", get(pages::ready))
        .route("/metrics", get(pages::metrics))
        .route("/LICENSE", get(pages::license_text))
        .route("/version", get(|State(s): State<AppState>| async move { axum::Json(serde_json::json!({"version": env!("CARGO_PKG_VERSION"), "instance_id": s.instance_id})) }))

        // The MCP endpoint (https://paxel.github.io/sanshain-service/developer/mcp.html): JSON-RPC for AI clients, mounted
        // only when enabled.
        .merge(mcp_routes(state.clone()))

        .fallback_service(ServeDir::new(std::env::var("STATIC_DIR").unwrap_or_else(|_| "static".to_string())))
        // Requests whose body exceeds this limit are rejected with `413 Payload
        // Too Large` when the handler extracts the body. The limit counts the
        // bytes the extractor reads, i.e. the decompressed stream for
        // compressed requests.
        .layer(axum::extract::DefaultBodyLimit::max(state.max_body_bytes))
        .layer(tower_http::compression::CompressionLayer::new())
        .layer(tower_http::decompression::RequestDecompressionLayer::new())
        .layer(tower_http::trace::TraceLayer::new_for_http()
            .make_span_with(tower_http::trace::DefaultMakeSpan::new().level(tracing::Level::DEBUG))
            .on_response(tower_http::trace::DefaultOnResponse::new().level(tracing::Level::DEBUG)))
        .layer(from_fn_with_state(state.clone(), validate_csrf))
        // Outside the routes' own auth layers: sees every error answer.
        .layer(axum::middleware::from_fn(presentation::middleware::log_errors))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store, no-cache, must-revalidate, proxy-revalidate"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::PRAGMA,
            HeaderValue::from_static("no-cache"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::EXPIRES,
            HeaderValue::from_static("0"),
        ))
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
            HeaderValue::from_static(CONTENT_SECURITY_POLICY),
        ))
        .with_state(state)
}

/// `/mcp` when `MCP_ENABLED` is true; nothing at all — a 404, not a refusal
/// — when it is false. Bearer API tokens only, see `presentation::mcp`.
// Not formatted: the contract and guard lints below read `.route("` at the
// start of a line, the shape `create_app` keeps for the same reason.
#[rustfmt::skip]
fn mcp_routes(state: AppState) -> Router<AppState> {
    if !state.mcp.enabled {
        return Router::new();
    }
    Router::new()
        .route("/mcp", presentation::mcp::method_router(state.clone()).layer(from_fn_with_state(state, presentation::mcp::mcp_auth)))
}

#[cfg(test)]
mod tests {
    use super::CONTENT_SECURITY_POLICY;

    /// The de-inlining (ai/improvements.md #11) exists so `script-src` can drop
    /// `'unsafe-inline'`; this pins that it stays dropped. `style-src`
    /// deliberately keeps `'unsafe-inline'` (Mermaid injects `<style>`), so the
    /// assertion is scoped to the `script-src` directive rather than the whole
    /// header. It also pins the CDN removal and the anti-framing directive.
    #[test]
    fn csp_script_src_forbids_inline_and_is_self_only() {
        let script_src = CONTENT_SECURITY_POLICY
            .split(';')
            .map(str::trim)
            .find(|d| d.starts_with("script-src"))
            .expect("CSP declares a script-src directive");
        assert_eq!(
            script_src, "script-src 'self'",
            "script-src must be 'self' only — no 'unsafe-inline', 'unsafe-eval' or CDN hosts"
        );
        assert!(
            !CONTENT_SECURITY_POLICY.contains("cdn."),
            "no CDN host may appear in the CSP; third-party libraries are vendored"
        );
        assert!(
            CONTENT_SECURITY_POLICY.contains("frame-ancestors 'none'"),
            "CSP must forbid framing with frame-ancestors 'none'"
        );
    }

    /// Ensures every `/admin/` route in `create_app` states what it requires of
    /// its caller.
    ///
    /// This is a lint over the source of `create_app`, read at compile time: it
    /// parses each `.route(...)` call and checks that an admin route declares a
    /// `RouteGuard`. It is stricter than the check it replaces, which only asked
    /// whether *some* auth middleware was attached and so could not tell an
    /// administrator-only route from one any signed-in user may reach.
    ///
    /// A new admin route added without a guard fails here. It would also fail at
    /// runtime — `require` is the only way to reach these handlers, and an
    /// unrouted guard cannot be constructed — but failing the build is the point.
    #[test]
    fn all_admin_routes_declare_a_route_guard() {
        let source = include_str!("lib.rs");
        let mut undeclared = Vec::new();

        for line in source.lines() {
            let trimmed = line.trim();
            if !trimmed.starts_with(".route(\"") {
                continue;
            }
            let path = trimmed
                .strip_prefix(".route(\"")
                .and_then(|s| s.split('"').next())
                .unwrap_or("");

            if !path.starts_with("/admin/") {
                continue;
            }

            if !trimmed.contains("RouteGuard::") {
                undeclared.push(path.to_string());
            }
        }

        assert!(
            undeclared.is_empty(),
            "The following admin routes do not declare what they require: {:?}\n\
             Every /admin/* route must carry `require(state.clone(), RouteGuard::…)` — \
             `Authenticated` for any signed-in caller, `Global(permission)` for an \
             instance-wide permission, or `Producer(permission)` to additionally admit \
             a maintainer of the Producer named in the path.",
            undeclared
        );
    }

    /// The destructive and configuration surfaces must never be reachable by any
    /// signed-in caller.
    ///
    /// The guard vocabulary makes `Authenticated` easy to reach for, and on a
    /// read-only listing that is right. This pins the routes where it would be a
    /// privilege escalation, so a future edit cannot quietly widen them.
    #[test]
    fn dangerous_admin_routes_are_not_merely_authenticated() {
        let source = include_str!("lib.rs");
        let must_be_privileged = [
            "/admin/nuke/",
            "/admin/settings/",
            "/admin/auth-config",
            "/admin/backup/",
        ];
        let mut widened = Vec::new();

        for line in source.lines() {
            let trimmed = line.trim();
            if !trimmed.starts_with(".route(\"") {
                continue;
            }
            let path = trimmed
                .strip_prefix(".route(\"")
                .and_then(|s| s.split('"').next())
                .unwrap_or("");

            if !must_be_privileged.iter().any(|p| path.starts_with(p)) {
                continue;
            }
            if trimmed.contains("RouteGuard::Authenticated") {
                widened.push(path.to_string());
            }
        }

        assert!(
            widened.is_empty(),
            "These routes must require a permission, not merely a signed-in caller: {:?}",
            widened
        );
    }

    /// Every Producer/Consumer API route that changes the catalog must carry
    /// the write guard, so a read-only token is refused at the router and not
    /// by a check each handler has to remember.
    #[test]
    fn write_api_routes_refuse_read_only_tokens() {
        let source = include_str!("lib.rs");
        let writers = ["/provide", "/require", "/catalog/"];
        let mut unguarded = Vec::new();

        for line in source.lines() {
            let trimmed = line.trim();
            if !trimmed.starts_with(".route(\"") {
                continue;
            }
            let path = trimmed
                .strip_prefix(".route(\"")
                .and_then(|s| s.split('"').next())
                .unwrap_or("");
            if !writers.iter().any(|w| path.starts_with(w)) {
                continue;
            }
            if !trimmed.contains("api_auth_write") {
                unguarded.push(path.to_string());
            }
        }

        assert!(
            unguarded.is_empty(),
            "These catalog-changing routes do not refuse read-only tokens: {:?}\n\
             Guard them with `api_auth_write`.",
            unguarded
        );
    }

    /// Ensures the router and the maintenance contract in `maintenance.yaml`
    /// stay in sync.
    ///
    /// The sibling of `router_matches_api_yaml_contract`, for the other half of
    /// the surface. `api.yaml` is the contract tooling is written against and
    /// must stay stable; `maintenance.yaml` documents the administrative
    /// surface, which changes whenever the admin interface does. Every `/admin/*`
    /// route must appear in it — there is no allowlist here, because an
    /// administrative endpoint nobody documented is exactly what this file
    /// exists to prevent.
    #[test]
    fn router_matches_maintenance_yaml_contract() {
        use std::collections::BTreeSet;

        let source = include_str!("lib.rs");
        let registered: BTreeSet<String> = source
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with(".route(\""))
            .filter_map(|l| {
                l.strip_prefix(".route(\"")
                    .and_then(|s| s.split('"').next())
            })
            .filter(|p| p.starts_with("/admin/"))
            .map(str::to_string)
            .collect();

        let spec = include_str!("../maintenance.yaml");
        // The documented paths are the two-space-indented keys under `paths:`.
        let documented: BTreeSet<String> = spec
            .lines()
            .filter(|l| l.starts_with("  /admin/") && l.trim_end().ends_with(':'))
            .map(|l| l.trim().trim_end_matches(':').to_string())
            .collect();

        let undocumented: Vec<&String> = registered.difference(&documented).collect();
        assert!(
            undocumented.is_empty(),
            "These admin routes are not documented in maintenance.yaml: {:?}\n\
             Document them there — the maintenance surface has no allowlist.",
            undocumented
        );

        let stale: Vec<&String> = documented.difference(&registered).collect();
        assert!(
            stale.is_empty(),
            "maintenance.yaml documents routes that no longer exist: {:?}",
            stale
        );
    }

    /// Ensures the contracts declare what the router enforces: every
    /// `maintenance.yaml` operation carries `x-permission` and `x-scope`
    /// matching its `require(...)` guard (and `x-api-tokens: full` exactly
    /// where `require_with_api_tokens` admits a full API token), and every
    /// `api.yaml` operation carries the `x-auth` its middleware implies. The
    /// documentation site renders these fields, so a guard change that is not
    /// mirrored here fails the build instead of drifting.
    #[test]
    fn contracts_declare_the_router_guards() {
        use std::collections::BTreeMap;

        fn wire(permission: &str) -> String {
            let mut out = String::new();
            for (i, c) in permission.chars().enumerate() {
                if c.is_ascii_uppercase() && i > 0 {
                    out.push('_');
                }
                out.push(c.to_ascii_lowercase());
            }
            out
        }

        // (method, path) -> (guard kind, scope kind, permission wire name).
        // One path may be routed twice with different guards per method.
        let mut guards: BTreeMap<(String, String), (String, String, String)> = BTreeMap::new();
        for line in include_str!("lib.rs").lines().map(str::trim) {
            let Some(rest) = line.strip_prefix(".route(\"") else {
                continue;
            };
            let Some(path) = rest.split('"').next() else {
                continue;
            };
            let methods: Vec<&str> = ["get", "post", "put", "patch", "delete"]
                .into_iter()
                .filter(|m| rest.contains(&format!("{m}(")))
                .collect();
            let kind = if rest.contains("require_with_api_tokens(") {
                "require_with_api_tokens"
            } else if rest.contains("require(") {
                "require"
            } else if rest.contains("api_auth_write") {
                "api_auth_write"
            } else if rest.contains("api_auth") {
                "api_auth"
            } else if rest.contains("from_fn") {
                "session"
            } else {
                "none"
            };
            let (scope, permission) = match rest.find("RouteGuard::") {
                Some(at) => {
                    let guard = &rest[at + "RouteGuard::".len()..];
                    let scope: String = guard
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric())
                        .collect();
                    let permission = guard
                        .find("Permission::")
                        .map(|p| {
                            guard[p + "Permission::".len()..]
                                .chars()
                                .take_while(|c| c.is_ascii_alphanumeric())
                                .collect::<String>()
                        })
                        .map(|p| wire(&p))
                        .unwrap_or_else(|| "any-signed-in".to_string());
                    (scope, permission)
                }
                None => (String::new(), String::new()),
            };
            for method in methods {
                guards.insert(
                    (method.to_string(), path.to_string()),
                    (kind.to_string(), scope.clone(), permission.clone()),
                );
            }
        }

        let mut problems: Vec<String> = Vec::new();
        let field = |op: &serde_yaml_ng::Value, name: &str| -> String {
            op.get(name)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };

        let maintenance: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(include_str!("../maintenance.yaml"))
                .expect("maintenance.yaml must parse");
        for (path, item) in maintenance["paths"].as_mapping().expect("paths") {
            let path = path.as_str().expect("path key");
            for (method, op) in item.as_mapping().expect("path item") {
                let method = method.as_str().expect("method key");
                let Some((kind, scope, permission)) =
                    guards.get(&(method.to_string(), path.to_string()))
                else {
                    continue; // the sibling test reports unrouted paths
                };
                let expected_scope = match scope.as_str() {
                    "Global" | "Authenticated" => "global",
                    "Producer" if path.starts_with("/admin/consumers/") => "consumer",
                    "Producer" => "producer",
                    other => other,
                };
                // The promote route is guarded `Authenticated`; the handler
                // applies the GA gate itself and the contract documents that.
                let expected_permission = if path.ends_with("/promote") {
                    "release_ga".to_string()
                } else {
                    permission.clone()
                };
                if field(op, "x-permission") != expected_permission {
                    problems.push(format!(
                        "{method} {path}: x-permission is {:?}, the router guard says {expected_permission:?}",
                        field(op, "x-permission")
                    ));
                }
                if field(op, "x-scope") != expected_scope {
                    problems.push(format!(
                        "{method} {path}: x-scope is {:?}, the router guard says {expected_scope:?}",
                        field(op, "x-scope")
                    ));
                }
                let expected_tokens = if kind == "require_with_api_tokens" {
                    "full"
                } else {
                    ""
                };
                if field(op, "x-api-tokens") != expected_tokens {
                    problems.push(format!(
                        "{method} {path}: x-api-tokens is {:?}, the router says {expected_tokens:?}",
                        field(op, "x-api-tokens")
                    ));
                }
            }
        }

        let api: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(include_str!("../api.yaml")).expect("api.yaml must parse");
        for (path, item) in api["paths"].as_mapping().expect("paths") {
            let path = path.as_str().expect("path key");
            for (method, op) in item.as_mapping().expect("path item") {
                let method = method.as_str().expect("method key");
                let Some((kind, _, _)) = guards.get(&(method.to_string(), path.to_string())) else {
                    continue;
                };
                let expected = match kind.as_str() {
                    "api_auth_write" => "session-or-api-token-full",
                    "api_auth" => "session-or-api-token",
                    "none" => "none",
                    _ => "session",
                };
                if field(op, "x-auth") != expected {
                    problems.push(format!(
                        "{method} {path}: x-auth is {:?}, the router says {expected:?}",
                        field(op, "x-auth")
                    ));
                }
            }
        }

        assert!(
            problems.is_empty(),
            "The contracts disagree with the router guards:\n{}\n\
             Fix the x-permission / x-scope / x-api-tokens fields in maintenance.yaml or \
             x-auth in api.yaml (or the guard in lib.rs).",
            problems.join("\n")
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

    /// Ensures the router in `create_app` and the OpenAPI contract in `api.yaml`
    /// stay in sync.
    ///
    /// `api.yaml` documents the client API contract (build plugins, CLIs). Every
    /// path it documents must be registered in the router, and every registered
    /// route must be accounted for: documented there, documented in
    /// `maintenance.yaml` (the administrative surface), or listed in the explicit
    /// allowlist of routes that belong to neither contract — HTML pages, the
    /// browser's own session endpoints, operational probes. Stale allowlist
    /// entries fail the test too, so the list cannot rot.
    #[test]
    fn router_matches_api_yaml_contract() {
        use std::collections::BTreeSet;

        // Routes intentionally NOT part of the client API contract. Moving a
        // route into the contract means documenting it in api.yaml and removing
        // it here. Axum `{param}` syntax matches OpenAPI templating, so paths
        // compare as plain strings.
        const NOT_IN_CLIENT_CONTRACT: &[&str] = &[
            // HTML pages, static assets, operational endpoints
            "/",
            "/dashboard",
            "/health",
            "/ready",
            "/metrics",
            "/LICENSE",
            // Browser session, account, and CSRF endpoints for the UI
            "/auth/change-password",
            "/auth/favorites",
            "/auth/favorites/{item_type}/{item_name}",
            "/auth/login",
            "/auth/oidc/login",
            "/auth/oidc/callback",
            "/auth/options",
            "/auth/logout",
            "/auth/me",
            "/auth/register",
            "/csrf-token",
            "/tokens",
            "/tokens/{id}",
            // Reports, live updates, and other UI-facing read APIs
            "/api/dashboard",
            "/api/dashboard/recent",
            // JSON-RPC for AI clients (https://paxel.github.io/sanshain-service/developer/mcp.html), not REST
            "/mcp",
        ];

        let source = include_str!("lib.rs");
        let mut router_paths: BTreeSet<&str> = BTreeSet::new();
        for line in source.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix(".route(\"")
                && let Some(path) = rest.split('"').next()
            {
                router_paths.insert(path);
            }
        }

        let spec: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(include_str!("../api.yaml")).expect("api.yaml must parse");
        let spec_paths: BTreeSet<&str> = spec
            .get("paths")
            .and_then(|p| p.as_mapping())
            .expect("api.yaml must have a `paths` mapping")
            .keys()
            .map(|k| k.as_str().expect("api.yaml path keys must be strings"))
            .collect();

        let allowlist: BTreeSet<&str> = NOT_IN_CLIENT_CONTRACT.iter().copied().collect();

        // Routes documented in the maintenance contract instead. That file has
        // its own check below; here it only needs to account for a route's
        // absence from the client contract.
        let maintenance_spec: serde_yaml_ng::Value =
            serde_yaml_ng::from_str(include_str!("../maintenance.yaml"))
                .expect("maintenance.yaml must parse");
        let maintenance_paths: BTreeSet<&str> = maintenance_spec
            .get("paths")
            .and_then(|p| p.as_mapping())
            .expect("maintenance.yaml must have a `paths` mapping")
            .keys()
            .map(|k| {
                k.as_str()
                    .expect("maintenance.yaml path keys must be strings")
            })
            .collect();

        let phantom: Vec<&str> = spec_paths.difference(&router_paths).copied().collect();
        assert!(
            phantom.is_empty(),
            "api.yaml documents paths that are not registered in create_app(): {:?}\n\
             Register the route or remove the path from api.yaml.",
            phantom
        );

        let undocumented: Vec<&str> = router_paths
            .iter()
            .filter(|p| {
                !spec_paths.contains(*p)
                    && !allowlist.contains(*p)
                    && !maintenance_paths.contains(*p)
            })
            .copied()
            .collect();
        assert!(
            undocumented.is_empty(),
            "Routes registered in create_app() are neither documented in api.yaml \
             nor allowlisted: {:?}\n\
             Document them in api.yaml, or add them to NOT_IN_CLIENT_CONTRACT if they \
             are intentionally outside the client API contract.",
            undocumented
        );

        let stale: Vec<&str> = allowlist
            .iter()
            .filter(|p| !router_paths.contains(*p) || spec_paths.contains(*p))
            .copied()
            .collect();
        assert!(
            stale.is_empty(),
            "Stale NOT_IN_CLIENT_CONTRACT entries (route removed, or now documented \
             in api.yaml): {:?}\n\
             Remove them from the allowlist.",
            stale
        );
    }
}
