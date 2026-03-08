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
