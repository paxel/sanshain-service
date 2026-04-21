use serde_yaml::{Mapping, Value};

pub struct AsyncApiSpec {
    pub channel: String,
    pub operation: String, // PUB or SUB
    pub yaml_content: String,
}

pub fn split_asyncapi(yaml_str: &str) -> Result<Vec<AsyncApiSpec>, String> {
    let root: Value = serde_yaml::from_str(yaml_str)
        .map_err(|e| format!("Failed to parse AsyncAPI YAML: {}", e))?;

    let mut specs = Vec::new();

    if let Some(channels) = root.get("channels").and_then(|c| c.as_mapping()) {
        for (channel_name, channel_value) in channels {
            let channel_name_str = channel_name.as_str().ok_or("Channel name must be a string")?;

            if let Some(_publish) = channel_value.get("publish") {
                specs.push(create_spec(&root, channel_name_str, "PUB", channel_value, "publish")?);
            }
            if let Some(_subscribe) = channel_value.get("subscribe") {
                specs.push(create_spec(&root, channel_name_str, "SUB", channel_value, "subscribe")?);
            }
        }
    }

    Ok(specs)
}

fn create_spec(root: &Value, channel_name: &str, operation: &str, channel_value: &Value, op_key: &str) -> Result<AsyncApiSpec, String> {
    let mut snippet = Mapping::new();
    
    // Copy top-level fields except channels
    if let Some(m) = root.as_mapping() {
        for (k, v) in m {
            if k.as_str() != Some("channels") {
                snippet.insert(k.clone(), v.clone());
            }
        }
    }

    let mut channels = Mapping::new();
    let mut channel_map = channel_value.as_mapping().cloned().unwrap_or_default();
    
    // Remove other operations from this channel in the snippet
    let other_op = if op_key == "publish" { "subscribe" } else { "publish" };
    channel_map.remove(&Value::String(other_op.to_string()));
    
    channels.insert(Value::String(channel_name.to_string()), Value::Mapping(channel_map));
    snippet.insert(Value::String("channels".to_string()), Value::Mapping(channels));

    let yaml_content = serde_yaml::to_string(&Value::Mapping(snippet))
        .map_err(|e| format!("Failed to serialize AsyncAPI snippet: {}", e))?;

    Ok(AsyncApiSpec {
        channel: channel_name.to_string(),
        operation: operation.to_string(),
        yaml_content,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_asyncapi_2_x() {
        let yaml = r#"
asyncapi: 2.6.0
info:
  title: Test
  version: 1.0.0
channels:
  user-created:
    publish:
      message:
        payload:
          type: object
    subscribe:
      message:
        payload:
          type: object
  order-placed:
    publish:
      message:
        payload:
          type: object
"#;
        let result = split_asyncapi(yaml).unwrap();
        assert_eq!(result.len(), 3);
        
        let user_created_pub = result.iter().find(|s| s.channel == "user-created" && s.operation == "PUB").unwrap();
        assert!(user_created_pub.yaml_content.contains("publish:"));
        assert!(!user_created_pub.yaml_content.contains("subscribe:"));
        assert!(user_created_pub.yaml_content.contains("asyncapi: 2.6.0"));
        
        let order_placed_pub = result.iter().find(|s| s.channel == "order-placed" && s.operation == "PUB").unwrap();
        assert!(order_placed_pub.yaml_content.contains("order-placed"));
    }
}
