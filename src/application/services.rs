use std::collections::HashMap;

use crate::domain::models::*;
use crate::domain::ports::{RepositoryError, SpecRepository};
use crate::openapi;

#[derive(Debug)]
pub enum AppError {
    BadRequest(String),
    Conflict,
    NotFound,
    Internal(String),
}

impl From<RepositoryError> for AppError {
    fn from(e: RepositoryError) -> Self {
        match e {
            RepositoryError::NotFound => AppError::NotFound,
            RepositoryError::Conflict => AppError::Conflict,
            RepositoryError::Internal(msg) => AppError::Internal(msg),
        }
    }
}

pub async fn provide_spec(
    repo: &impl SpecRepository,
    servicename: &str,
    branch: &str,
    openapi_yaml: &str,
) -> Result<(), AppError> {
    let endpoints = openapi::split_openapi(openapi_yaml)
        .map_err(|e| AppError::BadRequest(e))?;

    let service_id = repo.ensure_service(servicename).await?;
    let branch_id = repo.ensure_branch(service_id, branch).await?;

    let existing = repo.get_endpoints_for_branch(branch_id).await?;
    let mut existing_map: HashMap<(String, String), String> = existing
        .into_iter()
        .map(|e| ((e.path, e.method), e.yaml_content))
        .collect();

    let mut to_insert = Vec::new();

    for endpoint in endpoints {
        let key = (endpoint.path.clone(), endpoint.method.clone());
        if let Some(existing_yaml) = existing_map.remove(&key) {
            if existing_yaml != endpoint.yaml_content {
                tracing::warn!(
                    "Rejected update for {} {}: DTO changed but path remained the same",
                    endpoint.method,
                    endpoint.path
                );
                return Err(AppError::Conflict);
            }
        } else {
            to_insert.push(EndpointRecord {
                id: None,
                path: endpoint.path,
                method: endpoint.method,
                yaml_content: endpoint.yaml_content,
            });
        }
    }

    for endpoint in &to_insert {
        repo.insert_endpoint(branch_id, endpoint).await?;
    }

    Ok(())
}

pub async fn require_endpoint(
    repo: &impl SpecRepository,
    clientname: &str,
    servicename: &str,
    branch: &str,
    path: &str,
    method: &str,
) -> Result<String, AppError> {
    let client_id = repo.ensure_client(clientname).await?;
    let service_id = repo.ensure_service(servicename).await?;

    let method_upper = method.to_uppercase();
    let endpoint = repo.find_endpoint(service_id, branch, path, &method_upper).await?;

    let endpoint_id = endpoint.as_ref().map(|e| e.0);
    let yaml_content = endpoint.map(|e| e.1);

    repo.record_dependency(client_id, endpoint_id, service_id, branch, path, &method_upper).await?;

    yaml_content.ok_or(AppError::NotFound)
}

pub async fn generate_report(
    repo: &impl SpecRepository,
    branch: &str,
) -> Result<DependencyReport, AppError> {
    Ok(repo.get_report(branch).await?)
}

pub fn render_report_markdown(report: &DependencyReport) -> String {
    let mut md = format!("# SanShain Dependency Report: Branch `{}`\n\n", report.branch);

    md.push_str("## Summary\n");
    md.push_str(&format!("- Total Dependencies: {}\n", report.dependency_graph.len()));
    md.push_str(&format!("- Unused Endpoints: {}\n", report.unused_endpoints.len()));
    md.push_str(&format!("- Missing Requirements: {}\n\n", report.missing_endpoints.len()));

    md.push_str("## Dependency Graph\n");
    if report.dependency_graph.is_empty() {
        md.push_str("No active dependencies recorded for this branch.\n\n");
    } else {
        md.push_str("| Client | Service | Path | Method |\n");
        md.push_str("| --- | --- | --- | --- |\n");
        for dep in &report.dependency_graph {
            md.push_str(&format!("| {} | {} | `{}` | `{}` |\n", dep.client, dep.service, dep.path, dep.method));
        }
        md.push_str("\n");
    }

    md.push_str("## Unused Endpoints\n");
    md.push_str("> Endpoints that are provided by a service but have no recorded client requirements.\n\n");
    if report.unused_endpoints.is_empty() {
        md.push_str("All provided endpoints are in use.\n\n");
    } else {
        md.push_str("| Service | Path | Method |\n");
        md.push_str("| --- | --- | --- |\n");
        for ep in &report.unused_endpoints {
            md.push_str(&format!("| {} | `{}` | `{}` |\n", ep.service, ep.path, ep.method));
        }
        md.push_str("\n");
    }

    md.push_str("## Missing Requirements\n");
    md.push_str("> Requirements from clients for endpoints that do not exist in this branch.\n\n");
    if report.missing_endpoints.is_empty() {
        md.push_str("No missing requirements identified.\n\n");
    } else {
        md.push_str("| Client | Service | Path | Method |\n");
        md.push_str("| --- | --- | --- | --- |\n");
        for ep in &report.missing_endpoints {
            md.push_str(&format!("| {} | {} | `{}` | `{}` |\n", ep.client, ep.service, ep.path, ep.method));
        }
        md.push_str("\n");
    }

    md
}
