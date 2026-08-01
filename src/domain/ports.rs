use crate::domain::models::*;
use std::collections::HashMap;
use std::future::Future;
use thiserror::Error;

/// Error type for authentication provider operations.
#[derive(Error, Debug)]
pub enum AuthProviderError {
    #[error("Invalid credentials")]
    InvalidCredentials,
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Internal error: {0}")]
    Internal(String),
}

/// Port trait for pluggable authentication providers.
pub trait AuthProvider: Send + Sync {
    /// Authenticate a user by username and password.
    fn authenticate(
        &self,
        username: &str,
        password: &str,
    ) -> impl Future<Output = Result<AuthenticatedUser, AuthProviderError>> + Send;

    /// Test connectivity to the auth backend.
    fn test_connection(&self) -> impl Future<Output = Result<(), AuthProviderError>> + Send;
}

/// Port for reading a user's group membership from the directory.
///
/// Separate from [`AuthProvider`] because it answers a different question at a
/// different time: authentication happens once, at login, with the user's
/// password; membership has to be re-read afterwards, without one, or a change
/// in the directory would never take effect.
pub trait DirectoryGroups: Send + Sync {
    fn groups_for(
        &self,
        username: &str,
    ) -> impl Future<Output = Result<Vec<String>, AuthProviderError>> + Send;
}

#[derive(Error, Debug)]
pub enum RepositoryError {
    #[error("Not Found")]
    NotFound,
    #[error("Conflict")]
    Conflict,
    #[error("Internal Error: {0}")]
    Internal(String),
}

/// Port for all persistence operations required by the application layer.
pub struct RecordDependencyParams<'a> {
    pub client_id: i64,
    pub endpoint_id: Option<i64>,
    pub api_type: ApiType,
    pub service_id: i64,
    pub branch_name: &'a str,
    pub path: &'a str,
    pub method: &'a str,
}

pub type EndpointDetails = (i64, String, bool, bool);
pub type EndpointMap = HashMap<(String, String), EndpointDetails>;

pub struct UpdateEndpointParams<'a> {
    pub branch_id: i64,
    pub api_type: ApiType,
    pub path: &'a str,
    pub method: &'a str,
    pub yaml_content: &'a str,
    pub deprecated: bool,
    pub external: bool,
}

/// Content of a new audit-log entry — everything except the acting username,
/// which the caller resolves from the request context.
pub struct NewAuditLog<'a> {
    pub action: &'a str,
    pub details: &'a str,
    pub service: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub action_type: Option<&'a str>,
    pub diff: Option<&'a str>,
}

pub trait SpecRepository: Send + Sync {
    /// Liveness/readiness check against the backing store: runs a trivial query
    /// (`SELECT 1`) to confirm the connection pool can reach the database.
    fn ping(&self) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Get the current spec version and content hash for a service/branch.
    fn get_spec_version(
        &self,
        service_id: i64,
        branch_id: i64,
    ) -> impl Future<Output = Result<Option<(SemVer, String)>, RepositoryError>> + Send;

    /// Update or increment the spec version and content hash for a service/branch.
    /// Returns the new version.
    fn increment_spec_version(
        &self,
        service_id: i64,
        branch_id: i64,
        content_hash: &str,
        impact: Impact,
    ) -> impl Future<Output = Result<SemVer, RepositoryError>> + Send;

    /// Ensure a service exists and return its ID.
    fn ensure_service(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<i64, RepositoryError>> + Send;

    /// Find a service by name (read-only, does not create). Returns None if not found.
    fn find_service(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<Option<i64>, RepositoryError>> + Send;

    /// Get a service name by its ID. Returns None if not found.
    fn get_service_name_by_id(
        &self,
        service_id: i64,
    ) -> impl Future<Output = Result<Option<String>, RepositoryError>> + Send;

    /// Ensure a branch exists for a service and return its ID.
    fn ensure_branch(
        &self,
        service_id: i64,
        branch_name: &str,
    ) -> impl Future<Output = Result<i64, RepositoryError>> + Send;

    /// Find a branch by service ID and name (read-only, does not create). Returns None if not found.
    fn find_branch(
        &self,
        service_id: i64,
        branch_name: &str,
    ) -> impl Future<Output = Result<Option<i64>, RepositoryError>> + Send;

    /// Fetch all existing endpoints for a branch.
    fn get_endpoints_for_branch(
        &self,
        branch_id: i64,
    ) -> impl Future<Output = Result<Vec<EndpointRecord>, RepositoryError>> + Send;

    /// Insert a new endpoint for a branch.
    fn insert_endpoint(
        &self,
        branch_id: i64,
        endpoint: &EndpointRecord,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Prune all old versions of endpoints and reset the branch version to 1.
    fn reset_branch_history(
        &self,
        service_name: &str,
        branch_name: &str,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Ensure a client exists and return its ID.
    fn ensure_client(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<i64, RepositoryError>> + Send;

    /// Find an endpoint by service, branch, path, and method. Returns (endpoint_id, yaml_content, deprecated, external).
    fn find_endpoint(
        &self,
        service_id: i64,
        branch_name: &str,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> impl Future<Output = Result<Option<(i64, String, bool, bool)>, RepositoryError>> + Send;

    /// Find multiple endpoints by service, branch, path, and method.
    /// Returns a map of (path, method) to (endpoint_id, yaml_content).
    fn find_endpoints_bulk(
        &self,
        service_id: i64,
        branch_name: &str,
        api_type: ApiType,
        endpoints: &[(String, String)],
    ) -> impl Future<Output = Result<EndpointMap, RepositoryError>> + Send;

    /// Record a client dependency on an endpoint.
    fn record_dependency(
        &self,
        params: RecordDependencyParams,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Record multiple client dependencies at once.
    fn record_dependencies_bulk(
        &self,
        params: Vec<RecordDependencyParams>,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Get the full dependency report for a branch.
    fn get_report(
        &self,
        branch: &str,
    ) -> impl Future<Output = Result<DependencyReport, RepositoryError>> + Send;

    /// Check if a branch name matches any protected branch pattern.
    fn is_branch_protected(
        &self,
        branch_name: &str,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Add a protected branch pattern.
    fn add_protected_branch(
        &self,
        pattern: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Remove a protected branch pattern.
    fn remove_protected_branch(
        &self,
        pattern: &str,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// List all protected branch patterns.
    fn list_protected_branches(
        &self,
    ) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// Update an existing endpoint's YAML content and deprecated/external status.
    fn update_endpoint(
        &self,
        params: UpdateEndpointParams<'_>,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Soft-delete an endpoint (mark as deleted). Used on protected branches to preserve history.
    fn soft_delete_endpoint(
        &self,
        branch_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Hard-delete endpoints by branch, path, and method. Used on non-protected branches.
    fn hard_delete_endpoint(
        &self,
        branch_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Check if a soft-deleted endpoint exists for the given branch, path, and method.
    fn is_endpoint_deleted(
        &self,
        branch_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Delete all services and their branches, endpoints, and related dependencies.
    fn delete_all_services(&self) -> impl Future<Output = Result<u64, RepositoryError>> + Send;

    /// Delete all clients and their dependencies.
    fn delete_all_clients(&self) -> impl Future<Output = Result<u64, RepositoryError>> + Send;

    /// Delete all non-admin users and their sessions.
    fn delete_all_non_admin_users(
        &self,
    ) -> impl Future<Output = Result<u64, RepositoryError>> + Send;

    /// Nuke the entire database: delete all data from all tables (except settings and the calling admin user).
    fn nuke_database(
        &self,
        keep_user_id: Option<i64>,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Delete a service and all its branches, endpoints, and related dependencies.
    fn delete_producer(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Delete a branch (by service name and branch name) and its endpoints and related dependencies.
    fn delete_branch(
        &self,
        service_name: &str,
        branch_name: &str,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Delete a client and all its dependencies.
    fn delete_consumer(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// List all services.
    fn list_producers(&self) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// List all services with detailed information (like fallback branch).
    fn list_producers_detailed(
        &self,
    ) -> impl Future<Output = Result<Vec<ProducerSummary>, RepositoryError>> + Send;

    /// Set the fallback branch for a service.
    fn set_fallback_branch(
        &self,
        service_name: &str,
        branch: Option<&str>,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Update service metadata (icon, domain).
    fn update_producer_metadata(
        &self,
        service_name: &str,
        icon: Option<&str>,
        domain: Option<&str>,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Get the fallback branch for a service.
    fn get_fallback_branch(
        &self,
        service_name: &str,
    ) -> impl Future<Output = Result<Option<String>, RepositoryError>> + Send;

    /// Atomically set a branch's `source_protected_branch` (item #17), but only
    /// if it is currently unset. Returns `true` if this call set it, `false` if
    /// the branch already had a value (left untouched) — callers use this to
    /// distinguish "I set it" from "someone already had a different answer",
    /// for mismatch logging.
    fn set_source_protected_branch_if_unset(
        &self,
        branch_id: i64,
        value: &str,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Get a branch's current `source_protected_branch`, if any.
    fn get_source_protected_branch(
        &self,
        branch_id: i64,
    ) -> impl Future<Output = Result<Option<String>, RepositoryError>> + Send;

    /// Admin-only: unconditionally set (or clear, with `None`) a branch's
    /// `source_protected_branch`, overwriting any existing value.
    fn admin_set_source_protected_branch(
        &self,
        branch_id: i64,
        value: Option<&str>,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// List all branches for a service.
    fn list_branches(
        &self,
        service_name: &str,
    ) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// List all distinct branch names across all services.
    fn list_all_branches(
        &self,
    ) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// List all clients.
    fn list_consumers(&self) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// List all branches that a client has dependencies on.
    fn list_consumer_branches(
        &self,
        client_name: &str,
    ) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// List all endpoints a client depends on for a given branch.
    fn list_consumer_endpoints(
        &self,
        client_name: &str,
        branch: &str,
    ) -> impl Future<Output = Result<Vec<ConsumerEndpointInfo>, RepositoryError>> + Send;

    // --- Auth ---

    /// Count total users.
    fn user_count(&self) -> impl Future<Output = Result<i64, RepositoryError>> + Send;

    /// Find a user by username.
    fn find_user(
        &self,
        username: &str,
    ) -> impl Future<Output = Result<Option<User>, RepositoryError>> + Send;

    /// Create a new user.
    fn create_user(
        &self,
        username: &str,
        password_hash: &str,
        approved: bool,
    ) -> impl Future<Output = Result<User, RepositoryError>> + Send;

    /// Update a user's password hash.
    fn update_password(
        &self,
        user_id: i64,
        new_hash: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// List all users (id, username, approved).
    fn list_users(&self) -> impl Future<Output = Result<Vec<User>, RepositoryError>> + Send;

    /// Approve a user by ID.
    fn approve_user(
        &self,
        user_id: i64,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Delete a user by ID.
    fn delete_user(
        &self,
        user_id: i64,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Create a session for a user, returning the session with a generated token.
    fn create_session(
        &self,
        user_id: i64,
        expires_at: &str,
    ) -> impl Future<Output = Result<Session, RepositoryError>> + Send;

    /// Create a session for a user with a specific token.
    fn create_session_with_token(
        &self,
        user_id: i64,
        token: &str,
        expires_at: &str,
    ) -> impl Future<Output = Result<Session, RepositoryError>> + Send;

    /// Validate a session token, returning the user and session if valid and not expired.
    fn validate_session(
        &self,
        token: &str,
    ) -> impl Future<Output = Result<Option<(User, Session)>, RepositoryError>> + Send;

    /// Delete a session (logout).
    fn delete_session(
        &self,
        token: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Get a setting value by key.
    fn get_setting(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<String>, RepositoryError>> + Send;

    /// Set a setting value.
    fn set_setting(
        &self,
        key: &str,
        value: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    // --- API Tokens ---

    /// Create an API token (stores the hash, not the raw token).
    fn create_api_token(
        &self,
        id: &str,
        user_id: i64,
        name: &str,
        token_hash: &str,
        created_at: &str,
        expires_at: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// List all API tokens for a user (never includes raw token).
    fn list_api_tokens(
        &self,
        user_id: i64,
    ) -> impl Future<Output = Result<Vec<ApiToken>, RepositoryError>> + Send;

    /// Delete an API token by ID (only if owned by user_id). Returns whether a
    /// token was actually deleted, so the caller can answer 404 rather than
    /// reporting success for a token that never existed.
    ///
    /// Deliberately does *not* return the token's name: audit entries for token
    /// revocation must not record it (a name its creator considered private
    /// cannot be redacted from a permanent audit log afterwards), so the name
    /// is not handed to callers who might log it.
    fn delete_api_token(
        &self,
        token_id: &str,
        user_id: i64,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Validate an API token hash, returning the user if valid and not expired. Also updates last_used_at.
    fn validate_api_token(
        &self,
        token_hash: &str,
    ) -> impl Future<Output = Result<Option<User>, RepositoryError>> + Send;

    /// Delete non-protected branches that haven't been updated since the given cutoff ISO timestamp.
    /// Returns the number of deleted branches.
    fn delete_stale_branches(
        &self,
        cutoff_iso: &str,
    ) -> impl Future<Output = Result<u64, RepositoryError>> + Send;

    /// Delete dependency rows whose `last_seen_at` is older than the given cutoff ISO timestamp.
    /// Returns the number of deleted rows.
    fn delete_stale_dependencies(
        &self,
        cutoff_iso: &str,
    ) -> impl Future<Output = Result<u64, RepositoryError>> + Send;

    /// Get the endpoint ID for a given branch, path, and method.
    fn get_endpoint_id(
        &self,
        branch_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> impl Future<Output = Result<Option<i64>, RepositoryError>> + Send;

    /// Insert a new endpoint version record.
    fn insert_endpoint_version(
        &self,
        endpoint_id: i64,
        version: i32,
        yaml_content: &str,
        diff: Option<&str>,
        created_at: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Get the latest version number for an endpoint (0 if none).
    fn get_latest_endpoint_version(
        &self,
        endpoint_id: i64,
    ) -> impl Future<Output = Result<i32, RepositoryError>> + Send;

    /// Get the version history for an endpoint.
    fn get_endpoint_versions(
        &self,
        endpoint_id: i64,
    ) -> impl Future<Output = Result<Vec<EndpointVersion>, RepositoryError>> + Send;

    /// Get a global timeline of endpoint versions across all services and branches.
    fn get_global_endpoint_versions(
        &self,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<EndpointVersion>, RepositoryError>> + Send;

    /// Apply a set of specification changes (insert, update, delete) atomically.
    fn apply_spec_changes(
        &self,
        branch_id: i64,
        changes: Vec<SpecChange>,
        is_protected: bool,
        username: Option<&str>,
        source_branch: Option<&str>,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    // --- Roles and Groups ---

    /// Grant a role to a user. Granting a role already held is a no-op.
    fn grant_user_role(
        &self,
        user_id: i64,
        role: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Revoke a role from a user. Returns whether a grant was actually removed.
    fn revoke_user_role(
        &self,
        user_id: i64,
        role: &str,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Roles granted directly to a user, ignoring any held via a group.
    fn list_user_roles(
        &self,
        user_id: i64,
    ) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// Every role name a user holds from stored grants — directly, or through a
    /// group whose membership Sanshain stores.
    ///
    /// Directory-sourced groups are deliberately absent: their membership is not
    /// stored, so it is resolved above the repository where the directory can be
    /// consulted.
    fn effective_stored_roles(
        &self,
        user_id: i64,
    ) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// Create a group, or return the existing one with the same name and source.
    fn create_group(
        &self,
        name: &str,
        source: GroupSource,
    ) -> impl Future<Output = Result<Group, RepositoryError>> + Send;

    /// Rename a group. Returns whether the group existed.
    fn rename_group(
        &self,
        group_id: i64,
        name: &str,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Delete a group along with its membership and role grants. Returns whether
    /// the group existed.
    fn delete_group(
        &self,
        group_id: i64,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// All groups, of every source.
    fn list_groups(&self) -> impl Future<Output = Result<Vec<Group>, RepositoryError>> + Send;

    /// Replace a group's role grants with exactly `roles`.
    fn set_group_roles(
        &self,
        group_id: i64,
        roles: &[String],
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Roles granted to a group.
    fn list_group_roles(
        &self,
        group_id: i64,
    ) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// Add a user to a group. Adding an existing member is a no-op.
    fn add_group_member(
        &self,
        group_id: i64,
        user_id: i64,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Remove a user from a group. Returns whether they were a member.
    fn remove_group_member(
        &self,
        group_id: i64,
        user_id: i64,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Ids of the users stored as members of a group. Empty for a
    /// directory-sourced group, which stores no membership.
    fn list_group_member_ids(
        &self,
        group_id: i64,
    ) -> impl Future<Output = Result<Vec<i64>, RepositoryError>> + Send;

    // --- Pending Specs ---

    /// Hold a refused Provide, replacing any entry already held for the same
    /// Producer, branch and API type. Returns the entry's id.
    fn upsert_pending_spec(
        &self,
        service_id: i64,
        branch: &str,
        api_type: ApiType,
        content: &str,
        reason: &str,
        submitted_by: &str,
    ) -> impl Future<Output = Result<i64, RepositoryError>> + Send;

    /// A held Provide by id.
    fn get_pending_spec(
        &self,
        id: i64,
    ) -> impl Future<Output = Result<Option<PendingSpec>, RepositoryError>> + Send;

    /// Every held Provide, newest first.
    fn list_pending_specs(
        &self,
    ) -> impl Future<Output = Result<Vec<PendingSpec>, RepositoryError>> + Send;

    /// Discard a held Provide by id. Returns whether one was held.
    fn delete_pending_spec(
        &self,
        id: i64,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Discard whatever is held for a Producer, branch and API type.
    ///
    /// Called when a Provide succeeds for that key: the Producer has moved on,
    /// so the held submission is dead and must not survive to be applied later
    /// over the top of the change that fixed it.
    fn clear_pending_spec(
        &self,
        service_id: i64,
        branch: &str,
        api_type: ApiType,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    // --- Producer Onboarding ---

    /// Put a Producer into onboarding, or take it out.
    fn set_producer_onboarding(
        &self,
        service_id: i64,
        onboarding: bool,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Whether a Producer is in onboarding. Unknown Producers are not.
    fn is_producer_onboarding(
        &self,
        service_id: i64,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Names of the Producers currently in onboarding.
    ///
    /// The flag never expires, so this listing is the only thing that will
    /// remind an operator it is still on.
    fn list_onboarding_producers(
        &self,
    ) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    // --- Maintainer Scope ---

    /// Assign a user as maintainer of a Producer. Reassigning is a no-op.
    fn add_user_maintainer(
        &self,
        service_id: i64,
        user_id: i64,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Remove a user's maintainership. Returns whether they held it.
    fn remove_user_maintainer(
        &self,
        service_id: i64,
        user_id: i64,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Assign a group as maintainer of a Producer. Reassigning is a no-op.
    fn add_group_maintainer(
        &self,
        service_id: i64,
        group_id: i64,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Remove a group's maintainership. Returns whether it held it.
    fn remove_group_maintainer(
        &self,
        service_id: i64,
        group_id: i64,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Users assigned directly as maintainers of a Producer.
    fn list_user_maintainer_ids(
        &self,
        service_id: i64,
    ) -> impl Future<Output = Result<Vec<i64>, RepositoryError>> + Send;

    /// Groups assigned as maintainers of a Producer.
    fn list_group_maintainer_ids(
        &self,
        service_id: i64,
    ) -> impl Future<Output = Result<Vec<i64>, RepositoryError>> + Send;

    /// Whether a user maintains a Producer, directly or through a group whose
    /// membership Sanshain stores.
    ///
    /// Directory-sourced groups are resolved above the repository, where the
    /// directory can be consulted, and combined with this answer.
    fn maintains_producer(
        &self,
        user_id: i64,
        service_id: i64,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Names of the Producers a user maintains, directly or through a stored
    /// group membership.
    fn list_maintained_producers(
        &self,
        user_id: i64,
    ) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    // --- Service Tags ---

    /// Set tags for a service (replaces existing tags). Tags are additive with existing ones.
    fn add_service_tags(
        &self,
        service_id: i64,
        tags: &[String],
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Get all service tags as a map of service_name -> Vec<tag>.
    fn get_all_service_tags(
        &self,
    ) -> impl Future<Output = Result<HashMap<String, Vec<String>>, RepositoryError>> + Send;

    // --- Audit Logs ---

    /// Insert an audit log record
    fn insert_audit_log(
        &self,
        username: &str,
        log: NewAuditLog<'_>,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Get audit log records with filtering
    fn get_audit_logs(
        &self,
        filter: AuditLogFilter,
    ) -> impl Future<Output = Result<Vec<AuditLogEntry>, RepositoryError>> + Send;

    /// Get the recent audit log records, ordered by timestamp DESC
    fn get_recent_audit_logs(
        &self,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<AuditLogEntry>, RepositoryError>> + Send;

    // --- User Favorites ---

    /// Get user favorites by item type ('service' or 'client')
    fn get_user_favorites(
        &self,
        user_id: i64,
        item_type: &str,
    ) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// Add a user favorite
    fn add_user_favorite(
        &self,
        user_id: i64,
        item_type: &str,
        item_name: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Remove a user favorite
    fn remove_user_favorite(
        &self,
        user_id: i64,
        item_type: &str,
        item_name: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// List all branches with their last modified timestamp metadata
    fn list_branches_with_metadata(
        &self,
    ) -> impl Future<Output = Result<Vec<BranchMetadata>, RepositoryError>> + Send;

    /// Last-published time per `(service_name, branch_name)`.
    ///
    /// Returns `(service_name, branch_name, updated_at)` rows. `updated_at` tracks
    /// the last publishing change to the branch (see `apply_spec_changes`), not reads.
    fn list_branch_last_published(
        &self,
    ) -> impl Future<Output = Result<Vec<(String, String, String)>, RepositoryError>> + Send;

    /// Non-deleted endpoint count per `(service_name, branch_name)`, for every
    /// branch (including ones with zero endpoints — callers filter as needed).
    ///
    /// Returns `(service_name, branch_name, count)` rows.
    fn list_branch_endpoint_counts(
        &self,
    ) -> impl Future<Output = Result<Vec<(String, String, i64)>, RepositoryError>> + Send;

    // --- AsyncAPI Channel Message Contracts (item #20) ---

    /// Get the message-level channel contract for `(branch, channel, message)`, if any.
    fn get_channel_message_contract(
        &self,
        branch_name: &str,
        channel: &str,
        message_name: &str,
    ) -> impl Future<Output = Result<Option<ChannelMessageContract>, RepositoryError>> + Send;

    /// Insert or replace a channel message contract (owner service and payload).
    fn upsert_channel_message_contract(
        &self,
        contract: &ChannelMessageContract,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Delete the channel message contract for `(branch, channel, message)`.
    fn delete_channel_message_contract(
        &self,
        branch_name: &str,
        channel: &str,
        message_name: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// List all channel message contracts registered on a branch (sorted by channel, message).
    fn list_channel_message_contracts(
        &self,
        branch_name: &str,
    ) -> impl Future<Output = Result<Vec<ChannelMessageContract>, RepositoryError>> + Send;

    /// Delete channel message contracts whose `branch_name` is not in `live_branches`.
    /// Used by the periodic cleanup task to drop rows for removed/stale branches.
    /// Returns the number of deleted rows.
    fn delete_orphaned_channel_message_contracts(
        &self,
        live_branches: &[String],
    ) -> impl Future<Output = Result<u64, RepositoryError>> + Send;
}
