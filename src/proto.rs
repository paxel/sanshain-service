use regex::Regex;
use std::sync::LazyLock;

static RE_RPC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*rpc\s+(\w+)\s*\(([^)]+)\)\s*returns\s*\(([^)]+)\)\s*(?:\{[^}]*\}|;)")
        .expect("failed to compile rpc regex")
});

pub struct ProtoSpec {
    pub service: String,
    pub method: String,
    pub content: String,
}

pub fn split_proto(content: &str) -> Result<Vec<ProtoSpec>, String> {
    let mut specs = Vec::new();

    // Find all service blocks using balanced braces
    let mut services = Vec::new();
    let bytes = content.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if content[i..].starts_with("service ") {
            let start_idx = i;
            if let Some(brace_start_rel) = content[i..].find('{') {
                let brace_start = i + brace_start_rel;
                let service_name = content[start_idx..brace_start]
                    .split_whitespace()
                    .last()
                    .unwrap_or("Unknown")
                    .to_string();

                let mut brace_count = 1;
                let mut j = brace_start + 1;
                while brace_count > 0 && j < bytes.len() {
                    if bytes[j] == b'{' {
                        brace_count += 1;
                    } else if bytes[j] == b'}' {
                        brace_count -= 1;
                    }
                    j += 1;
                }
                let body = content[brace_start + 1..j - 1].to_string();
                services.push((service_name, body));
                i = j;
                continue;
            }
        }
        i += 1;
    }

    // Get non-service lines for common base
    let mut common_base = String::new();
    let mut in_service = false;
    let mut brace_count = 0;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("service ") && trimmed.contains('{') {
            in_service = true;
            brace_count += 1;
            continue;
        }
        if in_service {
            if trimmed.contains('{') {
                brace_count += 1;
            }
            if trimmed.contains('}') {
                brace_count -= 1;
            }
            if brace_count == 0 {
                in_service = false;
            }
            continue;
        }
        common_base.push_str(line);
        common_base.push('\n');
    }

    let re_rpc = &*RE_RPC;

    for (service_name, body) in services {
        for rpc_cap in re_rpc.captures_iter(&body) {
            let method_name = &rpc_cap[1];
            let rpc_line = rpc_cap[0].trim();

            let mut snippet = common_base.clone();
            snippet.push_str(&format!("\nservice {} {{\n", service_name));
            snippet.push_str("  ");
            snippet.push_str(rpc_line);
            if !rpc_line.ends_with(';') && !rpc_line.ends_with('}') {
                snippet.push(';');
            }
            snippet.push_str("\n}\n");

            specs.push(ProtoSpec {
                service: service_name.clone(),
                method: method_name.to_string(),
                content: snippet,
            });
        }
    }

    Ok(specs)
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
}
