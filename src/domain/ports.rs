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

pub type EndpointDetails = (i64, String);
pub type EndpointMap = HashMap<(String, String), EndpointDetails>;

pub trait SpecRepository: Send + Sync {
    /// Get the current spec version and content hash for a service/branch.
    fn get_spec_version(
        &self,
        service_id: i64,
        branch_id: i64,
    ) -> impl Future<Output = Result<Option<(i32, String)>, RepositoryError>> + Send;

    /// Update or increment the spec version and content hash for a service/branch.
    /// Returns the new version.
    fn increment_spec_version(
        &self,
        service_id: i64,
        branch_id: i64,
        content_hash: &str,
    ) -> impl Future<Output = Result<i32, RepositoryError>> + Send;

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

    /// Ensure a client exists and return its ID.
    fn ensure_client(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<i64, RepositoryError>> + Send;

    /// Find an endpoint by service, branch, path, and method. Returns (endpoint_id, yaml_content).
    fn find_endpoint(
        &self,
        service_id: i64,
        branch_name: &str,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> impl Future<Output = Result<Option<(i64, String)>, RepositoryError>> + Send;

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

    /// Update an existing endpoint's YAML content.
    fn update_endpoint(
        &self,
        branch_id: i64,
        api_type: ApiType,
        path: &str,
        method: &str,
        yaml_content: &str,
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
    fn delete_service(
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
    fn delete_client(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// List all services.
    fn list_services(&self) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// List all services with detailed information (like fallback branch).
    fn list_services_detailed(
        &self,
    ) -> impl Future<Output = Result<Vec<ServiceSummary>, RepositoryError>> + Send;

    /// Set the fallback branch for a service.
    fn set_fallback_branch(
        &self,
        service_name: &str,
        branch: Option<&str>,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Get the fallback branch for a service.
    fn get_fallback_branch(
        &self,
        service_name: &str,
    ) -> impl Future<Output = Result<Option<String>, RepositoryError>> + Send;

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
    fn list_clients(&self) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// List all branches that a client has dependencies on.
    fn list_client_branches(
        &self,
        client_name: &str,
    ) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// List all endpoints a client depends on for a given branch.
    fn list_client_endpoints(
        &self,
        client_name: &str,
        branch: &str,
    ) -> impl Future<Output = Result<Vec<ClientEndpointInfo>, RepositoryError>> + Send;

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
        is_admin: bool,
        approved: bool,
    ) -> impl Future<Output = Result<User, RepositoryError>> + Send;

    /// Update a user's password hash.
    fn update_password(
        &self,
        user_id: i64,
        new_hash: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// List all users (id, username, is_admin, approved).
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

    /// Delete an API token by ID (only if owned by user_id).
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

    /// Apply a set of specification changes (insert, update, delete) atomically.
    fn apply_spec_changes(
        &self,
        branch_id: i64,
        changes: Vec<SpecChange>,
        is_protected: bool,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

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

    // --- Shared Contracts (Problem 2) ---

    /// Get a shared contract for a branch and endpoint.
    fn get_shared_contract(
        &self,
        branch_name: &str,
        api_type: ApiType,
        path: &str,
        method: &str,
    ) -> impl Future<Output = Result<Option<SharedContract>, RepositoryError>> + Send;

    /// Upsert a shared contract.
    fn upsert_shared_contract(
        &self,
        contract: SharedContract,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
}
