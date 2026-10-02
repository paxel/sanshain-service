use crate::domain::models::{ApiType, Impact};
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

/// Read the Producer-declared version out of a spec document's `info.version`
/// (OpenAPI and AsyncAPI share the same `info` shape). The field is
/// load-bearing in 2.0: absent or non-semver values reject the Provide.
pub fn extract_info_version(yaml_str: &str) -> Result<crate::domain::models::SemVer, String> {
    #[derive(serde::Deserialize)]
    struct InfoOnly {
        info: Option<InfoVersion>,
    }
    #[derive(serde::Deserialize)]
    struct InfoVersion {
        version: Option<String>,
    }

    let parsed: InfoOnly = serde_yaml_ng::from_str(yaml_str)
        .map_err(|e| format!("Failed to parse spec YAML: {}", e))?;
    let raw = parsed
        .info
        .and_then(|i| i.version)
        .ok_or_else(|| "spec has no info.version — the version is required and lives in the spec file (MAJOR.MINOR.PATCH)".to_string())?;
    crate::domain::models::SemVer::parse_spec_version(&raw)
        .map_err(|e| format!("info.version: {}", e))
}

/// The key an endpoint lookup must match against the stored
/// `normalized_path`. Only OpenAPI paths are normalized at provide time;
/// AsyncAPI channels and Proto service names are stored verbatim, so
/// normalizing them here would blank their `{param}` segments and miss every
/// parameterized channel.
pub fn lookup_path(api_type: ApiType, path: &str) -> String {
    match api_type {
        ApiType::OpenApi => normalize_path(path),
        ApiType::AsyncApi | ApiType::Proto => path.to_string(),
    }
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
/// - No existing path, method, response status code, parameter or property
///   removed, unless it was deprecated
/// - No property, parameter or schema type changed — at any nesting depth, in
///   components as well as in inline request and response bodies
/// - No new required field in a request schema, no new required parameter, and
///   no optional parameter made required
/// - No enum value removed from a request schema or parameter, and no enum
///   restriction added to one that had none
///
/// Adding optional request fields or parameters, response fields, response
/// codes, response enum values, or new schemas is OK.
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
///
/// Deliberately says nothing about the documents as text. `old_yaml` is not the
/// document previously provided — it is a reconstruction merged back together
/// from the stored per-endpoint fragments, taking the document header from
/// whichever fragment sorts first and re-serialising it. Comparing that against
/// a producer's original is comparing two different artefacts, and it never
/// matches, so a textual fallback here reported a change on every provide.
///
/// The caller already computes a per-endpoint diff, fragment against fragment,
/// which is the comparison that actually holds. Patch-level impact is derived
/// from that; anything this function cannot see in the parsed API surface is
/// not its business.
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
            check_schema_compatible((old, new), name, old_schema, new_schema, is_request)?;
        }
    }

    // Check that no existing paths/methods or status codes were removed.
    // Removing a path is allowed when all of its operations were deprecated.
    for (path, old_item) in &old.paths.paths {
        if let ReferenceOr::Item(old_pi) = old_item {
            match new.paths.paths.get(path) {
                Some(ReferenceOr::Item(new_pi)) => {
                    check_operations_compatible(path, old, old_pi, new, new_pi)?;
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
///
/// Recurses into property and array-item schemas, which inherit `is_request`
/// from their position. Two `$ref`s are compared by name only: every named
/// component is checked on its own, which also keeps a self-referencing schema
/// from recursing forever. An inline schema swapped for a `$ref` (or back) is
/// compared by shape, so extracting a component is not a change.
fn check_schema_compatible(
    specs: Specs<'_>,
    name: &str,
    old: &openapiv3::Schema,
    new: &openapiv3::Schema,
    is_request: bool,
) -> Result<(), String> {
    // Compositions (allOf/oneOf/anyOf) and untyped schemas describe as
    // "unknown" and are not compared.
    let old_type = describe_schema_type(&old.schema_kind);
    let new_type = describe_schema_type(&new.schema_kind);
    if old_type != new_type && old_type != "unknown" && new_type != "unknown" {
        return Err(format!(
            "Schema '{}' changed type from '{}' to '{}'",
            name, old_type, new_type
        ));
    }

    if is_request {
        check_enum_not_narrowed(name, old, new)?;
    }

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

        // Check no property types changed, then each kept property one level down
        for (prop_name, old_prop) in &old_props {
            if let Some(new_prop) = new_props.get(prop_name) {
                check_subschema_compatible(
                    specs,
                    &format!("{}.{}", name, prop_name),
                    (old_prop.schema, new_prop.schema),
                    is_request,
                    |old_type, new_type| {
                        format!(
                            "Property '{}' in schema '{}' changed type from '{}' to '{}'",
                            prop_name, name, old_type, new_type
                        )
                    },
                )?;
            }
        }
    }

    if let (SchemaKind::Type(OaType::Array(old_arr)), SchemaKind::Type(OaType::Array(new_arr))) =
        (&old.schema_kind, &new.schema_kind)
        && let (Some(old_items), Some(new_items)) = (&old_arr.items, &new_arr.items)
    {
        let items_name = format!("{}[]", name);
        check_subschema_compatible(
            specs,
            &items_name,
            (old_items, new_items),
            is_request,
            |old_type, new_type| {
                format!(
                    "Schema '{}' changed type from '{}' to '{}'",
                    items_name, old_type, new_type
                )
            },
        )?;
    }

    Ok(())
}

/// The old and the new document, for resolving `$ref`s on either side.
type Specs<'a> = (&'a OpenAPI, &'a OpenAPI);

/// Compare a property or array-item schema; `type_changed` words the error
/// when its type differs. Two inline schemas recurse; two `$ref`s must name the
/// same component; a mix resolves the `$ref` and recurses, which ends because
/// the inline side gets smaller with every step.
fn check_subschema_compatible(
    specs: Specs<'_>,
    name: &str,
    (old, new): (
        &ReferenceOr<Box<openapiv3::Schema>>,
        &ReferenceOr<Box<openapiv3::Schema>>,
    ),
    is_request: bool,
    type_changed: impl Fn(&str, &str) -> String,
) -> Result<(), String> {
    let (old_type, new_type) = (describe_boxed_ref(old), describe_boxed_ref(new));
    match (old, new) {
        (ReferenceOr::Item(old_schema), ReferenceOr::Item(new_schema)) => {
            // "unknown" is not compared, as at the top of `check_schema_compatible`
            if old_type != new_type && old_type != "unknown" && new_type != "unknown" {
                return Err(type_changed(&old_type, &new_type));
            }
            check_schema_compatible(specs, name, old_schema, new_schema, is_request)
        }
        (ReferenceOr::Reference { .. }, ReferenceOr::Reference { .. }) => {
            if old_type == new_type {
                Ok(())
            } else {
                Err(type_changed(&old_type, &new_type))
            }
        }
        _ => match (resolve_boxed(specs.0, old), resolve_boxed(specs.1, new)) {
            (Some(old_schema), Some(new_schema)) => {
                check_schema_compatible(specs, name, old_schema, new_schema, is_request)
            }
            _ => Err(type_changed(&old_type, &new_type)),
        },
    }
}

fn resolve_boxed<'a>(
    spec: &'a OpenAPI,
    schema: &'a ReferenceOr<Box<openapiv3::Schema>>,
) -> Option<&'a openapiv3::Schema> {
    match schema {
        ReferenceOr::Item(s) => Some(s),
        ReferenceOr::Reference { reference } => component_schema(spec, reference),
    }
}

/// A request value the old contract accepted must still be accepted: no enum
/// value may disappear, and an unrestricted value may not become an enum.
/// Widening is fine. Only request schemas are checked — a new response enum
/// value is deliberately not breaking.
fn check_enum_not_narrowed(
    name: &str,
    old: &openapiv3::Schema,
    new: &openapiv3::Schema,
) -> Result<(), String> {
    match (enum_values(old), enum_values(new)) {
        (None, Some(_)) => Err(format!(
            "Request schema '{}' was restricted to an enum",
            name
        )),
        (Some(old_values), Some(new_values)) => match old_values.difference(&new_values).next() {
            Some(removed) => Err(format!(
                "Enum value {} was removed from request schema '{}'",
                removed, name
            )),
            None => Ok(()),
        },
        _ => Ok(()),
    }
}

/// The enum values of a scalar schema as JSON literals, or `None` when the
/// schema has no enum restriction.
fn enum_values(schema: &openapiv3::Schema) -> Option<BTreeSet<String>> {
    let values = match &schema.schema_kind {
        SchemaKind::Type(OaType::String(t)) => json_literals(&t.enumeration),
        SchemaKind::Type(OaType::Number(t)) => json_literals(&t.enumeration),
        SchemaKind::Type(OaType::Integer(t)) => json_literals(&t.enumeration),
        SchemaKind::Type(OaType::Boolean(t)) => json_literals(&t.enumeration),
        _ => return None,
    };
    if values.is_empty() {
        None
    } else {
        Some(values)
    }
}

fn json_literals<T: Serialize>(values: &[T]) -> BTreeSet<String> {
    values
        .iter()
        .filter_map(|v| serde_json::to_string(v).ok())
        .collect()
}

struct PropertyInfo<'a> {
    deprecated: bool,
    schema: &'a ReferenceOr<Box<openapiv3::Schema>>,
}

/// Extract property names, deprecation flags and schemas from an object schema.
fn extract_object_properties(
    schema: &openapiv3::Schema,
) -> Option<HashMap<&str, PropertyInfo<'_>>> {
    match &schema.schema_kind {
        SchemaKind::Type(OaType::Object(obj)) => {
            let mut props = HashMap::new();
            for (name, prop_ref) in &obj.properties {
                let info = PropertyInfo {
                    deprecated: match prop_ref {
                        ReferenceOr::Item(box_schema) => box_schema.schema_data.deprecated,
                        ReferenceOr::Reference { .. } => false,
                    },
                    schema: prop_ref,
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

/// Describe an inline-or-`$ref` subschema: its type, or the reference itself.
fn describe_boxed_ref(schema: &ReferenceOr<Box<openapiv3::Schema>>) -> String {
    match schema {
        ReferenceOr::Item(s) => describe_schema_type(&s.schema_kind),
        ReferenceOr::Reference { reference } => reference.clone(),
    }
}

/// Check that operations on a path haven't had breaking changes.
fn check_operations_compatible(
    path: &str,
    old_spec: &OpenAPI,
    old: &PathItem,
    new_spec: &OpenAPI,
    new: &PathItem,
) -> Result<(), String> {
    let old_methods = get_methods(old);
    let new_methods = get_methods(new);

    for (method, old_op) in &old_methods {
        let Some((_, new_op)) = new_methods.iter().find(|(m, _)| m == method) else {
            if old_op.deprecated {
                continue;
            }
            return Err(format!(
                "Method '{}' was removed from path '{}'",
                method.to_uppercase(),
                path
            ));
        };
        let operation = format!("{} {}", method.to_uppercase(), path);

        // Check response codes not removed
        for (status, _) in &old_op.responses.responses {
            if !new_op.responses.responses.contains_key(status) {
                return Err(format!(
                    "Response status '{}' was removed from {}",
                    format_status(status),
                    operation
                ));
            }
        }

        check_parameters_compatible(
            &operation,
            &collect_parameters(old_spec, old, old_op),
            &collect_parameters(new_spec, new, new_op),
            old_spec,
            new_spec,
        )?;

        if let (Some(ReferenceOr::Item(old_body)), Some(ReferenceOr::Item(new_body))) =
            (&old_op.request_body, &new_op.request_body)
        {
            check_content_compatible(
                &format!("{} request body", operation),
                (old_spec, &old_body.content),
                (new_spec, &new_body.content),
                true,
            )?;
        }

        for (status, old_response) in &old_op.responses.responses {
            if let (ReferenceOr::Item(old_response), Some(ReferenceOr::Item(new_response))) =
                (old_response, new_op.responses.responses.get(status))
            {
                check_content_compatible(
                    &format!("{} response {}", operation, format_status(status)),
                    (old_spec, &old_response.content),
                    (new_spec, &new_response.content),
                    false,
                )?;
            }
        }
    }

    Ok(())
}

/// Parameters by `(location, name)`. Header names are lowercased in the key,
/// since HTTP header names are case-insensitive (RFC 7230).
type ParameterMap<'a> = HashMap<(&'static str, String), &'a openapiv3::ParameterData>;

/// The parameters an operation takes: the path item's, overridden by the
/// operation's own, with `$ref`s into `components.parameters` resolved.
fn collect_parameters<'a>(
    spec: &'a OpenAPI,
    path_item: &'a PathItem,
    op: &'a openapiv3::Operation,
) -> ParameterMap<'a> {
    let mut params = HashMap::new();
    for param_ref in path_item.parameters.iter().chain(&op.parameters) {
        if let Some(param) = resolve_parameter(spec, param_ref) {
            let data = param.parameter_data_ref();
            let location = parameter_location(param);
            let key = if location == "header" {
                data.name.to_lowercase()
            } else {
                data.name.clone()
            };
            params.insert((location, key), data);
        }
    }
    params
}

fn resolve_parameter<'a>(
    spec: &'a OpenAPI,
    param: &'a ReferenceOr<openapiv3::Parameter>,
) -> Option<&'a openapiv3::Parameter> {
    match param {
        ReferenceOr::Item(p) => Some(p),
        ReferenceOr::Reference { reference } => {
            let name = reference.strip_prefix("#/components/parameters/")?;
            match spec.components.as_ref()?.parameters.get(name)? {
                ReferenceOr::Item(p) => Some(p),
                ReferenceOr::Reference { .. } => None,
            }
        }
    }
}

fn resolve_schema<'a>(
    spec: &'a OpenAPI,
    schema: &'a ReferenceOr<openapiv3::Schema>,
) -> Option<&'a openapiv3::Schema> {
    match schema {
        ReferenceOr::Item(s) => Some(s),
        ReferenceOr::Reference { reference } => component_schema(spec, reference),
    }
}

/// The schema a `#/components/schemas/...` reference points at, if any.
fn component_schema<'a>(spec: &'a OpenAPI, reference: &str) -> Option<&'a openapiv3::Schema> {
    let name = reference.strip_prefix("#/components/schemas/")?;
    match spec.components.as_ref()?.schemas.get(name)? {
        ReferenceOr::Item(s) => Some(s),
        ReferenceOr::Reference { .. } => None,
    }
}

fn parameter_location(param: &openapiv3::Parameter) -> &'static str {
    match param {
        openapiv3::Parameter::Query { .. } => "query",
        openapiv3::Parameter::Header { .. } => "header",
        openapiv3::Parameter::Path { .. } => "path",
        openapiv3::Parameter::Cookie { .. } => "cookie",
    }
}

/// A Consumer built against the old parameters must still be served: none
/// removed (unless deprecated), none newly required, and each kept one still
/// accepting what it accepted (type, enum values, nested shape).
///
/// Path parameters are exempt from the removed/required checks: the path
/// template already fixes them, and a changed template is a removed path.
/// Declaring one that was left implicit, or adding the `required: true` the
/// specification demands for it, changes nothing on the wire.
fn check_parameters_compatible(
    operation: &str,
    old: &ParameterMap<'_>,
    new: &ParameterMap<'_>,
    old_spec: &OpenAPI,
    new_spec: &OpenAPI,
) -> Result<(), String> {
    for (key, old_param) in old {
        let (location, name) = (key.0, old_param.name.as_str());
        let Some(new_param) = new.get(key) else {
            if old_param.deprecated.unwrap_or(false) || location == "path" {
                continue;
            }
            return Err(format!(
                "Parameter '{}' ({}) was removed from {}",
                name, location, operation
            ));
        };
        if new_param.required && !old_param.required && location != "path" {
            return Err(format!(
                "Parameter '{}' ({}) of {} became required",
                name, location, operation
            ));
        }
        let label = format!("{} {} parameter {}", operation, location, name);
        if let (
            openapiv3::ParameterSchemaOrContent::Schema(old_schema),
            openapiv3::ParameterSchemaOrContent::Schema(new_schema),
        ) = (&old_param.format, &new_param.format)
            && let (Some(old_schema), Some(new_schema)) = (
                resolve_schema(old_spec, old_schema),
                resolve_schema(new_spec, new_schema),
            )
        {
            check_schema_compatible((old_spec, new_spec), &label, old_schema, new_schema, true)?;
        }
    }

    for (key, new_param) in new {
        if new_param.required && key.0 != "path" && !old.contains_key(key) {
            return Err(format!(
                "Required parameter '{}' ({}) was added to {}",
                new_param.name, key.0, operation
            ));
        }
    }

    Ok(())
}

/// Compare the body schemas of each media type both versions offer. Schemas are
/// resolved one level, so swapping an inline body for a `$ref` (or one `$ref`
/// for another) is compared by shape, not by spelling.
fn check_content_compatible(
    label: &str,
    old: (&OpenAPI, &openapiv3::Content),
    new: (&OpenAPI, &openapiv3::Content),
    is_request: bool,
) -> Result<(), String> {
    let ((old_spec, old_content), (new_spec, new_content)) = (old, new);
    for (media_type, old_media) in old_content {
        if let Some(new_media) = new_content.get(media_type)
            && let (Some(old_schema), Some(new_schema)) = (&old_media.schema, &new_media.schema)
            && let (Some(old_schema), Some(new_schema)) = (
                resolve_schema(old_spec, old_schema),
                resolve_schema(new_spec, new_schema),
            )
        {
            check_schema_compatible(
                (old_spec, new_spec),
                &format!("{} ({})", label, media_type),
                old_schema,
                new_schema,
                is_request,
            )?;
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
    fn test_extract_info_version_reads_semver() {
        let yaml = "openapi: 3.0.3\ninfo:\n  title: T\n  version: 2.1.3\npaths: {}\n";
        assert_eq!(extract_info_version(yaml).unwrap().to_string(), "2.1.3");
    }

    #[test]
    fn test_extract_info_version_missing_is_loud() {
        let yaml = "openapi: 3.0.3\ninfo:\n  title: T\npaths: {}\n";
        let err = extract_info_version(yaml).unwrap_err();
        assert!(err.contains("no info.version"), "got: {}", err);
    }

    #[test]
    fn test_extract_info_version_rejects_snapshot_suffix_with_pointer() {
        let yaml = "openapi: 3.0.3\ninfo:\n  version: 1.2.0-SNAPSHOT\npaths: {}\n";
        let err = extract_info_version(yaml).unwrap_err();
        assert!(err.contains("stability"), "got: {}", err);
        assert!(err.contains("1.2.0"), "got: {}", err);
    }

    #[test]
    fn test_extract_info_version_accepts_short_and_v_prefixed_forms() {
        for (input, canonical) in [
            ("1.0", "1.0.0"),
            ("2", "2.0.0"),
            ("v2.0.0", "2.0.0"),
            ("v1.2", "1.2.0"),
        ] {
            let yaml = format!(
                "openapi: 3.0.3\ninfo:\n  version: \"{}\"\npaths: {{}}\n",
                input
            );
            assert_eq!(
                extract_info_version(&yaml).unwrap().to_string(),
                canonical,
                "'{}' normalizes with implicit zeroes",
                input
            );
        }
    }

    #[test]
    fn test_extract_info_version_rejects_malformed_forms() {
        for bad in ["1.2.3.4", "one.two.three", "1.2.3+b7"] {
            let yaml = format!(
                "openapi: 3.0.3\ninfo:\n  version: \"{}\"\npaths: {{}}\n",
                bad
            );
            assert!(
                extract_info_version(&yaml).is_err(),
                "'{}' should be rejected",
                bad
            );
        }
    }

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

    /// A `GET /users` spec whose operation carries the given `parameters:`
    /// block (already indented for the operation level; empty for none).
    fn users_get_with_params(params: &str) -> String {
        format!(
            "openapi: 3.0.0\ninfo:\n  title: Test\n  version: 1.0.0\npaths:\n  /users:\n    get:\n{params}      responses:\n        '200':\n          description: OK\n"
        )
    }

    const LIMIT_OPTIONAL: &str = "      parameters:\n        - name: limit\n          in: query\n          schema:\n            type: integer\n";
    const LIMIT_REQUIRED: &str = "      parameters:\n        - name: limit\n          in: query\n          required: true\n          schema:\n            type: integer\n";

    #[test]
    fn test_breaking_change_new_required_parameter() {
        let err = check_backward_compatibility(
            &users_get_with_params(""),
            &users_get_with_params(LIMIT_REQUIRED),
        )
        .unwrap_err();
        assert_eq!(
            err,
            "Required parameter 'limit' (query) was added to GET /users"
        );
    }

    #[test]
    fn test_non_breaking_change_new_optional_parameter() {
        assert_eq!(
            check_backward_compatibility(
                &users_get_with_params(""),
                &users_get_with_params(LIMIT_OPTIONAL),
            ),
            Ok(())
        );
    }

    #[test]
    fn test_breaking_change_optional_parameter_made_required() {
        let err = check_backward_compatibility(
            &users_get_with_params(LIMIT_OPTIONAL),
            &users_get_with_params(LIMIT_REQUIRED),
        )
        .unwrap_err();
        assert_eq!(
            err,
            "Parameter 'limit' (query) of GET /users became required"
        );
    }

    #[test]
    fn test_breaking_change_removed_parameter() {
        let err = check_backward_compatibility(
            &users_get_with_params(LIMIT_OPTIONAL),
            &users_get_with_params(""),
        )
        .unwrap_err();
        assert_eq!(err, "Parameter 'limit' (query) was removed from GET /users");
    }

    #[test]
    fn test_removing_deprecated_parameter_is_not_breaking() {
        let deprecated = "      parameters:\n        - name: limit\n          in: query\n          deprecated: true\n          schema:\n            type: integer\n";
        assert_eq!(
            check_backward_compatibility(
                &users_get_with_params(deprecated),
                &users_get_with_params(""),
            ),
            Ok(())
        );
    }

    #[test]
    fn test_breaking_change_parameter_type_changed() {
        let as_string = LIMIT_OPTIONAL.replace("type: integer", "type: string");
        let err = check_backward_compatibility(
            &users_get_with_params(LIMIT_OPTIONAL),
            &users_get_with_params(&as_string),
        )
        .unwrap_err();
        assert_eq!(
            err,
            "Schema 'GET /users query parameter limit' changed type from 'integer' to 'string'"
        );
    }

    #[test]
    fn test_parameters_with_same_name_in_different_locations_are_distinct() {
        // Moving `limit` from the query to a header removes the query one.
        let as_header = LIMIT_OPTIONAL.replace("in: query", "in: header");
        let err = check_backward_compatibility(
            &users_get_with_params(LIMIT_OPTIONAL),
            &users_get_with_params(&as_header),
        )
        .unwrap_err();
        assert_eq!(err, "Parameter 'limit' (query) was removed from GET /users");
    }

    #[test]
    fn test_breaking_change_required_path_level_ref_parameter_added() {
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
paths:
  /users:
    parameters:
      - $ref: '#/components/parameters/Tenant'
    get:
      responses:
        '200':
          description: OK
components:
  parameters:
    Tenant:
      name: X-Tenant
      in: header
      required: true
      schema:
        type: string
"#;
        let err = check_backward_compatibility(old_yaml, new_yaml).unwrap_err();
        assert_eq!(
            err,
            "Required parameter 'X-Tenant' (header) was added to GET /users"
        );
    }

    #[test]
    fn test_operation_parameter_overrides_path_level_one() {
        // The path item makes `limit` required, the operation relaxes it to
        // optional: what the operation accepts is unchanged, so no break.
        let old_yaml = users_get_with_params(LIMIT_OPTIONAL);
        let new_yaml = old_yaml.replace(
            "  /users:\n    get:\n",
            "  /users:\n    parameters:\n      - name: limit\n        in: query\n        required: true\n        schema:\n          type: integer\n    get:\n",
        );
        assert_eq!(check_backward_compatibility(&old_yaml, &new_yaml), Ok(()));
    }

    const STATUS_ENUM_AB: &str = "      parameters:\n        - name: status\n          in: query\n          schema:\n            type: string\n            enum: [a, b]\n";

    #[test]
    fn test_breaking_change_request_enum_value_removed() {
        let narrowed = STATUS_ENUM_AB.replace("[a, b]", "[a]");
        let err = check_backward_compatibility(
            &users_get_with_params(STATUS_ENUM_AB),
            &users_get_with_params(&narrowed),
        )
        .unwrap_err();
        assert_eq!(
            err,
            "Enum value \"b\" was removed from request schema 'GET /users query parameter status'"
        );
    }

    #[test]
    fn test_non_breaking_change_request_enum_widened() {
        let widened = STATUS_ENUM_AB.replace("[a, b]", "[a, b, c]");
        assert_eq!(
            check_backward_compatibility(
                &users_get_with_params(STATUS_ENUM_AB),
                &users_get_with_params(&widened),
            ),
            Ok(())
        );
    }

    #[test]
    fn test_breaking_change_request_restricted_to_enum() {
        let unrestricted = STATUS_ENUM_AB.replace("            enum: [a, b]\n", "");
        let err = check_backward_compatibility(
            &users_get_with_params(&unrestricted),
            &users_get_with_params(STATUS_ENUM_AB),
        )
        .unwrap_err();
        assert_eq!(
            err,
            "Request schema 'GET /users query parameter status' was restricted to an enum"
        );
    }

    /// `POST /users` with an inline request body and an inline `200` response
    /// body; each `{...}` is spliced in as that body's `properties:` block.
    fn users_post_with_bodies(request_props: &str, response_props: &str) -> String {
        format!(
            r#"
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
              type: object
              properties:
{request_props}
      responses:
        '200':
          description: OK
          content:
            application/json:
              schema:
                type: object
                properties:
{response_props}
"#
        )
    }

    const REQ_ADDRESS: &str = "                address:\n                  type: object\n                  properties:\n                    zip:\n                      type: string\n";
    const RESP_ROLE: &str = "                  role:\n                    type: string\n                    enum: [admin, user]\n";

    #[test]
    fn test_breaking_change_nested_property_removed_from_inline_request_body() {
        let without_zip = REQ_ADDRESS.replace(
            "                  properties:\n                    zip:\n                      type: string\n",
            "                  properties: {}\n",
        );
        let err = check_backward_compatibility(
            &users_post_with_bodies(REQ_ADDRESS, RESP_ROLE),
            &users_post_with_bodies(&without_zip, RESP_ROLE),
        )
        .unwrap_err();
        assert_eq!(
            err,
            "Property 'zip' was removed from schema 'POST /users request body (application/json).address'"
        );
    }

    #[test]
    fn test_breaking_change_new_required_nested_field_in_inline_request_body() {
        let zip_required = REQ_ADDRESS.replace(
            "                  type: object\n",
            "                  type: object\n                  required: [zip]\n",
        );
        let err = check_backward_compatibility(
            &users_post_with_bodies(REQ_ADDRESS, RESP_ROLE),
            &users_post_with_bodies(&zip_required, RESP_ROLE),
        )
        .unwrap_err();
        assert_eq!(
            err,
            "New required field 'zip' was added to request schema 'POST /users request body (application/json).address'"
        );
    }

    #[test]
    fn test_breaking_change_type_changed_in_inline_response_body() {
        let role_as_int = "                  role:\n                    type: integer\n";
        let err = check_backward_compatibility(
            &users_post_with_bodies(REQ_ADDRESS, RESP_ROLE),
            &users_post_with_bodies(REQ_ADDRESS, role_as_int),
        )
        .unwrap_err();
        assert_eq!(
            err,
            "Property 'role' in schema 'POST /users response 200 (application/json)' changed type from 'string' to 'integer'"
        );
    }

    #[test]
    fn test_non_breaking_change_response_enum_value_added() {
        let widened = RESP_ROLE.replace("[admin, user]", "[admin, user, guest]");
        assert_eq!(
            check_backward_compatibility(
                &users_post_with_bodies(REQ_ADDRESS, RESP_ROLE),
                &users_post_with_bodies(REQ_ADDRESS, &widened),
            ),
            Ok(())
        );
    }

    #[test]
    fn test_breaking_change_array_item_type_changed_in_component() {
        let old_yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths: {}
components:
  schemas:
    User:
      type: object
      properties:
        tags:
          type: array
          items:
            type: string
"#;
        let new_yaml = old_yaml.replace(
            "          items:\n            type: string\n",
            "          items:\n            type: integer\n",
        );
        let err = check_backward_compatibility(old_yaml, &new_yaml).unwrap_err();
        assert_eq!(
            err,
            "Schema 'User.tags[]' changed type from 'string' to 'integer'"
        );
    }

    #[test]
    fn test_body_swapped_from_ref_to_equal_inline_schema_is_not_breaking() {
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
                type: object
                properties:
                  name:
                    type: string
"#;
        assert_eq!(check_backward_compatibility(old_yaml, new_yaml), Ok(()));
    }

    #[test]
    fn test_unchanged_self_referencing_spec_is_compatible() {
        // A schema that refers to itself must not send the recursion in circles.
        let yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths:
  /nodes:
    post:
      parameters:
        - name: depth
          in: query
          schema:
            type: integer
            enum: [1, 2, 3]
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/Node'
      responses:
        '200':
          description: OK
components:
  schemas:
    Node:
      type: object
      properties:
        children:
          type: array
          items:
            $ref: '#/components/schemas/Node'
        meta:
          type: object
          properties:
            label:
              type: string
"#;
        assert_eq!(check_backward_compatibility(yaml, yaml), Ok(()));
    }

    #[test]
    fn test_analyze_impact_new_required_parameter_is_major() {
        assert_eq!(
            analyze_impact(
                &users_get_with_params(""),
                &users_get_with_params(LIMIT_REQUIRED),
            ),
            Impact::Major
        );
    }

    /// `GET /users/{id}` with the given operation-level `parameters:` block.
    fn user_by_id_with_params(params: &str) -> String {
        format!(
            "openapi: 3.0.0\ninfo:\n  title: Test\n  version: 1.0.0\npaths:\n  /users/{{id}}:\n    get:\n{params}      responses:\n        '200':\n          description: OK\n"
        )
    }

    #[test]
    fn test_declaring_an_implicit_path_parameter_is_not_breaking() {
        let declared = "      parameters:\n        - name: id\n          in: path\n          required: true\n          schema:\n            type: string\n";
        assert_eq!(
            check_backward_compatibility(
                &user_by_id_with_params(""),
                &user_by_id_with_params(declared),
            ),
            Ok(())
        );
    }

    #[test]
    fn test_adding_required_true_to_a_path_parameter_is_not_breaking() {
        let implicit = "      parameters:\n        - name: id\n          in: path\n          schema:\n            type: string\n";
        let explicit = implicit.replace("in: path\n", "in: path\n          required: true\n");
        assert_eq!(
            check_backward_compatibility(
                &user_by_id_with_params(implicit),
                &user_by_id_with_params(&explicit),
            ),
            Ok(())
        );
    }

    #[test]
    fn test_breaking_change_path_parameter_type_changed() {
        let as_string = "      parameters:\n        - name: id\n          in: path\n          required: true\n          schema:\n            type: string\n";
        let as_integer = as_string.replace("type: string", "type: integer");
        let err = check_backward_compatibility(
            &user_by_id_with_params(as_string),
            &user_by_id_with_params(&as_integer),
        )
        .unwrap_err();
        assert_eq!(
            err,
            "Schema 'GET /users/{id} path parameter id' changed type from 'string' to 'integer'"
        );
    }

    #[test]
    fn test_header_parameter_names_are_case_insensitive() {
        let mixed = "      parameters:\n        - name: X-Tenant\n          in: header\n          required: true\n          schema:\n            type: string\n";
        let lower = mixed.replace("X-Tenant", "x-tenant");
        assert_eq!(
            check_backward_compatibility(
                &users_get_with_params(mixed),
                &users_get_with_params(&lower),
            ),
            Ok(())
        );
    }

    #[test]
    fn test_inline_nested_object_gaining_explicit_type_is_not_breaking() {
        let untyped = "                address:\n                  properties:\n                    zip:\n                      type: string\n";
        assert_eq!(
            check_backward_compatibility(
                &users_post_with_bodies(untyped, RESP_ROLE),
                &users_post_with_bodies(REQ_ADDRESS, RESP_ROLE),
            ),
            Ok(())
        );
    }

    #[test]
    fn test_inline_nested_object_extracted_to_equal_ref_is_not_breaking() {
        let old_yaml = users_post_with_bodies(REQ_ADDRESS, RESP_ROLE);
        let new_yaml = users_post_with_bodies(
            "                address:\n                  $ref: '#/components/schemas/Address'\n",
            RESP_ROLE,
        ) + "components:\n  schemas:\n    Address:\n      type: object\n      properties:\n        zip:\n          type: string\n";
        assert_eq!(check_backward_compatibility(&old_yaml, &new_yaml), Ok(()));
    }

    #[test]
    fn test_breaking_change_hidden_in_inline_to_ref_swap() {
        let old_yaml = users_post_with_bodies(REQ_ADDRESS, RESP_ROLE);
        let new_yaml = users_post_with_bodies(
            "                address:\n                  $ref: '#/components/schemas/Address'\n",
            RESP_ROLE,
        ) + "components:\n  schemas:\n    Address:\n      type: object\n      properties:\n        zip:\n          type: integer\n";
        let err = check_backward_compatibility(&old_yaml, &new_yaml).unwrap_err();
        assert_eq!(
            err,
            "Property 'zip' in schema 'POST /users request body (application/json).address' changed type from 'string' to 'integer'"
        );
    }

    #[test]
    fn test_property_ref_renamed_is_still_a_type_change() {
        let old_yaml = r#"
openapi: 3.0.0
info:
  title: Test
  version: 1.0.0
paths: {}
components:
  schemas:
    User:
      type: object
      properties:
        address:
          $ref: '#/components/schemas/Address'
    Address:
      type: object
    PostalAddress:
      type: object
"#;
        let new_yaml = old_yaml.replace(
            "          $ref: '#/components/schemas/Address'",
            "          $ref: '#/components/schemas/PostalAddress'",
        );
        let err = check_backward_compatibility(old_yaml, &new_yaml).unwrap_err();
        assert_eq!(
            err,
            "Property 'address' in schema 'User' changed type from '#/components/schemas/Address' to '#/components/schemas/PostalAddress'"
        );
    }
}
