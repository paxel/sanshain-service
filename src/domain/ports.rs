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
}
