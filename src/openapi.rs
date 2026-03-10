use openapiv3::{OpenAPI, PathItem, ReferenceOr};
use serde_yaml;

pub struct EndpointSpec {
    pub path: String,
    pub method: String,
    pub yaml_content: String,
}

pub fn split_openapi(yaml_str: &str) -> Result<Vec<EndpointSpec>, String> {
    let openapi: OpenAPI = serde_yaml::from_str(yaml_str)
        .map_err(|e| format!("Failed to parse OpenAPI YAML: {}", e))?;

    let mut endpoints = Vec::new();

    for (path, path_item_ref) in &openapi.paths.paths {
        let path_item = match path_item_ref {
            ReferenceOr::Item(item) => item,
            ReferenceOr::Reference { reference: _ } => {
                // In a real implementation, we should resolve references.
                // For now, we skip or return an error if paths are references (uncommon for top-level paths).
                continue;
            }
        };

        let methods = get_methods(path_item);
        for (method, operation) in methods {
            // Create a minimal OpenAPI spec for this specific endpoint
            let mut single_endpoint_openapi = OpenAPI {
                openapi: openapi.openapi.clone(),
                info: openapi.info.clone(),
                servers: openapi.servers.clone(),
                ..Default::default()
            };

            // Versioning is handled via path, so we can normalize the version in the snippet to avoid spurious conflicts.
            // Actually, maybe it's better to keep it, but the user says "version must be part of the path".
            // If the user changes the version in 'info', should that be a conflict?
            // User: "the version must be part of the path like api/v1.0/create/users... we just demand a different path if the dto changes"
            // This suggests that changes to the 'info' block version shouldn't necessarily trigger a conflict if the path is the same.
            // But if they change the DTO, it IS a conflict.
            
            // To focus only on DTO/Path changes, we could normalize 'info.version' or even 'info' entirely.
            // Let's see if we should just keep it as is and fix the test.
            // The user might want to see the correct version in the snippet.
            
            let mut paths = openapi.paths.paths.clone();
            paths.clear();
            
            let mut new_path_item = PathItem::default();
            match method.as_str() {
                "get" => new_path_item.get = Some(operation.clone()),
                "post" => new_path_item.post = Some(operation.clone()),
                "put" => new_path_item.put = Some(operation.clone()),
                "delete" => new_path_item.delete = Some(operation.clone()),
                "options" => new_path_item.options = Some(operation.clone()),
                "head" => new_path_item.head = Some(operation.clone()),
                "patch" => new_path_item.patch = Some(operation.clone()),
                "trace" => new_path_item.trace = Some(operation.clone()),
                _ => continue,
            }
            
            single_endpoint_openapi.paths.paths.insert(path.clone(), ReferenceOr::Item(new_path_item));
            
            // Crucial: Include only necessary components/schemas.
            // Simplified: for now, include all components if they exist.
            // Better: extract only the ones used by this operation.
            single_endpoint_openapi.components = openapi.components.clone();

            let endpoint_yaml = serde_yaml::to_string(&single_endpoint_openapi)
                .map_err(|e| format!("Failed to serialize endpoint YAML: {}", e))?;

            endpoints.push(EndpointSpec {
                path: path.clone(),
                method: method.to_uppercase(),
                yaml_content: endpoint_yaml,
            });
        }
    }

    Ok(endpoints)
}

fn get_methods(path_item: &PathItem) -> Vec<(String, &openapiv3::Operation)> {
    let mut methods = Vec::new();
    if let Some(op) = &path_item.get { methods.push(("get".to_string(), op)); }
    if let Some(op) = &path_item.post { methods.push(("post".to_string(), op)); }
    if let Some(op) = &path_item.put { methods.push(("put".to_string(), op)); }
    if let Some(op) = &path_item.delete { methods.push(("delete".to_string(), op)); }
    if let Some(op) = &path_item.options { methods.push(("options".to_string(), op)); }
    if let Some(op) = &path_item.head { methods.push(("head".to_string(), op)); }
    if let Some(op) = &path_item.patch { methods.push(("patch".to_string(), op)); }
    if let Some(op) = &path_item.trace { methods.push(("trace".to_string(), op)); }
    methods
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_single_endpoint() {
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
        let result = split_openapi(yaml).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "/users");
        assert_eq!(result[0].method, "GET");
        assert!(result[0].yaml_content.contains("/users"));
    }

    #[test]
    fn test_split_multiple_endpoints() {
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
    post:
      responses:
        '201':
          description: Created
  /items:
    delete:
      responses:
        '204':
          description: Deleted
"#;
        let result = split_openapi(yaml).unwrap();
        assert_eq!(result.len(), 3);
        let methods: Vec<(&str, &str)> = result.iter().map(|e| (e.path.as_str(), e.method.as_str())).collect();
        assert!(methods.contains(&("/users", "GET")));
        assert!(methods.contains(&("/users", "POST")));
        assert!(methods.contains(&("/items", "DELETE")));
    }

    #[test]
    fn test_split_invalid_yaml() {
        let result = split_openapi("not valid yaml: [[[");
        assert!(result.is_err());
    }

    #[test]
    fn test_split_includes_components() {
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
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/User'
components:
  schemas:
    User:
      type: object
      properties:
        name:
          type: string
"#;
        let result = split_openapi(yaml).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].yaml_content.contains("User"));
    }

    #[test]
    fn test_split_empty_paths() {
        let yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths: {}
"#;
        let result = split_openapi(yaml).unwrap();
        assert!(result.is_empty());
    }
}
