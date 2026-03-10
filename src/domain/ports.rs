use crate::domain::models::*;
use std::future::Future;

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
}
