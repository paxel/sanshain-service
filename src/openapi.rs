use crate::domain::models::Impact;
use openapiv3::{Components, OpenAPI, PathItem, ReferenceOr, SchemaKind, Type as OaType};
use serde::Serialize;
use serde_json;
use serde_yaml_ng;
use similar::TextDiff;
use std::collections::{BTreeSet, HashMap, HashSet};

pub struct EndpointSpec {
    pub path: String,
    pub normalized_path: String,
    pub method: String,
    pub yaml_content: String,
    pub deprecated: bool,
}

pub fn normalize_path(path: &str) -> String {
    let path = path.trim();

    // Collapse multiple slashes
    let mut path = collapse_slashes(path);

    // Replace variable placeholders with {}
    // Placeholders are usually {name} or {name:pattern}
    path = blank_path_variables(&path);

    // Trim trailing slash if it's not the only character
    if path.len() > 1 && path.ends_with('/') {
        path.pop();
    }

    path
}

fn collapse_slashes(path: &str) -> String {
    let mut collapsed = String::with_capacity(path.len());
    let mut previous_was_slash = false;
    for ch in path.chars() {
        if ch == '/' {
            if !previous_was_slash {
                collapsed.push('/');
            }
            previous_was_slash = true;
        } else {
            collapsed.push(ch);
            previous_was_slash = false;
        }
    }
    collapsed
}

fn blank_path_variables(path: &str) -> String {
    let mut blanked = String::with_capacity(path.len());
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        blanked.push_str(&rest[..open]);
        let after_open = &rest[open + 1..];
        match after_open.find('}') {
            // A placeholder needs at least one character between the braces;
            // bare `{}` and unclosed `{` are kept verbatim.
            Some(close) if close > 0 => {
                blanked.push_str("{}");
                rest = &after_open[close + 1..];
            }
            _ => {
                blanked.push('{');
                rest = after_open;
            }
        }
    }
    blanked.push_str(rest);
    blanked
}

pub fn split_openapi(yaml_str: &str) -> Result<Vec<EndpointSpec>, String> {
    tracing::debug!("Splitting OpenAPI specification ({} bytes)", yaml_str.len());
    let openapi: OpenAPI = serde_yaml_ng::from_str(yaml_str)
        .map_err(|e| format!("Failed to parse OpenAPI YAML: {}", e))?;

    let component_graph = build_component_graph(&openapi.components);
    let mut endpoints = Vec::new();

    for (path, path_item_ref) in &openapi.paths.paths {
        let path_item = match path_item_ref {
            ReferenceOr::Item(item) => item,
            ReferenceOr::Reference { reference: _ } => continue,
        };

        let methods = get_methods(path_item);
        for (method, operation) in methods {
            let mut single_endpoint_openapi = OpenAPI {
                openapi: openapi.openapi.clone(),
                info: openapi.info.clone(),
                servers: openapi.servers.clone(),
                ..Default::default()
            };

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

            single_endpoint_openapi
                .paths
                .paths
                .insert(path.clone(), ReferenceOr::Item(new_path_item));

            single_endpoint_openapi.components = extract_used_components_optimized(
                &single_endpoint_openapi,
                &openapi.components,
                &component_graph,
            );

            if let Some(ref mut components) = single_endpoint_openapi.components {
                components.schemas.sort_keys();
                components.responses.sort_keys();
                components.parameters.sort_keys();
                components.examples.sort_keys();
                components.request_bodies.sort_keys();
                components.headers.sort_keys();
                components.security_schemes.sort_keys();
                components.links.sort_keys();
                components.callbacks.sort_keys();
            }

            let endpoint_yaml = serde_yaml_ng::to_string(&single_endpoint_openapi)
                .map_err(|e| format!("Failed to serialize endpoint YAML: {}", e))?;

            endpoints.push(EndpointSpec {
                path: path.clone(),
                normalized_path: normalize_path(path),
                method: method.to_uppercase(),
                yaml_content: endpoint_yaml,
                deprecated: operation.deprecated,
            });
        }
    }

    Ok(endpoints)
}

/// Build a dependency graph of components.
fn build_component_graph(components: &Option<Components>) -> HashMap<String, BTreeSet<String>> {
    let mut graph = HashMap::new();
    let c = match components {
        Some(c) => c,
        None => return graph,
    };

    for (name, item) in &c.schemas {
        graph.insert(format!("schemas/{}", name), collect_refs(item));
    }
    for (name, item) in &c.responses {
        graph.insert(format!("responses/{}", name), collect_refs(item));
    }
    for (name, item) in &c.parameters {
        graph.insert(format!("parameters/{}", name), collect_refs(item));
    }
    for (name, item) in &c.examples {
        graph.insert(format!("examples/{}", name), collect_refs(item));
    }
    for (name, item) in &c.request_bodies {
        graph.insert(format!("requestBodies/{}", name), collect_refs(item));
    }
    for (name, item) in &c.headers {
        graph.insert(format!("headers/{}", name), collect_refs(item));
    }
    for (name, item) in &c.security_schemes {
        graph.insert(format!("securitySchemes/{}", name), collect_refs(item));
    }
    for (name, item) in &c.links {
        graph.insert(format!("links/{}", name), collect_refs(item));
    }
    for (name, item) in &c.callbacks {
        graph.insert(format!("callbacks/{}", name), collect_refs(item));
    }

    graph
}

fn extract_used_components_optimized(
    partial_spec: &OpenAPI,
    all_components: &Option<Components>,
    graph: &HashMap<String, BTreeSet<String>>,
) -> Option<Components> {
    let all_components = match all_components {
        Some(c) => c,
        None => return None,
    };

    let mut to_visit: Vec<String> = collect_refs(&partial_spec.paths).into_iter().collect();
    let mut visited: BTreeSet<String> = BTreeSet::new();

    while let Some(ref_key) = to_visit.pop() {
        if !visited.insert(ref_key.clone()) {
            continue;
        }
        if let Some(neighbors) = graph.get(&ref_key) {
            for neighbor in neighbors {
                if !visited.contains(neighbor) {
                    to_visit.push(neighbor.clone());
                }
            }
        }
    }

    let mut filtered = Components::default();
    for key in &visited {
        let parts: Vec<&str> = key.splitn(2, '/').collect();
        if parts.len() != 2 {
            continue;
        }
        let category = parts[0];
        let name = parts[1];

        match category {
            "schemas" => {
                if let Some(s) = all_components.schemas.get(name) {
                    filtered.schemas.insert(name.to_string(), s.clone());
                }
            }
            "responses" => {
                if let Some(r) = all_components.responses.get(name) {
                    filtered.responses.insert(name.to_string(), r.clone());
                }
            }
            "parameters" => {
                if let Some(p) = all_components.parameters.get(name) {
                    filtered.parameters.insert(name.to_string(), p.clone());
                }
            }
            "examples" => {
                if let Some(e) = all_components.examples.get(name) {
                    filtered.examples.insert(name.to_string(), e.clone());
                }
            }
            "requestBodies" => {
                if let Some(rb) = all_components.request_bodies.get(name) {
                    filtered.request_bodies.insert(name.to_string(), rb.clone());
                }
            }
            "headers" => {
                if let Some(h) = all_components.headers.get(name) {
                    filtered.headers.insert(name.to_string(), h.clone());
                }
            }
            "securitySchemes" => {
                if let Some(ss) = all_components.security_schemes.get(name) {
                    filtered
                        .security_schemes
                        .insert(name.to_string(), ss.clone());
                }
            }
            "links" => {
                if let Some(l) = all_components.links.get(name) {
                    filtered.links.insert(name.to_string(), l.clone());
                }
            }
            "callbacks" => {
                if let Some(c) = all_components.callbacks.get(name) {
                    filtered.callbacks.insert(name.to_string(), c.clone());
                }
            }
            _ => {}
        }
    }

    if filtered.schemas.is_empty()
        && filtered.responses.is_empty()
        && filtered.parameters.is_empty()
        && filtered.examples.is_empty()
        && filtered.request_bodies.is_empty()
        && filtered.headers.is_empty()
        && filtered.security_schemes.is_empty()
        && filtered.links.is_empty()
        && filtered.callbacks.is_empty()
    {
        None
    } else {
        Some(filtered)
    }
}

/// Extract all `$ref` strings from a serializable object without YAML round-tripping.
fn collect_refs(value: &impl Serialize) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    if let Ok(json_value) = serde_json::to_value(value) {
        collect_refs_recursive(&json_value, &mut refs);
    }
    refs
}

fn collect_refs_recursive(value: &serde_json::Value, refs: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(stripped) = map
                .get("$ref")
                .and_then(|v| v.as_str())
                .and_then(|r| r.strip_prefix("#/components/"))
            {
                refs.insert(stripped.to_string());
            }
            for v in map.values() {
                collect_refs_recursive(v, refs);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                collect_refs_recursive(v, refs);
            }
        }
        _ => {}
    }
}

/// Merge multiple per-endpoint YAML snippets (from the same service) into a single OpenAPI spec
/// with deduplicated schemas/components.
pub fn merge_endpoint_yamls(yamls: &[String]) -> Result<String, String> {
    if yamls.is_empty() {
        return Err("No endpoint YAMLs to merge".to_string());
    }

    // Use the first snippet as the base (info, servers, openapi version)
    let mut merged: OpenAPI = serde_yaml_ng::from_str(&yamls[0])
        .map_err(|e| format!("Failed to parse first endpoint YAML: {}", e))?;

    // Merge paths and components from all subsequent snippets
    for yaml in &yamls[1..] {
        let spec: OpenAPI = serde_yaml_ng::from_str(yaml)
            .map_err(|e| format!("Failed to parse endpoint YAML: {}", e))?;

        // Merge paths
        for (path, path_item_ref) in spec.paths.paths {
            if let Some(existing_ref) = merged.paths.paths.get_mut(&path) {
                // Merge operations into existing path item
                if let (ReferenceOr::Item(existing_item), ReferenceOr::Item(new_item)) =
                    (existing_ref, path_item_ref)
                {
                    if new_item.get.is_some() {
                        existing_item.get = new_item.get;
                    }
                    if new_item.post.is_some() {
                        existing_item.post = new_item.post;
                    }
                    if new_item.put.is_some() {
                        existing_item.put = new_item.put;
                    }
                    if new_item.delete.is_some() {
                        existing_item.delete = new_item.delete;
                    }
                    if new_item.options.is_some() {
                        existing_item.options = new_item.options;
                    }
                    if new_item.head.is_some() {
                        existing_item.head = new_item.head;
                    }
                    if new_item.patch.is_some() {
                        existing_item.patch = new_item.patch;
                    }
                    if new_item.trace.is_some() {
                        existing_item.trace = new_item.trace;
                    }
                }
            } else {
                merged.paths.paths.insert(path, path_item_ref);
            }
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

    // Sort paths and component maps for deterministic output regardless of input order.
    merged.paths.paths.sort_keys();
    if let Some(ref mut components) = merged.components {
        components.schemas.sort_keys();
        components.responses.sort_keys();
        components.parameters.sort_keys();
        components.request_bodies.sort_keys();
        components.headers.sort_keys();
        components.security_schemes.sort_keys();
    }

    serde_yaml_ng::to_string(&merged).map_err(|e| format!("Failed to serialize merged YAML: {}", e))
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
///
/// Adding new optional/required fields, new response codes, or new schemas is OK.
///
/// Returns Ok(()) if compatible, Err(description) if breaking.
pub fn check_backward_compatibility(old_yaml: &str, new_yaml: &str) -> Result<(), String> {
    tracing::debug!("Checking backward compatibility...");
    let old: OpenAPI = serde_yaml_ng::from_str(old_yaml)
        .map_err(|e| format!("Failed to parse old YAML: {}", e))?;
    let new: OpenAPI = serde_yaml_ng::from_str(new_yaml)
        .map_err(|e| format!("Failed to parse new YAML: {}", e))?;

    check_openapi_compatible(&old, &new)
}

/// Analyze the impact of a specification change to determine the SemVer increment.
pub fn analyze_impact(old_yaml: &str, new_yaml: &str) -> Impact {
    let old: OpenAPI = match serde_yaml_ng::from_str(old_yaml) {
        Ok(o) => o,
        Err(_) => return Impact::Major,
    };
    let new: OpenAPI = match serde_yaml_ng::from_str(new_yaml) {
        Ok(n) => n,
        Err(_) => return Impact::Major,
    };

    if check_openapi_compatible(&old, &new).is_err() {
        return Impact::Major;
    }

    if has_additions(&old, &new) {
        return Impact::Minor;
    }

    if old_yaml != new_yaml {
        return Impact::Patch;
    }

    Impact::None
}

fn has_additions(old: &OpenAPI, new: &OpenAPI) -> bool {
    // New paths
    for path in new.paths.paths.keys() {
        if !old.paths.paths.contains_key(path) {
            return true;
        }
    }

    // New methods or responses
    for (path, new_item_ref) in &new.paths.paths {
        if let (Some(ReferenceOr::Item(old_item)), ReferenceOr::Item(new_item)) =
            (old.paths.paths.get(path), new_item_ref)
        {
            let old_methods = get_methods(old_item);
            let new_methods = get_methods(new_item);

            let old_method_names: HashSet<_> = old_methods.iter().map(|(m, _)| m).collect();
            for (method, new_op) in &new_methods {
                if !old_method_names.contains(method) {
                    return true;
                }

                // New response in existing method
                let Some(old_entry) = old_methods.iter().find(|(m, _)| m == method) else {
                    continue;
                };
                let old_op = old_entry.1;
                for status in new_op.responses.responses.keys() {
                    if !old_op.responses.responses.contains_key(status) {
                        return true;
                    }
                }
            }
        }
    }

    // New schemas
    let old_schemas = collect_schema_map(old);
    let new_schemas = collect_schema_map(new);
    for name in new_schemas.keys() {
        if !old_schemas.contains_key(name) {
            return true;
        }
    }

    // New fields in existing schemas
    for (name, old_schema) in &old_schemas {
        if let Some(new_schema) = new_schemas.get(name) {
            let old_props = extract_object_properties(old_schema);
            let new_props = extract_object_properties(new_schema);
            if let (Some(old_props), Some(new_props)) = (old_props, new_props) {
                for prop_name in new_props.keys() {
                    if !old_props.contains_key(prop_name) {
                        return true;
                    }
                }
            }
        }
    }

    false
}

/// Check if a new OpenAPI spec is backward-compatible with an old one.
pub fn check_openapi_compatible(old: &OpenAPI, new: &OpenAPI) -> Result<(), String> {
    let old_schemas = collect_schema_map(old);
    let new_schemas = collect_schema_map(new);
    let new_request_schemas = collect_request_schemas(new);

    // Check each existing schema for breaking changes
    for (name, old_schema) in &old_schemas {
        if let Some(new_schema) = new_schemas.get(name) {
            let is_request = new_request_schemas.contains(name);
            check_schema_compatible(name, old_schema, new_schema, is_request)?;
        }
    }

    // Check that no existing paths/methods or status codes were removed.
    // Removing a path is allowed when all of its operations were deprecated.
    for (path, old_item) in &old.paths.paths {
        if let ReferenceOr::Item(old_pi) = old_item {
            match new.paths.paths.get(path) {
                Some(ReferenceOr::Item(new_pi)) => {
                    check_operations_compatible(path, old_pi, new_pi)?;
                }
                _ => {
                    if !get_methods(old_pi).iter().all(|(_, op)| op.deprecated) {
                        return Err(format!("Path '{}' was removed", path));
                    }
                }
            }
        }
    }

    Ok(())
}

fn collect_request_schemas(spec: &OpenAPI) -> HashSet<String> {
    let mut request_schemas = HashSet::new();

    // Find all schemas used in request bodies
    for path_item_ref in spec.paths.paths.values() {
        if let ReferenceOr::Item(pi) = path_item_ref {
            for (_, op) in get_methods(pi) {
                if let Some(rb_ref) = &op.request_body {
                    let refs = collect_refs(rb_ref);
                    for r in refs {
                        if let Some(name) = r.strip_prefix("schemas/") {
                            request_schemas.insert(name.to_string());
                        }
                    }
                }
            }
        }
    }

    // Transitive closure to find all schemas reachable from request bodies
    let mut changed = true;
    while changed {
        changed = false;
        let mut new_schemas = Vec::new();
        for name in &request_schemas {
            if let Some(components) = &spec.components
                && let Some(ReferenceOr::Item(s)) = components.schemas.get(name)
            {
                let refs = collect_refs(s);
                for r in refs {
                    if let Some(sub_name) = r.strip_prefix("schemas/")
                        && !request_schemas.contains(sub_name)
                    {
                        new_schemas.push(sub_name.to_string());
                    }
                }
            }
        }
        if !new_schemas.is_empty() {
            for s in new_schemas {
                request_schemas.insert(s);
            }
            changed = true;
        }
    }

    request_schemas
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
fn check_schema_compatible(
    name: &str,
    old: &openapiv3::Schema,
    new: &openapiv3::Schema,
    is_request: bool,
) -> Result<(), String> {
    let old_props = extract_object_properties(old);
    let new_props = extract_object_properties(new);

    if let (Some(old_props), Some(new_props)) = (old_props, new_props) {
        let old_required = extract_required_fields(old);
        let new_required = extract_required_fields(new);

        // Check no existing properties were removed (deprecated ones may go)
        for (prop_name, old_prop) in &old_props {
            if !new_props.contains_key(prop_name) && !old_prop.deprecated {
                return Err(format!(
                    "Property '{}' was removed from schema '{}'",
                    prop_name, name
                ));
            }
        }

        // Check for new required fields in request schemas
        for req in &new_required {
            if is_request && !old_required.contains(req) {
                return Err(format!(
                    "New required field '{}' was added to request schema '{}'",
                    req, name
                ));
            }
        }

        // Check no property types changed
        for (prop_name, old_prop) in &old_props {
            if let Some(new_prop) = new_props.get(prop_name)
                && old_prop.type_str != new_prop.type_str
            {
                return Err(format!(
                    "Property '{}' in schema '{}' changed type from '{}' to '{}'",
                    prop_name, name, old_prop.type_str, new_prop.type_str
                ));
            }
        }
    }

    Ok(())
}

struct PropertyInfo {
    type_str: String,
    deprecated: bool,
}

/// Extract property names, type strings, and deprecation flags from an object schema.
fn extract_object_properties(schema: &openapiv3::Schema) -> Option<HashMap<&str, PropertyInfo>> {
    match &schema.schema_kind {
        SchemaKind::Type(OaType::Object(obj)) => {
            let mut props = HashMap::new();
            for (name, prop_ref) in &obj.properties {
                let info = match prop_ref {
                    ReferenceOr::Item(box_schema) => PropertyInfo {
                        type_str: describe_schema_type(&box_schema.schema_kind),
                        deprecated: box_schema.schema_data.deprecated,
                    },
                    ReferenceOr::Reference { reference } => PropertyInfo {
                        type_str: reference.clone(),
                        deprecated: false,
                    },
                };
                props.insert(name.as_str(), info);
            }
            Some(props)
        }
        _ => None,
    }
}

/// Extract required field names from a schema.
fn extract_required_fields(schema: &openapiv3::Schema) -> HashSet<String> {
    match &schema.schema_kind {
        SchemaKind::Type(OaType::Object(obj)) => obj.required.iter().cloned().collect(),
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
            if old_op.deprecated {
                continue;
            }
            return Err(format!(
                "Method '{}' was removed from path '{}'",
                method.to_uppercase(),
                path
            ));
        }
        // Check response codes not removed
        let new_methods = get_methods(new);
        for (nm, new_op) in &new_methods {
            if nm == method {
                for (status, _) in &old_op.responses.responses {
                    if !new_op.responses.responses.contains_key(status) {
                        return Err(format!(
                            "Response status '{}' was removed from {} {}",
                            format_status(status),
                            method.to_uppercase(),
                            path
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
    if let Some(op) = &path_item.get {
        methods.push(("get".to_string(), op));
    }
    if let Some(op) = &path_item.post {
        methods.push(("post".to_string(), op));
    }
    if let Some(op) = &path_item.put {
        methods.push(("put".to_string(), op));
    }
    if let Some(op) = &path_item.delete {
        methods.push(("delete".to_string(), op));
    }
    if let Some(op) = &path_item.options {
        methods.push(("options".to_string(), op));
    }
    if let Some(op) = &path_item.head {
        methods.push(("head".to_string(), op));
    }
    if let Some(op) = &path_item.patch {
        methods.push(("patch".to_string(), op));
    }
    if let Some(op) = &path_item.trace {
        methods.push(("trace".to_string(), op));
    }
    methods
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path() {
        assert_eq!(normalize_path("/api/users"), "/api/users");
        assert_eq!(normalize_path("/api/users/"), "/api/users");
        assert_eq!(normalize_path(" /api/users "), "/api/users");
        assert_eq!(normalize_path("/api//users"), "/api/users");
        assert_eq!(normalize_path("///api///users///"), "/api/users");
        assert_eq!(normalize_path("/"), "/");
        assert_eq!(normalize_path("//"), "/");
        assert_eq!(normalize_path("/api/{id}"), "/api/{}");
        assert_eq!(normalize_path("/api/{userId}"), "/api/{}");
        assert_eq!(normalize_path("/api/{id}/details"), "/api/{}/details");
        assert_eq!(normalize_path("/api/{id}/{action}"), "/api/{}/{}");
        assert_eq!(normalize_path("/api/{id:pattern}"), "/api/{}");
        assert_eq!(normalize_path("/api/{uId}/"), "/api/{}");
        assert_eq!(normalize_path("/api/{}"), "/api/{}");
        assert_eq!(normalize_path("/api/{unclosed"), "/api/{unclosed");
        assert_eq!(normalize_path("/api/{a{b}/x"), "/api/{}/x");
        assert_eq!(normalize_path("/api/{ü}"), "/api/{}");
    }

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
        let methods: Vec<(&str, &str)> = result
            .iter()
            .map(|e| (e.path.as_str(), e.method.as_str()))
            .collect();
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
    fn test_split_includes_transitive_refs_from_headers() {
        let yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /test:
    get:
      responses:
        '200':
          description: OK
          headers:
            X-Test:
              $ref: '#/components/headers/TestHeader'
components:
  headers:
    TestHeader:
      schema:
        $ref: '#/components/schemas/TestSchema'
  schemas:
    TestSchema:
      type: string
"#;
        let result = split_openapi(yaml).unwrap();
        // println!("YAML content: {}", result[0].yaml_content);
        assert_eq!(result.len(), 1);
        assert!(result[0].yaml_content.contains("TestHeader"));
        assert!(result[0].yaml_content.contains("TestSchema"));
        // Verify it's actually in the schemas section
        assert!(result[0].yaml_content.contains("schemas:"));
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
        let parsed: OpenAPI = serde_yaml_ng::from_str(&merged).unwrap();
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
        let merged =
            merge_endpoint_yamls(&[snippet_get.to_string(), snippet_post.to_string()]).unwrap();
        let parsed: OpenAPI = serde_yaml_ng::from_str(&merged).unwrap();
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

    #[test]
    fn test_merge_endpoint_yamls_order_independent() {
        let snippet_users = r#"
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
        let snippet_orders = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
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
    Order:
      type: object
      properties:
        id:
          type: integer
"#;
        let order_a =
            merge_endpoint_yamls(&[snippet_users.to_string(), snippet_orders.to_string()]).unwrap();
        let order_b =
            merge_endpoint_yamls(&[snippet_orders.to_string(), snippet_users.to_string()]).unwrap();
        assert_eq!(
            order_a, order_b,
            "Merge output must be identical regardless of input order"
        );
    }

    #[test]
    fn test_split_openapi_determinism() {
        let yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /test:
    get:
      responses:
        '200':
          description: OK
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/C'
components:
  schemas:
    A:
      type: string
    B:
      type: string
    C:
      allOf:
        - $ref: '#/components/schemas/A'
        - $ref: '#/components/schemas/B'
"#;
        let result1 = split_openapi(yaml).unwrap();

        for _ in 0..10 {
            let result2 = split_openapi(yaml).unwrap();
            assert_eq!(
                result1[0].yaml_content, result2[0].yaml_content,
                "Split output must be bit-for-bit identical across runs"
            );
        }
    }

    #[test]
    fn test_split_openapi_sorts_components() {
        // Input has Z, then A. Output should have A, then Z.
        let yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /test:
    get:
      responses:
        '200':
          description: OK
          content:
            application/json:
              schema:
                type: object
                properties:
                  z:
                    $ref: '#/components/schemas/Z'
                  a:
                    $ref: '#/components/schemas/A'
components:
  schemas:
    Z:
      type: string
    A:
      type: string
"#;
        let result = split_openapi(yaml).unwrap();
        let content = result[0].yaml_content.clone();

        // Find positions of "A:" and "Z:" in the output YAML.
        let pos_a = content.find("A:").expect("A not found");
        let pos_z = content.find("Z:").expect("Z not found");

        assert!(
            pos_a < pos_z,
            "A should come before Z in the output YAML:\n{}",
            content
        );
    }

    #[test]
    fn test_breaking_change_removed_path() {
        let old_yaml = r#"
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
        let new_yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths: {}
"#;
        let result = check_backward_compatibility(old_yaml, new_yaml);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Path '/users' was removed"));
    }

    #[test]
    fn test_removing_deprecated_path_and_method_is_not_breaking() {
        let old_yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /legacy:
    get:
      deprecated: true
      responses:
        '200':
          description: OK
  /users:
    get:
      responses:
        '200':
          description: OK
    post:
      deprecated: true
      responses:
        '201':
          description: Created
"#;
        let new_yaml = r#"
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
        assert_eq!(check_backward_compatibility(old_yaml, new_yaml), Ok(()));
    }

    #[test]
    fn test_removing_non_deprecated_method_is_still_breaking() {
        let old_yaml = r#"
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
"#;
        let new_yaml = r#"
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
        let err = check_backward_compatibility(old_yaml, new_yaml).unwrap_err();
        assert!(
            err.contains("Method 'POST' was removed from path '/users'"),
            "{err}"
        );
    }

    #[test]
    fn test_removing_deprecated_property_is_not_breaking() {
        let old_yaml = r#"
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
components:
  schemas:
    User:
      type: object
      properties:
        id:
          type: string
        legacy_name:
          type: string
          deprecated: true
"#;
        let new_yaml = r#"
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
components:
  schemas:
    User:
      type: object
      properties:
        id:
          type: string
"#;
        assert_eq!(check_backward_compatibility(old_yaml, new_yaml), Ok(()));

        // Same removal without the deprecation marker stays breaking.
        let old_without_marker = old_yaml.replace("\n          deprecated: true", "");
        let err = check_backward_compatibility(&old_without_marker, new_yaml).unwrap_err();
        assert!(
            err.contains("Property 'legacy_name' was removed from schema 'User'"),
            "{err}"
        );
    }

    #[test]
    fn test_breaking_change_new_required_request_field() {
        let old_yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    post:
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/User'
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
"#;
        let new_yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /users:
    post:
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/User'
      responses:
        '201':
          description: Created
components:
  schemas:
    User:
      type: object
      required:
        - name
      properties:
        name:
          type: string
"#;
        let result = check_backward_compatibility(old_yaml, new_yaml);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("New required field 'name' was added to request schema 'User'")
        );
    }

    #[test]
    fn test_non_breaking_change_new_required_response_field() {
        let old_yaml = r#"
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
        let new_yaml = r#"
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
      required:
        - name
      properties:
        name:
          type: string
"#;
        let result = check_backward_compatibility(old_yaml, new_yaml);
        assert!(
            result.is_ok(),
            "Adding required field to response schema should NOT be breaking, but got: {:?}",
            result
        );
    }
}
