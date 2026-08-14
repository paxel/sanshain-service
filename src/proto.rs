use regex::Regex;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::LazyLock;

static RE_RPC: LazyLock<Result<Regex, regex::Error>> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*rpc\s+(\w+)\s*\(([^)]+)\)\s*returns\s*\(([^)]+)\)\s*(?:\{[^}]*\}|;)")
});
static RE_SERVICE: LazyLock<Result<Regex, regex::Error>> =
    LazyLock::new(|| Regex::new(r"\bservice\s+([A-Za-z_]\w*)\s*\{"));
static RE_MESSAGE: LazyLock<Result<Regex, regex::Error>> =
    LazyLock::new(|| Regex::new(r"\bmessage\s+([A-Za-z_]\w*)\s*\{"));
static RE_FIELD: LazyLock<Result<Regex, regex::Error>> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^\s*(repeated|optional|required)?\s*((?:map\s*<[^>]+>)|(?:[A-Za-z_][\w.]*))\s+([A-Za-z_]\w*)\s*=\s*(\d+)\s*(\[[^\]]*\])?\s*;",
    )
});
static RE_OPTION_DEPRECATED: LazyLock<Result<Regex, regex::Error>> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*option\s+deprecated\s*=\s*true\s*;"));
static RE_DEPRECATED_TRUE: LazyLock<Result<Regex, regex::Error>> =
    LazyLock::new(|| Regex::new(r"deprecated\s*=\s*true"));
static RE_SANSHAIN_VERSION: LazyLock<Result<Regex, regex::Error>> =
    LazyLock::new(|| Regex::new(r"(?m)//\s*sanshain-version:\s*(\S+)"));

struct ServiceBlock {
    name: String,
    body: String,
    range: Range<usize>,
}

pub struct ProtoSpec {
    pub service: String,
    pub method: String,
    pub content: String,
    pub deprecated: bool,
}

/// Read the mandatory `// sanshain-version: MAJOR.MINOR.PATCH` marker out of a
/// proto file. Proto has no standard version slot, so the marker is required
/// and every failure mode is loud: no marker rejects the Provide with the
/// expected syntax, and multiple markers with different values are ambiguous.
/// The marker lives in the file-level content the splitter carries along, so
/// it travels into the split files Consumers download.
pub fn extract_sanshain_version(content: &str) -> Result<crate::domain::models::SemVer, String> {
    let re = RE_SANSHAIN_VERSION
        .as_ref()
        .map_err(|e| format!("Failed to compile version regex: {}", e))?;

    let mut values: Vec<&str> = re
        .captures_iter(content)
        .filter_map(|c| c.get(1).map(|m| m.as_str()))
        .collect();
    values.dedup();

    match values.as_slice() {
        [] => Err(
            "proto file has no version marker — add a comment '// sanshain-version: MAJOR.MINOR.PATCH' (e.g. '// sanshain-version: 1.2.0')"
                .to_string(),
        ),
        [single] => crate::domain::models::SemVer::parse_spec_version(single)
            .map_err(|e| format!("sanshain-version marker: {}", e)),
        many => {
            let unique: std::collections::BTreeSet<&str> = many.iter().copied().collect();
            if unique.len() == 1 {
                crate::domain::models::SemVer::parse_spec_version(many[0])
                    .map_err(|e| format!("sanshain-version marker: {}", e))
            } else {
                Err(format!(
                    "proto file has {} conflicting sanshain-version markers ({}) — keep exactly one",
                    unique.len(),
                    unique.into_iter().collect::<Vec<_>>().join(", ")
                ))
            }
        }
    }
}

pub fn split_proto(content: &str) -> Result<Vec<ProtoSpec>, String> {
    let re_rpc = RE_RPC
        .as_ref()
        .map_err(|e| format!("Failed to compile rpc regex: {}", e))?;
    let re_service = RE_SERVICE
        .as_ref()
        .map_err(|e| format!("Failed to compile service regex: {}", e))?;

    let re_deprecated = RE_DEPRECATED_TRUE
        .as_ref()
        .map_err(|e| format!("Failed to compile deprecated regex: {}", e))?;

    let mut specs = Vec::new();
    let services = extract_service_blocks(re_service, content);
    let common_base = remove_service_blocks(content, &services);

    for service in services {
        for rpc_cap in re_rpc.captures_iter(&service.body) {
            let method_name = &rpc_cap[1];
            let rpc_line = rpc_cap[0].trim();

            let mut snippet = common_base.clone();
            snippet.push_str(&format!("\nservice {} {{\n", service.name));
            snippet.push_str("  ");
            snippet.push_str(rpc_line);
            if !rpc_line.ends_with(';') && !rpc_line.ends_with('}') {
                snippet.push(';');
            }
            snippet.push_str("\n}\n");

            specs.push(ProtoSpec {
                service: service.name.clone(),
                method: method_name.to_string(),
                content: snippet,
                deprecated: re_deprecated.is_match(rpc_line),
            });
        }
    }

    Ok(specs)
}

fn extract_service_blocks(re_service: &Regex, content: &str) -> Vec<ServiceBlock> {
    re_service
        .captures_iter(content)
        .filter_map(|captures| {
            let service_match = captures.get(0)?;
            let name = captures.get(1)?.as_str().to_string();
            let open_brace = service_match.end() - 1;
            let close_brace = find_matching_brace(content, open_brace)?;
            Some(ServiceBlock {
                name,
                body: content[open_brace + 1..close_brace].to_string(),
                range: service_match.start()..close_brace + 1,
            })
        })
        .collect()
}

fn find_matching_brace(content: &str, open_brace: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, ch) in content[open_brace..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(open_brace + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn remove_service_blocks(content: &str, services: &[ServiceBlock]) -> String {
    let mut common_base = String::new();
    let mut cursor = 0;
    for service in services {
        common_base.push_str(&content[cursor..service.range.start]);
        cursor = service.range.end;
    }
    common_base.push_str(&content[cursor..]);
    common_base
}

struct ProtoField {
    label: String,
    field_type: String,
    number: u64,
    deprecated: bool,
}

struct ProtoMessage {
    deprecated: bool,
    fields: HashMap<String, ProtoField>,
}

struct ProtoRpc {
    request: String,
    response: String,
    deprecated: bool,
}

/// Check if a new proto endpoint snippet is backward-compatible with the old one.
/// Breaking changes:
/// - an rpc was removed or its request/response types changed (unless the old
///   rpc carried `option deprecated = true`)
/// - a message was removed (unless it carried `option deprecated = true`)
/// - a message field was removed (unless it was marked `[deprecated = true]`)
/// - a field changed its number, type, or repeated/optional label
///
/// Adding rpcs, messages, and fields is OK. Enum changes are not analyzed.
///
/// Returns Ok(()) if compatible, Err(description) if breaking.
pub fn check_backward_compatibility(old: &str, new: &str) -> Result<(), String> {
    let old_rpcs = parse_rpcs(old)?;
    let new_rpcs = parse_rpcs(new)?;
    for ((service, method), old_rpc) in &old_rpcs {
        match new_rpcs.get(&(service.clone(), method.clone())) {
            None => {
                if !old_rpc.deprecated {
                    return Err(format!(
                        "rpc '{}' was removed from service '{}'",
                        method, service
                    ));
                }
            }
            Some(new_rpc) => {
                if old_rpc.request != new_rpc.request || old_rpc.response != new_rpc.response {
                    return Err(format!(
                        "rpc '{}' in service '{}' changed signature from ({}) returns ({}) to ({}) returns ({})",
                        method,
                        service,
                        old_rpc.request,
                        old_rpc.response,
                        new_rpc.request,
                        new_rpc.response
                    ));
                }
            }
        }
    }

    let old_messages = parse_messages(old)?;
    let new_messages = parse_messages(new)?;
    for (name, old_message) in &old_messages {
        let Some(new_message) = new_messages.get(name) else {
            if old_message.deprecated {
                continue;
            }
            return Err(format!("Message '{}' was removed", name));
        };
        for (field_name, old_field) in &old_message.fields {
            let Some(new_field) = new_message.fields.get(field_name) else {
                if old_field.deprecated {
                    continue;
                }
                return Err(format!(
                    "Field '{}' was removed from message '{}'",
                    field_name, name
                ));
            };
            if old_field.number != new_field.number {
                return Err(format!(
                    "Field '{}' in message '{}' changed number from {} to {}",
                    field_name, name, old_field.number, new_field.number
                ));
            }
            if old_field.field_type != new_field.field_type {
                return Err(format!(
                    "Field '{}' in message '{}' changed type from '{}' to '{}'",
                    field_name, name, old_field.field_type, new_field.field_type
                ));
            }
            if old_field.label != new_field.label {
                return Err(format!(
                    "Field '{}' in message '{}' changed label from '{}' to '{}'",
                    field_name, name, old_field.label, new_field.label
                ));
            }
        }
    }

    Ok(())
}

fn parse_rpcs(content: &str) -> Result<HashMap<(String, String), ProtoRpc>, String> {
    let re_rpc = RE_RPC
        .as_ref()
        .map_err(|e| format!("Failed to compile rpc regex: {}", e))?;
    let re_service = RE_SERVICE
        .as_ref()
        .map_err(|e| format!("Failed to compile service regex: {}", e))?;
    let re_deprecated = RE_DEPRECATED_TRUE
        .as_ref()
        .map_err(|e| format!("Failed to compile deprecated regex: {}", e))?;

    let mut rpcs = HashMap::new();
    for service in extract_service_blocks(re_service, content) {
        for rpc_cap in re_rpc.captures_iter(&service.body) {
            rpcs.insert(
                (service.name.clone(), rpc_cap[1].to_string()),
                ProtoRpc {
                    request: normalize_type(&rpc_cap[2]),
                    response: normalize_type(&rpc_cap[3]),
                    deprecated: re_deprecated.is_match(&rpc_cap[0]),
                },
            );
        }
    }
    Ok(rpcs)
}

/// Collapse insignificant whitespace in a type expression such as
/// `stream  Req` or `map < string , int32 >`.
fn normalize_type(type_expr: &str) -> String {
    type_expr.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parse all `message` blocks (including nested ones, under qualified names
/// like `Outer.Inner`) into comparable field maps.
fn parse_messages(content: &str) -> Result<HashMap<String, ProtoMessage>, String> {
    let re_message = RE_MESSAGE
        .as_ref()
        .map_err(|e| format!("Failed to compile message regex: {}", e))?;
    let re_field = RE_FIELD
        .as_ref()
        .map_err(|e| format!("Failed to compile field regex: {}", e))?;
    let re_option_deprecated = RE_OPTION_DEPRECATED
        .as_ref()
        .map_err(|e| format!("Failed to compile option regex: {}", e))?;
    let re_deprecated = RE_DEPRECATED_TRUE
        .as_ref()
        .map_err(|e| format!("Failed to compile deprecated regex: {}", e))?;

    let mut messages = HashMap::new();
    collect_message_blocks(
        content,
        "",
        re_message,
        re_field,
        re_option_deprecated,
        re_deprecated,
        &mut messages,
    );
    Ok(messages)
}

fn collect_message_blocks(
    content: &str,
    prefix: &str,
    re_message: &Regex,
    re_field: &Regex,
    re_option_deprecated: &Regex,
    re_deprecated: &Regex,
    messages: &mut HashMap<String, ProtoMessage>,
) {
    let mut last_end = 0;
    for captures in re_message.captures_iter(content) {
        let Some(message_match) = captures.get(0) else {
            continue;
        };
        // Skip matches inside a previously handled block; nested messages are
        // collected by the recursive call below.
        if message_match.start() < last_end {
            continue;
        }
        let Some(name) = captures.get(1) else {
            continue;
        };
        let open_brace = message_match.end() - 1;
        let Some(close_brace) = find_matching_brace(content, open_brace) else {
            continue;
        };
        last_end = close_brace + 1;

        let body = &content[open_brace + 1..close_brace];
        let qualified_name = if prefix.is_empty() {
            name.as_str().to_string()
        } else {
            format!("{}.{}", prefix, name.as_str())
        };

        collect_message_blocks(
            body,
            &qualified_name,
            re_message,
            re_field,
            re_option_deprecated,
            re_deprecated,
            messages,
        );

        // Scan fields on the body with nested message blocks removed, so
        // nested fields are not attributed to the parent message.
        let own_body = remove_message_blocks(body, re_message);
        let mut fields = HashMap::new();
        for field_cap in re_field.captures_iter(&own_body) {
            let number = field_cap[4].parse::<u64>().unwrap_or(0);
            fields.insert(
                field_cap[3].to_string(),
                ProtoField {
                    label: field_cap
                        .get(1)
                        .map(|l| l.as_str().to_string())
                        .unwrap_or_default(),
                    field_type: normalize_type(&field_cap[2]),
                    number,
                    deprecated: field_cap
                        .get(5)
                        .map(|opts| re_deprecated.is_match(opts.as_str()))
                        .unwrap_or(false),
                },
            );
        }

        messages.insert(
            qualified_name,
            ProtoMessage {
                deprecated: re_option_deprecated.is_match(&own_body),
                fields,
            },
        );
    }
}

fn remove_message_blocks(content: &str, re_message: &Regex) -> String {
    let mut result = String::new();
    let mut cursor = 0;
    for message_match in re_message.find_iter(content) {
        if message_match.start() < cursor {
            continue;
        }
        let open_brace = message_match.end() - 1;
        let Some(close_brace) = find_matching_brace(content, open_brace) else {
            continue;
        };
        result.push_str(&content[cursor..message_match.start()]);
        cursor = close_brace + 1;
    }
    result.push_str(&content[cursor..]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_proto() {
        let content = r#"
syntax = "proto3";
package test;

message Req {}
message Res {}

service UserService {
  rpc GetUser (Req) returns (Res);
  rpc CreateUser (Req) returns (Res) {}
}

service HealthService {
  rpc Check (Req) returns (Res);
}
"#;
        let result = split_proto(content).unwrap();
        assert_eq!(result.len(), 3);

        let get_user = result
            .iter()
            .find(|s| s.service == "UserService" && s.method == "GetUser")
            .unwrap();
        assert!(get_user.content.contains("syntax = \"proto3\""));
        assert!(get_user.content.contains("service UserService"));
        assert!(get_user.content.contains("rpc GetUser"));
        assert!(!get_user.content.contains("rpc CreateUser"));
        assert!(!get_user.content.contains("service HealthService"));
        assert!(get_user.content.contains("message Req"));

        let check = result
            .iter()
            .find(|s| s.service == "HealthService" && s.method == "Check")
            .unwrap();
        assert!(check.content.contains("service HealthService"));
    }

    #[test]
    fn test_split_proto_handles_non_ascii_before_service() {
        let content = r#"
syntax = "proto3";
// Grüße from a generated spec header.
message Req {}
message Res {}

service GreetingService {
  rpc SayHello (Req) returns (Res);
}
"#;

        let result = split_proto(content).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].service, "GreetingService");
        assert_eq!(result[0].method, "SayHello");
        assert!(result[0].content.contains("Grüße"));
    }

    #[test]
    fn test_split_proto_ignores_malformed_service_block() {
        let content = r#"
syntax = "proto3";
message Req {}
message Res {}

service BrokenService {
  rpc Broken (Req) returns (Res);
"#;

        let result = split_proto(content).unwrap();

        assert!(result.is_empty());
    }

    #[test]
    fn test_split_proto_detects_deprecated_rpc() {
        let content = r#"
syntax = "proto3";
message Req {}
message Res {}

service UserService {
  rpc OldCall (Req) returns (Res) { option deprecated = true; }
  rpc NewCall (Req) returns (Res);
}
"#;
        let result = split_proto(content).unwrap();
        let old_call = result.iter().find(|s| s.method == "OldCall").unwrap();
        let new_call = result.iter().find(|s| s.method == "NewCall").unwrap();
        assert!(old_call.deprecated);
        assert!(!new_call.deprecated);
    }

    const COMPAT_BASE: &str = r#"
syntax = "proto3";

message Req {
  string id = 1;
  int32 age = 2;
}
message Res {
  repeated string names = 1;
}

service UserService {
  rpc GetUser (Req) returns (Res);
}
"#;

    #[test]
    fn test_compat_identical_and_additive_ok() {
        assert_eq!(
            check_backward_compatibility(COMPAT_BASE, COMPAT_BASE),
            Ok(())
        );

        let added = COMPAT_BASE.replace("int32 age = 2;", "int32 age = 2;\n  string email = 3;");
        assert_eq!(check_backward_compatibility(COMPAT_BASE, &added), Ok(()));
    }

    #[test]
    fn test_compat_removed_field_is_breaking_unless_deprecated() {
        let removed = COMPAT_BASE.replace("  int32 age = 2;\n", "");
        let err = check_backward_compatibility(COMPAT_BASE, &removed).unwrap_err();
        assert_eq!(err, "Field 'age' was removed from message 'Req'");

        let deprecated =
            COMPAT_BASE.replace("int32 age = 2;", "int32 age = 2 [deprecated = true];");
        let removed_deprecated = deprecated.replace("  int32 age = 2 [deprecated = true];\n", "");
        assert_eq!(
            check_backward_compatibility(&deprecated, &removed_deprecated),
            Ok(())
        );
    }

    #[test]
    fn test_compat_field_number_change_is_breaking() {
        let changed = COMPAT_BASE.replace("int32 age = 2;", "int32 age = 5;");
        let err = check_backward_compatibility(COMPAT_BASE, &changed).unwrap_err();
        assert_eq!(
            err,
            "Field 'age' in message 'Req' changed number from 2 to 5"
        );
    }

    #[test]
    fn test_compat_field_type_change_is_breaking() {
        let changed = COMPAT_BASE.replace("int32 age = 2;", "string age = 2;");
        let err = check_backward_compatibility(COMPAT_BASE, &changed).unwrap_err();
        assert_eq!(
            err,
            "Field 'age' in message 'Req' changed type from 'int32' to 'string'"
        );
    }

    #[test]
    fn test_compat_label_change_is_breaking() {
        let changed = COMPAT_BASE.replace("repeated string names = 1;", "string names = 1;");
        let err = check_backward_compatibility(COMPAT_BASE, &changed).unwrap_err();
        assert_eq!(
            err,
            "Field 'names' in message 'Res' changed label from 'repeated' to ''"
        );
    }

    #[test]
    fn test_compat_rpc_signature_change_is_breaking() {
        let changed = COMPAT_BASE.replace(
            "rpc GetUser (Req) returns (Res);",
            "rpc GetUser (Req) returns (stream Res);",
        );
        let err = check_backward_compatibility(COMPAT_BASE, &changed).unwrap_err();
        assert_eq!(
            err,
            "rpc 'GetUser' in service 'UserService' changed signature from (Req) returns (Res) to (Req) returns (stream Res)"
        );
    }

    #[test]
    fn test_compat_removed_message_is_breaking_unless_deprecated() {
        let removed = COMPAT_BASE.replace("message Res {\n  repeated string names = 1;\n}\n", "");
        let err = check_backward_compatibility(COMPAT_BASE, &removed).unwrap_err();
        // Depending on scan order the rpc referencing Res is unchanged, so the
        // message-level check reports the removal.
        assert_eq!(err, "Message 'Res' was removed");

        let deprecated = COMPAT_BASE.replace(
            "message Res {\n",
            "message Res {\n  option deprecated = true;\n",
        );
        let removed_deprecated = deprecated.replace(
            "message Res {\n  option deprecated = true;\n  repeated string names = 1;\n}\n",
            "",
        );
        assert_eq!(
            check_backward_compatibility(&deprecated, &removed_deprecated),
            Ok(())
        );
    }

    #[test]
    fn test_compat_nested_messages_use_qualified_names() {
        let base = r#"
message Outer {
  string kept = 1;
  message Inner {
    string dropped = 1;
  }
}
"#;
        let changed = r#"
message Outer {
  string kept = 1;
  message Inner {
  }
}
"#;
        let err = check_backward_compatibility(base, changed).unwrap_err();
        assert_eq!(
            err,
            "Field 'dropped' was removed from message 'Outer.Inner'"
        );
    }

    #[test]
    fn test_extract_sanshain_version_reads_the_marker() {
        let content = "syntax = \"proto3\";\n// sanshain-version: 1.2.0\npackage a.b;\n";
        let version = extract_sanshain_version(content).unwrap();
        assert_eq!(version.to_string(), "1.2.0");
    }

    #[test]
    fn test_extract_sanshain_version_missing_marker_is_loud() {
        let err = extract_sanshain_version("syntax = \"proto3\";\n").unwrap_err();
        assert!(
            err.contains("// sanshain-version: MAJOR.MINOR.PATCH"),
            "error must show the expected syntax, got: {}",
            err
        );
    }

    #[test]
    fn test_extract_sanshain_version_conflicting_markers_are_ambiguous() {
        let content = "// sanshain-version: 1.2.0\n// sanshain-version: 1.3.0\n";
        let err = extract_sanshain_version(content).unwrap_err();
        assert!(err.contains("conflicting"), "got: {}", err);
        assert!(
            err.contains("1.2.0") && err.contains("1.3.0"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_extract_sanshain_version_repeated_identical_markers_are_fine() {
        let content = "// sanshain-version: 2.0.1\nmessage M {}\n// sanshain-version: 2.0.1\n";
        let version = extract_sanshain_version(content).unwrap();
        assert_eq!(version.to_string(), "2.0.1");
    }

    #[test]
    fn test_extract_sanshain_version_accepts_v_prefix_and_short_forms() {
        let version = extract_sanshain_version("// sanshain-version: v1.2\n").unwrap();
        assert_eq!(version.to_string(), "1.2.0");
    }

    #[test]
    fn test_extract_sanshain_version_rejects_non_semver() {
        let err = extract_sanshain_version("// sanshain-version: one.two\n").unwrap_err();
        assert!(err.contains("MAJOR[.MINOR[.PATCH]]"), "got: {}", err);
    }
}
