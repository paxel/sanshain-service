use openapiv3::{Components, OpenAPI, PathItem, ReferenceOr};
use regex::Regex;
use serde_yaml;
use std::collections::HashSet;

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
            
            // Include only the components/schemas actually referenced by this operation.
            single_endpoint_openapi.components = extract_used_components(
                &single_endpoint_openapi,
                &openapi.components,
            );

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

/// Extract all `$ref` strings from a YAML representation.
fn collect_refs_from_yaml(yaml: &str) -> HashSet<String> {
    let re = Regex::new(r#"\$ref:\s*'?\"?#/components/(\w+)/(\w+)'?\"?"#).unwrap();
    let mut refs = HashSet::new();
    for cap in re.captures_iter(yaml) {
        // Store as "category/name", e.g. "schemas/User"
        refs.insert(format!("{}/{}", &cap[1], &cap[2]));
    }
    refs
}

/// Given a partial OpenAPI (with paths but no components yet) and the full components,
/// return a filtered Components containing only what's transitively referenced.
fn extract_used_components(
    partial_spec: &OpenAPI,
    all_components: &Option<Components>,
) -> Option<Components> {
    let all_components = match all_components {
        Some(c) => c,
        None => return None,
    };

    // Serialize the paths portion to find initial refs
    let paths_yaml = serde_yaml::to_string(&partial_spec.paths).unwrap_or_default();
    let mut to_visit: Vec<String> = collect_refs_from_yaml(&paths_yaml).into_iter().collect();
    let mut visited: HashSet<String> = HashSet::new();

    // Transitively resolve refs from schemas
    while let Some(ref_key) = to_visit.pop() {
        if !visited.insert(ref_key.clone()) {
            continue;
        }
        // If it's a schema ref, serialize that schema and find nested refs
        if let Some(name) = ref_key.strip_prefix("schemas/") {
            if let Some(schema_ref) = all_components.schemas.get(name) {
                let schema_yaml = serde_yaml::to_string(schema_ref).unwrap_or_default();
                for nested in collect_refs_from_yaml(&schema_yaml) {
                    if !visited.contains(&nested) {
                        to_visit.push(nested);
                    }
                }
            }
        }
        // Similarly for responses
        if let Some(name) = ref_key.strip_prefix("responses/") {
            if let Some(resp_ref) = all_components.responses.get(name) {
                let resp_yaml = serde_yaml::to_string(resp_ref).unwrap_or_default();
                for nested in collect_refs_from_yaml(&resp_yaml) {
                    if !visited.contains(&nested) {
                        to_visit.push(nested);
                    }
                }
            }
        }
        // parameters
        if let Some(name) = ref_key.strip_prefix("parameters/") {
            if let Some(param_ref) = all_components.parameters.get(name) {
                let param_yaml = serde_yaml::to_string(param_ref).unwrap_or_default();
                for nested in collect_refs_from_yaml(&param_yaml) {
                    if !visited.contains(&nested) {
                        to_visit.push(nested);
                    }
                }
            }
        }
        // requestBodies
        if let Some(name) = ref_key.strip_prefix("requestBodies/") {
            if let Some(rb_ref) = all_components.request_bodies.get(name) {
                let rb_yaml = serde_yaml::to_string(rb_ref).unwrap_or_default();
                for nested in collect_refs_from_yaml(&rb_yaml) {
                    if !visited.contains(&nested) {
                        to_visit.push(nested);
                    }
                }
            }
        }
    }

    // Build filtered components
    let mut filtered = Components::default();
    for key in &visited {
        if let Some(name) = key.strip_prefix("schemas/") {
            if let Some(s) = all_components.schemas.get(name) {
                filtered.schemas.insert(name.to_string(), s.clone());
            }
        } else if let Some(name) = key.strip_prefix("responses/") {
            if let Some(r) = all_components.responses.get(name) {
                filtered.responses.insert(name.to_string(), r.clone());
            }
        } else if let Some(name) = key.strip_prefix("parameters/") {
            if let Some(p) = all_components.parameters.get(name) {
                filtered.parameters.insert(name.to_string(), p.clone());
            }
        } else if let Some(name) = key.strip_prefix("requestBodies/") {
            if let Some(rb) = all_components.request_bodies.get(name) {
                filtered.request_bodies.insert(name.to_string(), rb.clone());
            }
        } else if let Some(name) = key.strip_prefix("headers/") {
            if let Some(h) = all_components.headers.get(name) {
                filtered.headers.insert(name.to_string(), h.clone());
            }
        } else if let Some(name) = key.strip_prefix("securitySchemes/") {
            if let Some(ss) = all_components.security_schemes.get(name) {
                filtered.security_schemes.insert(name.to_string(), ss.clone());
            }
        }
    }

    // Return None if nothing was referenced
    if filtered.schemas.is_empty()
        && filtered.responses.is_empty()
        && filtered.parameters.is_empty()
        && filtered.request_bodies.is_empty()
        && filtered.headers.is_empty()
        && filtered.security_schemes.is_empty()
    {
        None
    } else {
        Some(filtered)
    }
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

    #[test]
    fn test_split_excludes_unused_schemas() {
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
    Order:
      type: object
      properties:
        id:
          type: integer
"#;
        let result = split_openapi(yaml).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].yaml_content.contains("User"));
        assert!(!result[0].yaml_content.contains("Order"));
    }

    #[test]
    fn test_split_includes_transitive_refs() {
        let yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /orders:
    get:
      responses:
        '200':
          description: OK
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Order'
components:
  schemas:
    Order:
      type: object
      properties:
        customer:
          $ref: '#/components/schemas/Customer'
    Customer:
      type: object
      properties:
        address:
          $ref: '#/components/schemas/Address'
    Address:
      type: object
      properties:
        street:
          type: string
    Unrelated:
      type: object
      properties:
        foo:
          type: string
"#;
        let result = split_openapi(yaml).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].yaml_content.contains("Order"));
        assert!(result[0].yaml_content.contains("Customer"));
        assert!(result[0].yaml_content.contains("Address"));
        assert!(!result[0].yaml_content.contains("Unrelated"));
    }

    #[test]
    fn test_split_each_endpoint_gets_own_schemas() {
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
  /orders:
    get:
      responses:
        '200':
          description: OK
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Order'
components:
  schemas:
    User:
      type: object
      properties:
        name:
          type: string
    Order:
      type: object
      properties:
        id:
          type: integer
"#;
        let result = split_openapi(yaml).unwrap();
        assert_eq!(result.len(), 2);
        let users_ep = result.iter().find(|e| e.path == "/users").unwrap();
        let orders_ep = result.iter().find(|e| e.path == "/orders").unwrap();
        assert!(users_ep.yaml_content.contains("User"));
        assert!(!users_ep.yaml_content.contains("Order"));
        assert!(orders_ep.yaml_content.contains("Order"));
        assert!(!orders_ep.yaml_content.contains("User"));
    }

    #[test]
    fn test_split_no_components_when_none_referenced() {
        let yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /health:
    get:
      responses:
        '200':
          description: OK
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
        assert!(!result[0].yaml_content.contains("User"));
        assert!(!result[0].yaml_content.contains("components"));
    }
}
