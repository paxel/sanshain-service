use regex::Regex;
use std::ops::Range;
use std::sync::LazyLock;

static RE_RPC: LazyLock<Result<Regex, regex::Error>> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*rpc\s+(\w+)\s*\(([^)]+)\)\s*returns\s*\(([^)]+)\)\s*(?:\{[^}]*\}|;)")
});
static RE_SERVICE: LazyLock<Result<Regex, regex::Error>> =
    LazyLock::new(|| Regex::new(r"\bservice\s+([A-Za-z_]\w*)\s*\{"));

struct ServiceBlock {
    name: String,
    body: String,
    range: Range<usize>,
}

pub struct ProtoSpec {
    pub service: String,
    pub method: String,
    pub content: String,
}

pub fn split_proto(content: &str) -> Result<Vec<ProtoSpec>, String> {
    let re_rpc = RE_RPC
        .as_ref()
        .map_err(|e| format!("Failed to compile rpc regex: {}", e))?;
    let re_service = RE_SERVICE
        .as_ref()
        .map_err(|e| format!("Failed to compile service regex: {}", e))?;

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
}
