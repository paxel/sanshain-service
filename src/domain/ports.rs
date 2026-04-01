use crate::domain::models::*;
use std::future::Future;

/// Error type for authentication provider operations.
#[derive(Debug)]
pub enum AuthProviderError {
    InvalidCredentials,
    ConnectionFailed(String),
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

#[derive(Debug)]
pub enum RepositoryError {
    NotFound,
    Conflict,
    Internal(String),
}

/// Port for all persistence operations required by the application layer.
pub trait SpecRepository: Send + Sync {
    /// Ensure a service exists and return its ID.
    fn ensure_service(&self, name: &str) -> impl Future<Output = Result<i64, RepositoryError>> + Send;

    /// Ensure a branch exists for a service and return its ID.
    fn ensure_branch(&self, service_id: i64, branch_name: &str) -> impl Future<Output = Result<i64, RepositoryError>> + Send;

    /// Fetch all existing endpoints for a branch.
    fn get_endpoints_for_branch(&self, branch_id: i64) -> impl Future<Output = Result<Vec<EndpointRecord>, RepositoryError>> + Send;

    /// Insert a new endpoint for a branch.
    fn insert_endpoint(&self, branch_id: i64, endpoint: &EndpointRecord) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Ensure a client exists and return its ID.
    fn ensure_client(&self, name: &str) -> impl Future<Output = Result<i64, RepositoryError>> + Send;

    /// Find an endpoint by service, branch, path, and method. Returns (endpoint_id, yaml_content).
    fn find_endpoint(
        &self,
        service_id: i64,
        branch_name: &str,
        path: &str,
        method: &str,
    ) -> impl Future<Output = Result<Option<(i64, String)>, RepositoryError>> + Send;

    /// Record a client dependency on an endpoint.
    fn record_dependency(
        &self,
        client_id: i64,
        endpoint_id: Option<i64>,
        service_id: i64,
        branch_name: &str,
        path: &str,
        method: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Get the full dependency report for a branch.
    fn get_report(&self, branch: &str) -> impl Future<Output = Result<DependencyReport, RepositoryError>> + Send;

    /// Check if a branch name matches any protected branch pattern.
    fn is_branch_protected(&self, branch_name: &str) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Add a protected branch pattern.
    fn add_protected_branch(&self, pattern: &str) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Remove a protected branch pattern.
    fn remove_protected_branch(&self, pattern: &str) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// List all protected branch patterns.
    fn list_protected_branches(&self) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// Update an existing endpoint's YAML content.
    fn update_endpoint(&self, branch_id: i64, path: &str, method: &str, yaml_content: &str) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Soft-delete an endpoint (mark as deleted). Used on protected branches to preserve history.
    fn soft_delete_endpoint(&self, branch_id: i64, path: &str, method: &str) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Hard-delete endpoints by branch, path, and method. Used on non-protected branches.
    fn hard_delete_endpoint(&self, branch_id: i64, path: &str, method: &str) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Check if a soft-deleted endpoint exists for the given branch, path, and method.
    fn is_endpoint_deleted(&self, branch_id: i64, path: &str, method: &str) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Delete a service and all its branches, endpoints, and related dependencies.
    fn delete_service(&self, name: &str) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Delete a branch (by service name and branch name) and its endpoints and related dependencies.
    fn delete_branch(&self, service_name: &str, branch_name: &str) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Delete a client and all its dependencies.
    fn delete_client(&self, name: &str) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// List all services.
    fn list_services(&self) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// List all branches for a service.
    fn list_branches(&self, service_name: &str) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// List all clients.
    fn list_clients(&self) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// List all branches that a client has dependencies on.
    fn list_client_branches(&self, client_name: &str) -> impl Future<Output = Result<Vec<String>, RepositoryError>> + Send;

    /// List all endpoints a client depends on for a given branch.
    fn list_client_endpoints(&self, client_name: &str, branch: &str) -> impl Future<Output = Result<Vec<ClientEndpointInfo>, RepositoryError>> + Send;

    // --- Auth ---

    /// Count total users.
    fn user_count(&self) -> impl Future<Output = Result<i64, RepositoryError>> + Send;

    /// Find a user by username.
    fn find_user(&self, username: &str) -> impl Future<Output = Result<Option<User>, RepositoryError>> + Send;

    /// Create a new user.
    fn create_user(&self, username: &str, password_hash: &str, is_admin: bool, approved: bool) -> impl Future<Output = Result<User, RepositoryError>> + Send;

    /// Update a user's password hash.
    fn update_password(&self, user_id: i64, new_hash: &str) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// List all users (id, username, is_admin, approved).
    fn list_users(&self) -> impl Future<Output = Result<Vec<User>, RepositoryError>> + Send;

    /// Approve a user by ID.
    fn approve_user(&self, user_id: i64) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Delete a user by ID.
    fn delete_user(&self, user_id: i64) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Create a session for a user, returning the session with a generated token.
    fn create_session(&self, user_id: i64, expires_at: &str) -> impl Future<Output = Result<Session, RepositoryError>> + Send;

    /// Validate a session token, returning the user and session if valid and not expired.
    fn validate_session(&self, token: &str) -> impl Future<Output = Result<Option<(User, Session)>, RepositoryError>> + Send;

    /// Delete a session (logout).
    fn delete_session(&self, token: &str) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Get a setting value by key.
    fn get_setting(&self, key: &str) -> impl Future<Output = Result<Option<String>, RepositoryError>> + Send;

    /// Set a setting value.
    fn set_setting(&self, key: &str, value: &str) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    // --- API Tokens ---

    /// Create an API token (stores the hash, not the raw token).
    fn create_api_token(&self, id: &str, user_id: i64, name: &str, token_hash: &str, created_at: &str, expires_at: &str) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// List all API tokens for a user (never includes raw token).
    fn list_api_tokens(&self, user_id: i64) -> impl Future<Output = Result<Vec<ApiToken>, RepositoryError>> + Send;

    /// Delete an API token by ID (only if owned by user_id).
    fn delete_api_token(&self, token_id: &str, user_id: i64) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Validate an API token hash, returning the user if valid and not expired. Also updates last_used_at.
    fn validate_api_token(&self, token_hash: &str) -> impl Future<Output = Result<Option<User>, RepositoryError>> + Send;

    /// Delete non-protected branches that haven't been updated since the given cutoff ISO timestamp.
    /// Returns the number of deleted branches.
    fn delete_stale_branches(&self, cutoff_iso: &str) -> impl Future<Output = Result<u64, RepositoryError>> + Send;
}
