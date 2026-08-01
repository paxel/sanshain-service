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

pub use presentation::handlers::{admin, api, auth, pages, roles};
pub use presentation::middleware::{
    LogCaptureLayer, RouteGuard, api_auth, authenticated_auth, require, validate_csrf,
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
    /// Usernames holding every permission by configuration. Read once at
    /// startup and never stored, so nothing in the database can revoke root.
    pub root_users: Arc<domain::permissions::RootUsers>,
    /// Caches directory group membership, so a change in the directory takes
    /// effect within the cache lifetime rather than at the next login.
    pub directory_roles: application::directory_roles::DirectoryRoleCache,
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
        .route("/branches/metadata", get(api::list_branches_metadata).layer(from_fn_with_state(state.clone(), api_auth)))
        .route("/endpoint-versions", get(api::endpoint_versions).layer(from_fn_with_state(state.clone(), api_auth)))
        // The audit trail records the Actor behind every change, across every
        // Producer, so it needs `ViewAudit`. `permission_auth` validates sessions
        // only, so API tokens cannot read it — deliberate, and safe because this
        // route is outside `api.yaml`.
        .route("/api/audit/timeline", get(api::audit_timeline).layer(require(state.clone(), RouteGuard::Global(Permission::ViewAudit))))
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
        // The inbox is opened by anyone signed in; which entries they may see or
        // act on is a per-Producer question the handler answers once the entry
        // has been loaded, since the Producer is not in the path.
        .route("/admin/pending-specs", get(admin::admin_list_pending_specs).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/pending-specs/{id}", get(admin::admin_get_pending_spec).delete(admin::admin_reject_pending_spec).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/pending-specs/{id}/accept", post(admin::admin_accept_pending_spec).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/producers/onboarding", get(admin::admin_list_onboarding_producers).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/producers/{name}/onboarding", get(admin::admin_get_producer_onboarding).put(admin::admin_set_producer_onboarding).layer(require(state.clone(), RouteGuard::Producer(Permission::SetOnboarding))))
        .route("/admin/producers/{name}/maintainers", get(roles::list_maintainers).post(roles::assign_maintainer).layer(require(state.clone(), RouteGuard::Global(Permission::ManageRoles))))
        .route("/admin/producers/{name}/maintainers/users/{user_id}", delete(roles::unassign_user_maintainer).layer(require(state.clone(), RouteGuard::Global(Permission::ManageRoles))))
        .route("/admin/producers/{name}/maintainers/groups/{group_id}", delete(roles::unassign_group_maintainer).layer(require(state.clone(), RouteGuard::Global(Permission::ManageRoles))))
        .route("/admin/users/{id}/maintains", get(roles::list_maintained_producers).layer(require(state.clone(), RouteGuard::Global(Permission::ManageRoles))))
        .route("/admin/protected-branches", get(admin::list_protected_branches).post(admin::add_protected_branch).layer(require(state.clone(), RouteGuard::Global(Permission::ManageProtectedBranches))))
        .route("/admin/protected-branches/{pattern}", delete(admin::delete_protected_branch).layer(require(state.clone(), RouteGuard::Global(Permission::ManageProtectedBranches))))
        .route("/admin/producers", get(admin::admin_list_producers).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/producers/{name}", delete(admin::admin_delete_producer).layer(require(state.clone(), RouteGuard::Producer(Permission::ManageProducers))))
        .route("/admin/producers/metadata", post(admin::admin_update_producer_metadata).layer(require(state.clone(), RouteGuard::Global(Permission::ManageProducers))))
        .route("/admin/endpoints/update", post(admin::admin_update_endpoint).layer(require(state.clone(), RouteGuard::Global(Permission::ManageProducers))))
        .route("/admin/producers/{name}/branches", get(admin::admin_list_branches).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/producers/{name}/branches/{branch}", delete(admin::admin_delete_branch).layer(require(state.clone(), RouteGuard::Producer(Permission::ManageProducers))))
        .route("/admin/producers/{name}/branches/{branch}/reset-history", post(admin::admin_reset_branch_history).layer(require(state.clone(), RouteGuard::Producer(Permission::ManageProducers))))
        .route("/admin/producers/{name}/branches/{branch}/source-protected-branch", get(admin::admin_get_source_protected_branch).put(admin::admin_set_source_protected_branch).layer(require(state.clone(), RouteGuard::Producer(Permission::ManageProducers))))
        .route("/admin/producers/{name}/branches/{branch}/endpoints", get(admin::admin_list_producer_endpoints).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/producers/{name}/branches/{branch}/full-spec", get(admin::admin_get_full_spec).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/consumers", get(admin::admin_list_consumers).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/consumers/{name}", delete(admin::admin_delete_consumer).layer(require(state.clone(), RouteGuard::Global(Permission::ManageConsumers))))
        .route("/admin/consumers/{name}/branches", get(admin::admin_list_consumer_branches).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/consumers/{name}/branches/{branch}/endpoints", get(admin::admin_list_consumer_endpoints).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/endpoint-yaml", get(admin::admin_get_endpoint_yaml).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/endpoint-versions", get(admin::admin_get_endpoint_versions).layer(require(state.clone(), RouteGuard::Authenticated)))
        .route("/admin/settings/dev-mode", get(admin::get_dev_mode).post(admin::set_dev_mode).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/settings/auto-approve", get(admin::get_auto_approve_users).post(admin::set_auto_approve_users).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/auth-config", get(admin::get_auth_config).put(admin::set_auth_config).layer(require(state.clone(), RouteGuard::Global(Permission::ManageAuthConfig))))
        .route("/admin/auth-config/test", post(admin::test_auth_config).layer(require(state.clone(), RouteGuard::Global(Permission::ManageAuthConfig))))
        .route("/admin/nuke/database", post(admin::admin_nuke_database).layer(require(state.clone(), RouteGuard::Global(Permission::RunDestructiveOperations))))
        .route("/admin/nuke/producers", post(admin::admin_nuke_producers).layer(require(state.clone(), RouteGuard::Global(Permission::RunDestructiveOperations))))
        .route("/admin/nuke/consumers", post(admin::admin_nuke_consumers).layer(require(state.clone(), RouteGuard::Global(Permission::RunDestructiveOperations))))
        .route("/admin/nuke/users", post(admin::admin_nuke_users).layer(require(state.clone(), RouteGuard::Global(Permission::RunDestructiveOperations))))
        .route("/admin/nuke/branch/{branch}", post(admin::admin_nuke_branch).layer(require(state.clone(), RouteGuard::Global(Permission::RunDestructiveOperations))))
        .route("/admin/settings/branch-max-age", get(admin::get_branch_max_age).post(admin::set_branch_max_age).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/cleanup/branches", post(admin::trigger_branch_cleanup).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/settings/branch-cleanup", post(admin::trigger_branch_cleanup).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/settings/dependency-max-age", get(admin::get_dependency_max_age).post(admin::set_dependency_max_age).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/cleanup/dependencies", post(admin::trigger_dependency_cleanup).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/settings/dependency-cleanup", post(admin::trigger_dependency_cleanup).layer(require(state.clone(), RouteGuard::Global(Permission::ManageSettings))))
        .route("/admin/users", get(admin::admin_list_users).layer(require(state.clone(), RouteGuard::Global(Permission::ManageUsers))))
        .route("/admin/users/{id}/approve", post(admin::admin_approve_user).layer(require(state.clone(), RouteGuard::Global(Permission::ManageUsers))))
        .route("/admin/users/{id}", delete(admin::admin_delete_user_handler).layer(require(state.clone(), RouteGuard::Global(Permission::ManageUsers))))
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
            HeaderValue::from_static("default-src 'self'; script-src 'self' 'unsafe-inline' https://cdn.tailwindcss.com https://cdn.jsdelivr.net; style-src 'self' 'unsafe-inline' https://cdn.tailwindcss.com https://cdn.jsdelivr.net; img-src 'self' data: blob:;"),
        ))
        .with_state(state)
}

#[cfg(test)]
mod tests {
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
        let must_be_privileged = ["/admin/nuke/", "/admin/settings/", "/admin/auth-config"];
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
            "/auth/logout",
            "/auth/me",
            "/auth/register",
            "/auth/tokens",
            "/auth/tokens/{id}",
            "/csrf-token",
            "/tokens",
            "/tokens/{id}",
            // Reports, live updates, and other UI-facing read APIs
            "/api/audit/timeline",
            "/api/sse/updates",
            "/api/ws/updates",
            "/endpoint-versions",
            "/report",
            "/report/isolation",
            "/report/markdown",
            "/report/merged",
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
