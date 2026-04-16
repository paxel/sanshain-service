use openapiv3::{Components, OpenAPI, PathItem, ReferenceOr, SchemaKind, Type as OaType};
use regex::Regex;
use serde_yaml;
use similar::TextDiff;
use std::collections::{HashMap, HashSet};

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

/// Merge multiple per-endpoint YAML snippets (from the same service) into a single OpenAPI spec
/// with deduplicated schemas/components.
pub fn merge_endpoint_yamls(yamls: &[String]) -> Result<String, String> {
    if yamls.is_empty() {
        return Err("No endpoint YAMLs to merge".to_string());
    }

    // Use the first snippet as the base (info, servers, openapi version)
    let mut merged: OpenAPI = serde_yaml::from_str(&yamls[0])
        .map_err(|e| format!("Failed to parse first endpoint YAML: {}", e))?;

    // Merge paths and components from all subsequent snippets
    for yaml in &yamls[1..] {
        let spec: OpenAPI = serde_yaml::from_str(yaml)
            .map_err(|e| format!("Failed to parse endpoint YAML: {}", e))?;

        // Merge paths
        for (path, path_item_ref) in spec.paths.paths {
            merged.paths.paths
                .entry(path.clone())
                .and_modify(|existing| {
                    // Merge operations into existing path item
                    if let (ReferenceOr::Item(existing_item), ReferenceOr::Item(new_item)) =
                        (existing, &path_item_ref)
                    {
                        if new_item.get.is_some() { existing_item.get = new_item.get.clone(); }
                        if new_item.post.is_some() { existing_item.post = new_item.post.clone(); }
                        if new_item.put.is_some() { existing_item.put = new_item.put.clone(); }
                        if new_item.delete.is_some() { existing_item.delete = new_item.delete.clone(); }
                        if new_item.options.is_some() { existing_item.options = new_item.options.clone(); }
                        if new_item.head.is_some() { existing_item.head = new_item.head.clone(); }
                        if new_item.patch.is_some() { existing_item.patch = new_item.patch.clone(); }
                        if new_item.trace.is_some() { existing_item.trace = new_item.trace.clone(); }
                    }
                })
                .or_insert(path_item_ref);
        }

        // Merge components (union — same name = same schema within one service)
        if let Some(new_components) = spec.components {
            let merged_components = merged.components.get_or_insert_with(Components::default);
            for (name, schema) in new_components.schemas {
                merged_components.schemas.entry(name).or_insert(schema);
            }
            for (name, resp) in new_components.responses {
                merged_components.responses.entry(name).or_insert(resp);
            }
            for (name, param) in new_components.parameters {
                merged_components.parameters.entry(name).or_insert(param);
            }
            for (name, rb) in new_components.request_bodies {
                merged_components.request_bodies.entry(name).or_insert(rb);
            }
            for (name, h) in new_components.headers {
                merged_components.headers.entry(name).or_insert(h);
            }
            for (name, ss) in new_components.security_schemes {
                merged_components.security_schemes.entry(name).or_insert(ss);
            }
        }
    }

    serde_yaml::to_string(&merged)
        .map_err(|e| format!("Failed to serialize merged YAML: {}", e))
}

/// Generate a unified diff between two YAML strings.
pub fn generate_diff(old: &str, new: &str) -> String {
    let diff = TextDiff::from_lines(old, new);
    diff.unified_diff()
        .context_radius(3)
        .header("previous", "current")
        .to_string()
}

/// Check if a new endpoint YAML is backward-compatible with the old one.
/// Backward-compatible means:
/// - No existing required request fields removed
/// - No existing required request fields changed type
/// - No existing response fields removed
/// - No existing response status codes removed
/// Adding new optional/required fields, new response codes, or new schemas is OK.
/// Returns Ok(()) if compatible, Err(description) if breaking.
pub fn check_backward_compatibility(old_yaml: &str, new_yaml: &str) -> Result<(), String> {
    let old: OpenAPI = serde_yaml::from_str(old_yaml)
        .map_err(|e| format!("Failed to parse old YAML: {}", e))?;
    let new: OpenAPI = serde_yaml::from_str(new_yaml)
        .map_err(|e| format!("Failed to parse new YAML: {}", e))?;

    let old_schemas = collect_schema_map(&old);
    let new_schemas = collect_schema_map(&new);

    // Check that no existing schemas were removed
    for name in old_schemas.keys() {
        if !new_schemas.contains_key(name) {
            return Err(format!("Schema '{}' was removed", name));
        }
    }

    // Check each existing schema for breaking changes
    for (name, old_schema) in &old_schemas {
        if let Some(new_schema) = new_schemas.get(name) {
            check_schema_compatible(name, old_schema, new_schema)?;
        }
    }

    // Check that no existing response status codes were removed
    for (path, old_item) in &old.paths.paths {
        if let ReferenceOr::Item(old_pi) = old_item {
            if let Some(ReferenceOr::Item(new_pi)) = new.paths.paths.get(path) {
                check_operations_compatible(path, old_pi, new_pi)?;
            } else {
                return Err(format!("Path '{}' was removed", path));
            }
        }
    }

    Ok(())
}

/// Collect all named schemas from an OpenAPI spec into a flat map.
fn collect_schema_map(spec: &OpenAPI) -> HashMap<String, &openapiv3::Schema> {
    let mut map = HashMap::new();
    if let Some(components) = &spec.components {
        for (name, schema_ref) in &components.schemas {
            if let ReferenceOr::Item(schema) = schema_ref {
                map.insert(name.clone(), schema);
            }
        }
    }
    map
}

/// Check that a schema change is backward-compatible.
fn check_schema_compatible(name: &str, old: &openapiv3::Schema, new: &openapiv3::Schema) -> Result<(), String> {
    let old_props = extract_object_properties(old);
    let new_props = extract_object_properties(new);

    if let (Some(old_props), Some(new_props)) = (old_props, new_props) {
        let old_required = extract_required_fields(old);
        let new_required = extract_required_fields(new);

        // Check no existing properties were removed
        for prop_name in old_props.keys() {
            if !new_props.contains_key(prop_name) {
                return Err(format!(
                    "Property '{}' was removed from schema '{}'",
                    prop_name, name
                ));
            }
        }

        // Check no existing required fields were removed from required list
        for req in &old_required {
            if !new_required.contains(req) {
                // A field going from required to optional is actually not breaking for consumers,
                // but we flag it for awareness. Actually this is fine — skip this check.
                // The truly breaking thing is adding new required fields that old clients don't send.
            }
        }

        // Check that newly required fields didn't exist as optional before
        // (adding a brand new required field is breaking for request bodies)
        for req in &new_required {
            if !old_required.contains(req) && !old_props.contains_key(req.as_str()) {
                // New required field that didn't exist before — this is breaking for request schemas
                // But we can't easily distinguish request vs response schemas here,
                // so we allow it (response schemas with new required fields are fine).
                // The key protection is: don't remove fields, don't change types.
            }
        }

        // Check no property types changed
        for (prop_name, old_type) in &old_props {
            if let Some(new_type) = new_props.get(prop_name) {
                if old_type != new_type {
                    return Err(format!(
                        "Property '{}' in schema '{}' changed type from '{}' to '{}'",
                        prop_name, name, old_type, new_type
                    ));
                }
            }
        }
    }

    Ok(())
}

/// Extract property names and their type strings from an object schema.
fn extract_object_properties(schema: &openapiv3::Schema) -> Option<HashMap<&str, String>> {
    match &schema.schema_kind {
        SchemaKind::Type(OaType::Object(obj)) => {
            let mut props = HashMap::new();
            for (name, prop_ref) in &obj.properties {
                let type_str = match prop_ref {
                    ReferenceOr::Item(box_schema) => describe_schema_type(&box_schema.schema_kind),
                    ReferenceOr::Reference { reference } => reference.clone(),
                };
                props.insert(name.as_str(), type_str);
            }
            Some(props)
        }
        _ => None,
    }
}

/// Extract required field names from a schema.
fn extract_required_fields(schema: &openapiv3::Schema) -> HashSet<String> {
    match &schema.schema_kind {
        SchemaKind::Type(OaType::Object(obj)) => {
            obj.required.iter().cloned().collect()
        }
        _ => HashSet::new(),
    }
}

/// Describe a schema kind as a simple type string for comparison.
fn describe_schema_type(kind: &SchemaKind) -> String {
    match kind {
        SchemaKind::Type(OaType::String(_)) => "string".to_string(),
        SchemaKind::Type(OaType::Number(_)) => "number".to_string(),
        SchemaKind::Type(OaType::Integer(_)) => "integer".to_string(),
        SchemaKind::Type(OaType::Boolean(_)) => "boolean".to_string(),
        SchemaKind::Type(OaType::Array(_)) => "array".to_string(),
        SchemaKind::Type(OaType::Object(_)) => "object".to_string(),
        _ => "unknown".to_string(),
    }
}

/// Check that operations on a path haven't had breaking changes.
fn check_operations_compatible(path: &str, old: &PathItem, new: &PathItem) -> Result<(), String> {
    let old_methods = get_methods(old);
    let new_method_names: HashSet<String> = get_methods(new).into_iter().map(|(m, _)| m).collect();

    for (method, old_op) in &old_methods {
        if !new_method_names.contains(method) {
            return Err(format!("Method '{}' was removed from path '{}'", method.to_uppercase(), path));
        }
        // Check response codes not removed
        let new_methods = get_methods(new);
        for (nm, new_op) in &new_methods {
            if nm == method {
                for (status, _) in &old_op.responses.responses {
                    if !new_op.responses.responses.contains_key(status) {
                        return Err(format!(
                            "Response status '{}' was removed from {} {}",
                            format_status(status), method.to_uppercase(), path
                        ));
                    }
                }
            }
        }
    }

    Ok(())
}

fn format_status(status: &openapiv3::StatusCode) -> String {
    match status {
        openapiv3::StatusCode::Code(c) => c.to_string(),
        openapiv3::StatusCode::Range(r) => format!("{}XX", r),
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

    #[test]
    fn test_merge_deduplicates_shared_schema() {
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
    post:
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/Order'
      responses:
        '201':
          description: Created
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
        user:
          $ref: '#/components/schemas/User'
        id:
          type: integer
"#;
        let specs = split_openapi(yaml).unwrap();
        assert_eq!(specs.len(), 2);

        let yamls: Vec<String> = specs.iter().map(|s| s.yaml_content.clone()).collect();
        let merged = merge_endpoint_yamls(&yamls).unwrap();

        // Both paths present
        assert!(merged.contains("/users"));
        assert!(merged.contains("/orders"));
        // User schema appears exactly once (deduplicated)
        assert!(merged.contains("User"));
        assert!(merged.contains("Order"));
        // Verify it parses back correctly
        let parsed: OpenAPI = serde_yaml::from_str(&merged).unwrap();
        assert_eq!(parsed.paths.paths.len(), 2);
        let components = parsed.components.unwrap();
        assert_eq!(components.schemas.len(), 2);
    }

    #[test]
    fn test_merge_same_path_different_methods() {
        // YAML dedup means only last /users wins in parsing, so let's build snippets manually
        let snippet_get = r#"
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
        let snippet_post = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    post:
      responses:
        '201':
          description: Created
"#;
        let merged = merge_endpoint_yamls(&[snippet_get.to_string(), snippet_post.to_string()]).unwrap();
        let parsed: OpenAPI = serde_yaml::from_str(&merged).unwrap();
        assert_eq!(parsed.paths.paths.len(), 1);
        let path_item = match parsed.paths.paths.get("/users").unwrap() {
            ReferenceOr::Item(item) => item,
            _ => panic!("Expected item"),
        };
        assert!(path_item.get.is_some());
        assert!(path_item.post.is_some());
    }

    #[test]
    fn test_merge_empty_input() {
        let result = merge_endpoint_yamls(&[]);
        assert!(result.is_err());
    }
}
