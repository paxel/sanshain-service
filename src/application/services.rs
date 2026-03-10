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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ports::RepositoryError;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct MockRepo {
        services: Mutex<HashMap<String, i64>>,
        branches: Mutex<HashMap<(i64, String), i64>>,
        endpoints: Mutex<HashMap<i64, Vec<EndpointRecord>>>,
        clients: Mutex<HashMap<String, i64>>,
        next_id: Mutex<i64>,
    }

    impl MockRepo {
        fn new() -> Self {
            Self {
                services: Mutex::new(HashMap::new()),
                branches: Mutex::new(HashMap::new()),
                endpoints: Mutex::new(HashMap::new()),
                clients: Mutex::new(HashMap::new()),
                next_id: Mutex::new(1),
            }
        }

        fn next_id(&self) -> i64 {
            let mut id = self.next_id.lock().unwrap();
            let current = *id;
            *id += 1;
            current
        }
    }

    impl SpecRepository for MockRepo {
        async fn ensure_service(&self, name: &str) -> Result<i64, RepositoryError> {
            let mut services = self.services.lock().unwrap();
            if let Some(&id) = services.get(name) {
                return Ok(id);
            }
            let id = self.next_id();
            services.insert(name.to_string(), id);
            Ok(id)
        }

        async fn ensure_branch(&self, service_id: i64, branch_name: &str) -> Result<i64, RepositoryError> {
            let mut branches = self.branches.lock().unwrap();
            let key = (service_id, branch_name.to_string());
            if let Some(&id) = branches.get(&key) {
                return Ok(id);
            }
            let id = self.next_id();
            branches.insert(key, id);
            Ok(id)
        }

        async fn get_endpoints_for_branch(&self, branch_id: i64) -> Result<Vec<EndpointRecord>, RepositoryError> {
            let endpoints = self.endpoints.lock().unwrap();
            Ok(endpoints.get(&branch_id).cloned().unwrap_or_default())
        }

        async fn insert_endpoint(&self, branch_id: i64, endpoint: &EndpointRecord) -> Result<(), RepositoryError> {
            let mut endpoints = self.endpoints.lock().unwrap();
            let mut record = endpoint.clone();
            record.id = Some(self.next_id());
            endpoints.entry(branch_id).or_default().push(record);
            Ok(())
        }

        async fn ensure_client(&self, name: &str) -> Result<i64, RepositoryError> {
            let mut clients = self.clients.lock().unwrap();
            if let Some(&id) = clients.get(name) {
                return Ok(id);
            }
            let id = self.next_id();
            clients.insert(name.to_string(), id);
            Ok(id)
        }

        async fn find_endpoint(
            &self,
            service_id: i64,
            branch_name: &str,
            path: &str,
            method: &str,
        ) -> Result<Option<(i64, String)>, RepositoryError> {
            let branches = self.branches.lock().unwrap();
            let key = (service_id, branch_name.to_string());
            if let Some(&branch_id) = branches.get(&key) {
                let endpoints = self.endpoints.lock().unwrap();
                if let Some(eps) = endpoints.get(&branch_id) {
                    for ep in eps {
                        if ep.path == path && ep.method == method {
                            return Ok(Some((ep.id.unwrap(), ep.yaml_content.clone())));
                        }
                    }
                }
            }
            Ok(None)
        }

        async fn record_dependency(
            &self,
            _client_id: i64,
            _endpoint_id: Option<i64>,
            _service_id: i64,
            _branch_name: &str,
            _path: &str,
            _method: &str,
        ) -> Result<(), RepositoryError> {
            Ok(())
        }

        async fn get_report(&self, branch: &str) -> Result<DependencyReport, RepositoryError> {
            Ok(DependencyReport {
                branch: branch.to_string(),
                dependency_graph: vec![],
                unused_endpoints: vec![],
                missing_endpoints: vec![],
            })
        }
    }

    #[tokio::test]
    async fn test_provide_spec_success() {
        let repo = MockRepo::new();
        let yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
"#;
        let result = provide_spec(&repo, "svc", "main", yaml).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_provide_spec_idempotent() {
        let repo = MockRepo::new();
        let yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
"#;
        provide_spec(&repo, "svc", "main", yaml).await.unwrap();
        let result = provide_spec(&repo, "svc", "main", yaml).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_provide_spec_conflict_on_changed_dto() {
        let repo = MockRepo::new();
        let yaml1 = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
"#;
        let yaml2 = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    get:
      description: Changed
      responses:
        '200':
          description: OK
"#;
        provide_spec(&repo, "svc", "main", yaml1).await.unwrap();
        let result = provide_spec(&repo, "svc", "main", yaml2).await;
        assert!(matches!(result, Err(AppError::Conflict)));
    }

    #[tokio::test]
    async fn test_provide_spec_invalid_yaml() {
        let repo = MockRepo::new();
        let result = provide_spec(&repo, "svc", "main", "not valid [[[").await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[tokio::test]
    async fn test_require_endpoint_not_found() {
        let repo = MockRepo::new();
        let result = require_endpoint(&repo, "client", "svc", "main", "/missing", "GET").await;
        assert!(matches!(result, Err(AppError::NotFound)));
    }

    #[tokio::test]
    async fn test_require_endpoint_found() {
        let repo = MockRepo::new();
        let yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    get:
      responses:
        '200':
          description: OK
"#;
        provide_spec(&repo, "svc", "main", yaml).await.unwrap();
        let result = require_endpoint(&repo, "client", "svc", "main", "/users", "GET").await;
        assert!(result.is_ok());
        assert!(result.unwrap().contains("/users"));
    }

    #[test]
    fn test_render_report_markdown_empty() {
        let report = DependencyReport {
            branch: "main".to_string(),
            dependency_graph: vec![],
            unused_endpoints: vec![],
            missing_endpoints: vec![],
        };
        let md = render_report_markdown(&report);
        assert!(md.contains("# SanShain Dependency Report: Branch `main`"));
        assert!(md.contains("Total Dependencies: 0"));
        assert!(md.contains("No active dependencies recorded"));
    }

    #[test]
    fn test_render_report_markdown_with_data() {
        let report = DependencyReport {
            branch: "dev".to_string(),
            dependency_graph: vec![DependencyInfo {
                client: "web".to_string(),
                service: "api".to_string(),
                path: "/users".to_string(),
                method: "GET".to_string(),
            }],
            unused_endpoints: vec![EndpointInfo {
                service: "api".to_string(),
                path: "/old".to_string(),
                method: "DELETE".to_string(),
            }],
            missing_endpoints: vec![MissingEndpointInfo {
                client: "web".to_string(),
                service: "api".to_string(),
                path: "/new".to_string(),
                method: "POST".to_string(),
            }],
        };
        let md = render_report_markdown(&report);
        assert!(md.contains("| web | api | `/users` | `GET` |"));
        assert!(md.contains("| api | `/old` | `DELETE` |"));
        assert!(md.contains("| web | api | `/new` | `POST` |"));
    }
}
