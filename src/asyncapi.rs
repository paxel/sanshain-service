use serde_yaml::{Mapping, Value};

pub struct AsyncApiSpec {
    pub channel: String,
    pub operation: String, // PUB or SUB
    pub yaml_content: String,
}

pub fn split_asyncapi(yaml_str: &str) -> Result<Vec<AsyncApiSpec>, String> {
    let root: Value = serde_yaml::from_str(yaml_str)
        .map_err(|e| format!("Failed to parse AsyncAPI YAML: {}", e))?;

    let version = root.get("asyncapi").and_then(|v| v.as_str()).unwrap_or("");
    if version.starts_with("3.") {
        split_asyncapi_v3(&root)
    } else {
        split_asyncapi_v2(&root)
    }
}

fn split_asyncapi_v2(root: &Value) -> Result<Vec<AsyncApiSpec>, String> {
    let mut specs = Vec::new();

    if let Some(channels) = root.get("channels").and_then(|c| c.as_mapping()) {
        for (channel_name, channel_value) in channels {
            let channel_name_str = channel_name.as_str().ok_or("Channel name must be a string")?;

            if let Some(_publish) = channel_value.get("publish") {
                specs.push(create_spec_v2(root, channel_name_str, "PUB", channel_value, "publish")?);
            }
            if let Some(_subscribe) = channel_value.get("subscribe") {
                specs.push(create_spec_v2(root, channel_name_str, "SUB", channel_value, "subscribe")?);
            }
        }
    }

    Ok(specs)
}

fn split_asyncapi_v3(root: &Value) -> Result<Vec<AsyncApiSpec>, String> {
    let mut specs = Vec::new();

    let operations = root.get("operations").and_then(|o| o.as_mapping())
        .ok_or_else(|| "AsyncAPI 3.x document has no 'operations' section".to_string())?;

    let channels = root.get("channels").and_then(|c| c.as_mapping());

    for (_op_name, op_value) in operations {
        let action = op_value.get("action").and_then(|a| a.as_str())
            .ok_or_else(|| "Operation missing 'action' field".to_string())?;

        let operation = match action {
            "send" => "PUB",
            "receive" => "SUB",
            other => return Err(format!("Unknown operation action: {}", other)),
        };

        // Resolve channel reference
        let channel_ref = op_value.get("channel").and_then(|c| c.get("$ref")).and_then(|r| r.as_str());
        let channel_name = if let Some(ref_str) = channel_ref {
            // e.g. "#/channels/UserSignup"
            ref_str.strip_prefix("#/channels/").ok_or_else(|| format!("Unsupported channel $ref: {}", ref_str))?
        } else {
            // channel might be inline or referenced by key directly
            return Err("Operation missing channel.$ref".to_string());
        };

        // Resolve the channel address (the actual topic/path name) or fall back to the channel key
        let channel_address = channels
            .and_then(|chs| chs.get(Value::String(channel_name.to_string())))
            .and_then(|ch| ch.get("address"))
            .and_then(|a| a.as_str())
            .unwrap_or(channel_name);

        specs.push(create_spec_v3(root, channel_name, channel_address, operation, op_value, channels)?);
    }

    Ok(specs)
}

fn create_spec_v3(
    root: &Value,
    channel_key: &str,
    channel_address: &str,
    operation: &str,
    op_value: &Value,
    channels: Option<&Mapping>,
) -> Result<AsyncApiSpec, String> {
    let mut snippet = Mapping::new();

    // Copy top-level fields except channels and operations
    if let Some(m) = root.as_mapping() {
        for (k, v) in m {
            let key_str = k.as_str().unwrap_or("");
            if key_str != "channels" && key_str != "operations" {
                snippet.insert(k.clone(), v.clone());
            }
        }
    }

    // Include only the referenced channel
    if let Some(chs) = channels {
        if let Some(ch_value) = chs.get(Value::String(channel_key.to_string())) {
            let mut ch_map = Mapping::new();
            ch_map.insert(Value::String(channel_key.to_string()), ch_value.clone());
            snippet.insert(Value::String("channels".to_string()), Value::Mapping(ch_map));
        }
    }

    // Include only this operation
    let mut ops = Mapping::new();
    // Use a synthetic key based on action + channel
    let op_key = format!("{}_{}", operation.to_lowercase(), channel_key);
    ops.insert(Value::String(op_key), op_value.clone());
    snippet.insert(Value::String("operations".to_string()), Value::Mapping(ops));

    let yaml_content = serde_yaml::to_string(&Value::Mapping(snippet))
        .map_err(|e| format!("Failed to serialize AsyncAPI snippet: {}", e))?;

    Ok(AsyncApiSpec {
        channel: channel_address.to_string(),
        operation: operation.to_string(),
        yaml_content,
    })
}

fn create_spec_v2(root: &Value, channel_name: &str, operation: &str, channel_value: &Value, op_key: &str) -> Result<AsyncApiSpec, String> {
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
    channel_map.remove(Value::String(other_op.to_string()));
    
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

    #[test]
    fn test_split_asyncapi_3_x() {
        let yaml = r#"
asyncapi: 3.0.0
info:
  title: Test
  version: 1.0.0
channels:
  UserSignup:
    address: user/signedup
    messages:
      UserMessage:
        payload:
          type: object
  OrderCreated:
    address: orders.created
    messages:
      OrderMessage:
        payload:
          type: object
operations:
  consumeUserSignups:
    action: receive
    channel:
      $ref: '#/channels/UserSignup'
  publishOrder:
    action: send
    channel:
      $ref: '#/channels/OrderCreated'
"#;
        let result = split_asyncapi(yaml).unwrap();
        assert_eq!(result.len(), 2);

        let user_sub = result.iter().find(|s| s.channel == "user/signedup" && s.operation == "SUB").unwrap();
        assert!(user_sub.yaml_content.contains("asyncapi: 3.0.0"));
        assert!(user_sub.yaml_content.contains("UserSignup"));
        assert!(!user_sub.yaml_content.contains("OrderCreated"));

        let order_pub = result.iter().find(|s| s.channel == "orders.created" && s.operation == "PUB").unwrap();
        assert!(order_pub.yaml_content.contains("OrderCreated"));
        assert!(!order_pub.yaml_content.contains("UserSignup"));
    }

    #[test]
    fn test_split_asyncapi_3_x_fallback_to_channel_key() {
        let yaml = r#"
asyncapi: 3.0.0
info:
  title: Test
  version: 1.0.0
channels:
  notifications:
    messages:
      NotifMessage:
        payload:
          type: object
operations:
  sendNotification:
    action: send
    channel:
      $ref: '#/channels/notifications'
"#;
        let result = split_asyncapi(yaml).unwrap();
        assert_eq!(result.len(), 1);
        // Without an address field, falls back to channel key name
        assert_eq!(result[0].channel, "notifications");
        assert_eq!(result[0].operation, "PUB");
    }
}
