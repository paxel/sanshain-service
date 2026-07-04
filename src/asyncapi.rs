use serde_yaml_ng::{Mapping, Value};
use std::collections::HashMap;

pub struct AsyncApiSpec {
    pub channel: String,
    pub operation: String, // PUB or SUB
    pub yaml_content: String,
    pub deprecated: bool,
}

pub fn split_asyncapi(yaml_str: &str) -> Result<Vec<AsyncApiSpec>, String> {
    let root: Value = serde_yaml_ng::from_str(yaml_str)
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
            let channel_name_str = channel_name
                .as_str()
                .ok_or("Channel name must be a string")?;

            if let Some(_publish) = channel_value.get("publish") {
                specs.push(create_spec_v2(
                    root,
                    channel_name_str,
                    "PUB",
                    channel_value,
                    "publish",
                )?);
            }
            if let Some(_subscribe) = channel_value.get("subscribe") {
                specs.push(create_spec_v2(
                    root,
                    channel_name_str,
                    "SUB",
                    channel_value,
                    "subscribe",
                )?);
            }
        }
    }

    Ok(specs)
}

fn split_asyncapi_v3(root: &Value) -> Result<Vec<AsyncApiSpec>, String> {
    let mut specs = Vec::new();

    let operations = root
        .get("operations")
        .and_then(|o| o.as_mapping())
        .ok_or_else(|| "AsyncAPI 3.x document has no 'operations' section".to_string())?;

    let channels = root.get("channels").and_then(|c| c.as_mapping());

    for (_op_name, op_value) in operations {
        let action = op_value
            .get("action")
            .and_then(|a| a.as_str())
            .ok_or_else(|| "Operation missing 'action' field".to_string())?;

        let operation = match action {
            "send" => "PUB",
            "receive" => "SUB",
            other => return Err(format!("Unknown operation action: {}", other)),
        };

        // Resolve channel reference
        let channel_ref = op_value
            .get("channel")
            .and_then(|c| c.get("$ref"))
            .and_then(|r| r.as_str());
        let channel_name = if let Some(ref_str) = channel_ref {
            // e.g. "#/channels/UserSignup"
            ref_str
                .strip_prefix("#/channels/")
                .ok_or_else(|| format!("Unsupported channel $ref: {}", ref_str))?
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

        specs.push(create_spec_v3(
            root,
            channel_name,
            channel_address,
            operation,
            op_value,
            channels,
        )?);
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
    if let Some(chs) = channels
        && let Some(ch_value) = chs.get(Value::String(channel_key.to_string()))
    {
        let mut ch_map = Mapping::new();
        ch_map.insert(Value::String(channel_key.to_string()), ch_value.clone());
        snippet.insert(
            Value::String("channels".to_string()),
            Value::Mapping(ch_map),
        );
    }

    // Include only this operation
    let mut ops = Mapping::new();
    // Use a synthetic key based on action + channel
    let op_key = format!("{}_{}", operation.to_lowercase(), channel_key);
    ops.insert(Value::String(op_key), op_value.clone());
    snippet.insert(Value::String("operations".to_string()), Value::Mapping(ops));

    let yaml_content = serde_yaml_ng::to_string(&Value::Mapping(snippet))
        .map_err(|e| format!("Failed to serialize AsyncAPI snippet: {}", e))?;

    Ok(AsyncApiSpec {
        channel: channel_address.to_string(),
        operation: operation.to_string(),
        yaml_content,
        deprecated: is_marked_deprecated(op_value),
    })
}

fn create_spec_v2(
    root: &Value,
    channel_name: &str,
    operation: &str,
    channel_value: &Value,
    op_key: &str,
) -> Result<AsyncApiSpec, String> {
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
    let other_op = if op_key == "publish" {
        "subscribe"
    } else {
        "publish"
    };
    channel_map.remove(Value::String(other_op.to_string()));

    channels.insert(
        Value::String(channel_name.to_string()),
        Value::Mapping(channel_map),
    );
    snippet.insert(
        Value::String("channels".to_string()),
        Value::Mapping(channels),
    );

    let yaml_content = serde_yaml_ng::to_string(&Value::Mapping(snippet))
        .map_err(|e| format!("Failed to serialize AsyncAPI snippet: {}", e))?;

    let deprecated = channel_value
        .get(op_key)
        .map(is_marked_deprecated)
        .unwrap_or(false);

    Ok(AsyncApiSpec {
        channel: channel_name.to_string(),
        operation: operation.to_string(),
        yaml_content,
        deprecated,
    })
}

/// `deprecated: true` or the common `x-deprecated: true` extension on an
/// operation, message, or schema node.
fn is_marked_deprecated(node: &Value) -> bool {
    ["deprecated", "x-deprecated"]
        .iter()
        .any(|key| node.get(key).and_then(Value::as_bool).unwrap_or(false))
}

/// Check if a new AsyncAPI endpoint snippet is backward-compatible with the old one.
/// Breaking changes:
/// - a message was removed from the channel (unless the old message was marked deprecated)
/// - a message payload or payload property changed type (or `$ref` target)
/// - a payload property was removed (unless the old property was marked deprecated)
///
/// Adding messages, adding properties, and metadata/description changes are OK.
/// Enum value changes and `required` changes are not analyzed.
///
/// Returns Ok(()) if compatible, Err(description) if breaking.
pub fn check_backward_compatibility(old_yaml: &str, new_yaml: &str) -> Result<(), String> {
    let old: Value = serde_yaml_ng::from_str(old_yaml)
        .map_err(|e| format!("Failed to parse old AsyncAPI YAML: {}", e))?;
    let new: Value = serde_yaml_ng::from_str(new_yaml)
        .map_err(|e| format!("Failed to parse new AsyncAPI YAML: {}", e))?;

    let old_messages = collect_messages(&old);
    let new_messages = collect_messages(&new);

    for (name, old_message) in &old_messages {
        match new_messages.get(name) {
            None => {
                if !is_marked_deprecated(old_message) {
                    return Err(format!("Message '{}' was removed", name));
                }
            }
            Some(new_message) => match (old_message.get("payload"), new_message.get("payload")) {
                (Some(old_payload), Some(new_payload)) => {
                    check_schema_compatible(
                        &format!("{}.payload", name),
                        old_payload,
                        new_payload,
                    )?;
                }
                (Some(_), None) => {
                    return Err(format!("Payload of message '{}' was removed", name));
                }
                _ => {}
            },
        }
    }

    Ok(())
}

/// Collect all messages of a document keyed by a stable name: the message
/// `name`/`title` when present, otherwise a channel/operation-derived key.
fn collect_messages(root: &Value) -> HashMap<String, &Value> {
    let mut messages = HashMap::new();
    let Some(channels) = root.get("channels").and_then(Value::as_mapping) else {
        return messages;
    };

    for (channel_name, channel_value) in channels {
        let channel_name = channel_name.as_str().unwrap_or_default();

        // AsyncAPI 3.x: channels.<name>.messages.<msgName>
        if let Some(v3_messages) = channel_value.get("messages").and_then(Value::as_mapping) {
            for (message_name, message) in v3_messages {
                let message_name = message_name.as_str().unwrap_or_default();
                messages.insert(format!("{}/{}", channel_name, message_name), message);
            }
        }

        // AsyncAPI 2.x: channels.<name>.publish|subscribe.message (optionally oneOf)
        for op_key in ["publish", "subscribe"] {
            let Some(message) = channel_value.get(op_key).and_then(|op| op.get("message")) else {
                continue;
            };
            if let Some(one_of) = message.get("oneOf").and_then(Value::as_sequence) {
                for (index, variant) in one_of.iter().enumerate() {
                    messages.insert(message_key(channel_name, op_key, variant, index), variant);
                }
            } else {
                messages.insert(message_key(channel_name, op_key, message, 0), message);
            }
        }
    }

    messages
}

fn message_key(channel: &str, op_key: &str, message: &Value, index: usize) -> String {
    let name = message
        .get("name")
        .or_else(|| message.get("title"))
        .and_then(Value::as_str);
    match name {
        Some(name) => format!("{}/{}/{}", channel, op_key, name),
        None => format!("{}/{}/#{}", channel, op_key, index),
    }
}

/// Recursively check that a payload schema change is backward-compatible.
fn check_schema_compatible(path: &str, old: &Value, new: &Value) -> Result<(), String> {
    let old_ref = old.get("$ref").and_then(Value::as_str);
    let new_ref = new.get("$ref").and_then(Value::as_str);
    if let (Some(old_ref), Some(new_ref)) = (old_ref, new_ref)
        && old_ref != new_ref
    {
        return Err(format!(
            "'{}' changed $ref from '{}' to '{}'",
            path, old_ref, new_ref
        ));
    }

    let old_type = old.get("type").and_then(Value::as_str);
    let new_type = new.get("type").and_then(Value::as_str);
    if let (Some(old_type), Some(new_type)) = (old_type, new_type)
        && old_type != new_type
    {
        return Err(format!(
            "'{}' changed type from '{}' to '{}'",
            path, old_type, new_type
        ));
    }

    if let Some(old_props) = old.get("properties").and_then(Value::as_mapping) {
        let new_props = new.get("properties").and_then(Value::as_mapping);
        for (prop_name, old_prop) in old_props {
            let prop_path = format!("{}.{}", path, prop_name.as_str().unwrap_or_default());
            match new_props.and_then(|props| props.get(prop_name)) {
                None => {
                    if !is_marked_deprecated(old_prop) {
                        return Err(format!("Property '{}' was removed", prop_path));
                    }
                }
                Some(new_prop) => check_schema_compatible(&prop_path, old_prop, new_prop)?,
            }
        }
    }

    if let (Some(old_items), Some(new_items)) = (old.get("items"), new.get("items")) {
        check_schema_compatible(&format!("{}[]", path), old_items, new_items)?;
    }

    Ok(())
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

        let user_created_pub = result
            .iter()
            .find(|s| s.channel == "user-created" && s.operation == "PUB")
            .unwrap();
        assert!(user_created_pub.yaml_content.contains("publish:"));
        assert!(!user_created_pub.yaml_content.contains("subscribe:"));
        assert!(user_created_pub.yaml_content.contains("asyncapi:"));
        assert!(user_created_pub.yaml_content.contains("2.6.0"));

        let order_placed_pub = result
            .iter()
            .find(|s| s.channel == "order-placed" && s.operation == "PUB")
            .unwrap();
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

        let user_sub = result
            .iter()
            .find(|s| s.channel == "user/signedup" && s.operation == "SUB")
            .unwrap();
        assert!(user_sub.yaml_content.contains("asyncapi:"));
        assert!(user_sub.yaml_content.contains("3.0.0"));
        assert!(user_sub.yaml_content.contains("UserSignup"));
        assert!(!user_sub.yaml_content.contains("OrderCreated"));

        let order_pub = result
            .iter()
            .find(|s| s.channel == "orders.created" && s.operation == "PUB")
            .unwrap();
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

    #[test]
    fn test_split_detects_deprecated_operations() {
        let v2 = r#"
asyncapi: 2.6.0
info: { title: T, version: 1.0.0 }
channels:
  old-events:
    publish:
      deprecated: true
      message:
        payload:
          type: object
  new-events:
    publish:
      message:
        payload:
          type: object
"#;
        let result = split_asyncapi(v2).unwrap();
        let old = result.iter().find(|s| s.channel == "old-events").unwrap();
        let new = result.iter().find(|s| s.channel == "new-events").unwrap();
        assert!(old.deprecated);
        assert!(!new.deprecated);

        let v3 = r#"
asyncapi: 3.0.0
info: { title: T, version: 1.0.0 }
channels:
  Legacy:
    address: legacy.topic
    messages:
      M:
        payload:
          type: object
operations:
  publishLegacy:
    action: send
    x-deprecated: true
    channel:
      $ref: '#/channels/Legacy'
"#;
        let result = split_asyncapi(v3).unwrap();
        assert_eq!(result.len(), 1);
        assert!(result[0].deprecated);
    }

    const COMPAT_BASE: &str = r#"
asyncapi: 2.6.0
info: { title: T, version: 1.0.0 }
channels:
  user-created:
    publish:
      message:
        name: UserCreated
        payload:
          type: object
          properties:
            id: { type: string }
            age: { type: integer }
"#;

    #[test]
    fn test_compat_identical_and_additive_ok() {
        assert_eq!(
            check_backward_compatibility(COMPAT_BASE, COMPAT_BASE),
            Ok(())
        );

        let added = COMPAT_BASE.replace(
            "age: { type: integer }",
            "age: { type: integer }\n            email: { type: string }",
        );
        assert_eq!(check_backward_compatibility(COMPAT_BASE, &added), Ok(()));
    }

    #[test]
    fn test_compat_removed_property_is_breaking() {
        let removed = COMPAT_BASE.replace("            age: { type: integer }\n", "");
        let err = check_backward_compatibility(COMPAT_BASE, &removed).unwrap_err();
        assert_eq!(
            err,
            "Property 'user-created/publish/UserCreated.payload.age' was removed"
        );
    }

    #[test]
    fn test_compat_removed_deprecated_property_ok() {
        let base = COMPAT_BASE.replace(
            "age: { type: integer }",
            "age: { type: integer, deprecated: true }",
        );
        let removed = base.replace("            age: { type: integer, deprecated: true }\n", "");
        assert_eq!(check_backward_compatibility(&base, &removed), Ok(()));
    }

    #[test]
    fn test_compat_type_change_is_breaking() {
        let changed = COMPAT_BASE.replace("age: { type: integer }", "age: { type: string }");
        let err = check_backward_compatibility(COMPAT_BASE, &changed).unwrap_err();
        assert_eq!(
            err,
            "'user-created/publish/UserCreated.payload.age' changed type from 'integer' to 'string'"
        );
    }

    #[test]
    fn test_compat_removed_message_is_breaking_unless_deprecated() {
        let two_messages = r#"
asyncapi: 3.0.0
info: { title: T, version: 1.0.0 }
channels:
  Events:
    address: events
    messages:
      Kept:
        payload: { type: object }
      Dropped:
        payload: { type: object }
"#;
        let one_message = r#"
asyncapi: 3.0.0
info: { title: T, version: 1.0.0 }
channels:
  Events:
    address: events
    messages:
      Kept:
        payload: { type: object }
"#;
        let err = check_backward_compatibility(two_messages, one_message).unwrap_err();
        assert_eq!(err, "Message 'Events/Dropped' was removed");

        let deprecated = two_messages.replace(
            "      Dropped:\n",
            "      Dropped:\n        deprecated: true\n",
        );
        assert_eq!(
            check_backward_compatibility(&deprecated, one_message),
            Ok(())
        );
    }
}
