use crate::domain::models::Impact;
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

/// A `publish`/`send` (PUB) message extracted from an AsyncAPI document, for
/// message-level channel contracts (ai/improvements.md item #20). Only messages
/// carrying an explicit identity (`name`, falling back to `title`) are returned;
/// unnamed messages have no cross-service identity and are skipped.
#[derive(Debug, Clone, PartialEq)]
pub struct PubMessage {
    /// Topic identity: the channel `address` (3.x) or the channel key (2.x).
    pub channel: String,
    /// Message identity: the message `name`, falling back to `title`.
    pub message_name: String,
    /// The message `payload` schema, re-serialized as YAML.
    pub payload_yaml: String,
    /// Whether the message (or its publishing operation) is marked deprecated.
    pub deprecated: bool,
}

/// Extract every named PUB (publish/send) message from an AsyncAPI 2.x or 3.x
/// document, used to register and validate message-level channel contracts.
///
/// Sanshain reads the AsyncAPI 2.x `publish` keyword from the **application's**
/// perspective (`publish` = this service publishes), matching its 3.x `send`
/// mapping. Note this is the inverse of the official 2.x spec, which defines
/// the keyword from the client's perspective.
pub fn extract_pub_messages(yaml_str: &str) -> Result<Vec<PubMessage>, String> {
    extract_messages(yaml_str, Direction::Pub)
}

/// Extract every named SUB (subscribe/receive) message — the mirror of
/// [`extract_pub_messages`], used to harvest a Producer's declared
/// subscriptions (ai/improvements.md #6, ADR-0006). The returned [`PubMessage`]
/// describes the subscribed message; its fields (channel, message name,
/// payload) are direction-neutral. The same app-perspective reading applies:
/// `subscribe` (2.x) / `receive` (3.x) = this service subscribes.
pub fn extract_sub_messages(yaml_str: &str) -> Result<Vec<PubMessage>, String> {
    extract_messages(yaml_str, Direction::Sub)
}

/// Which side of a channel operation to read, from the application's
/// perspective: `Pub` = what this service publishes (2.x `publish` / 3.x
/// `send`), `Sub` = what it subscribes to (2.x `subscribe` / 3.x `receive`).
#[derive(Clone, Copy)]
enum Direction {
    Pub,
    Sub,
}

impl Direction {
    fn v2_keyword(self) -> &'static str {
        match self {
            Direction::Pub => "publish",
            Direction::Sub => "subscribe",
        }
    }
    fn v3_action(self) -> &'static str {
        match self {
            Direction::Pub => "send",
            Direction::Sub => "receive",
        }
    }
}

fn extract_messages(yaml_str: &str, dir: Direction) -> Result<Vec<PubMessage>, String> {
    let root: Value = serde_yaml_ng::from_str(yaml_str)
        .map_err(|e| format!("Failed to parse AsyncAPI YAML: {}", e))?;
    let version = root.get("asyncapi").and_then(Value::as_str).unwrap_or("");
    if version.starts_with("3.") {
        Ok(extract_messages_v3(&root, dir))
    } else {
        Ok(extract_messages_v2(&root, dir))
    }
}

fn extract_messages_v2(root: &Value, dir: Direction) -> Vec<PubMessage> {
    let mut out = Vec::new();
    let Some(channels) = root.get("channels").and_then(Value::as_mapping) else {
        return out;
    };
    for (channel_name, channel_value) in channels {
        let channel = channel_name.as_str().unwrap_or_default();
        let Some(operation) = channel_value.get(dir.v2_keyword()) else {
            continue;
        };
        let op_deprecated = is_marked_deprecated(operation);
        let Some(message) = operation.get("message") else {
            continue;
        };
        let variants: Vec<&Value> = match message.get("oneOf").and_then(Value::as_sequence) {
            Some(one_of) => one_of.iter().collect(),
            None => vec![message],
        };
        for variant in variants {
            if let Some(pm) = pub_message_from_node(channel, variant, op_deprecated) {
                out.push(pm);
            }
        }
    }
    out
}

fn extract_messages_v3(root: &Value, dir: Direction) -> Vec<PubMessage> {
    let mut out = Vec::new();
    let Some(operations) = root.get("operations").and_then(Value::as_mapping) else {
        return out;
    };
    let channels = root.get("channels");
    for (_op_name, op) in operations {
        if op.get("action").and_then(Value::as_str) != Some(dir.v3_action()) {
            continue;
        }
        let op_deprecated = is_marked_deprecated(op);
        let Some(channel_ref) = op
            .get("channel")
            .and_then(|c| c.get("$ref"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(channel_key) = channel_ref.strip_prefix("#/channels/") else {
            continue;
        };
        let channel_node = channels.and_then(|chs| chs.get(channel_key));
        let channel_address = channel_node
            .and_then(|ch| ch.get("address"))
            .and_then(Value::as_str)
            .unwrap_or(channel_key);

        // Messages the operation publishes: explicit `messages` refs when
        // present, otherwise all messages declared on the referenced channel.
        if let Some(op_messages) = op.get("messages").and_then(Value::as_sequence) {
            for m_ref in op_messages {
                let Some(ref_str) = m_ref.get("$ref").and_then(Value::as_str) else {
                    continue;
                };
                if let Some(node) = resolve_json_pointer(root, ref_str)
                    && let Some(pm) = pub_message_from_node(channel_address, node, op_deprecated)
                {
                    out.push(pm);
                }
            }
        } else if let Some(msgs) = channel_node
            .and_then(|ch| ch.get("messages"))
            .and_then(Value::as_mapping)
        {
            for (_k, node) in msgs {
                if let Some(pm) = pub_message_from_node(channel_address, node, op_deprecated) {
                    out.push(pm);
                }
            }
        }
    }
    out
}

fn pub_message_from_node(
    channel: &str,
    message: &Value,
    op_deprecated: bool,
) -> Option<PubMessage> {
    let name = message
        .get("name")
        .or_else(|| message.get("title"))
        .and_then(Value::as_str)?;
    let payload = message.get("payload").cloned().unwrap_or(Value::Null);
    let payload_yaml = serde_yaml_ng::to_string(&payload).ok()?;
    Some(PubMessage {
        channel: channel.to_string(),
        message_name: name.to_string(),
        payload_yaml,
        deprecated: op_deprecated || is_marked_deprecated(message),
    })
}

/// Resolve a local JSON pointer (`#/a/b/c`) against a document.
fn resolve_json_pointer<'a>(root: &'a Value, pointer: &str) -> Option<&'a Value> {
    let path = pointer.strip_prefix("#/")?;
    let mut node = root;
    for raw in path.split('/') {
        let key = raw.replace("~1", "/").replace("~0", "~");
        node = node.get(key.as_str())?;
    }
    Some(node)
}

/// Check that a change to a single message `payload` schema is
/// backward-compatible: removed non-deprecated properties and type/`$ref`
/// changes are breaking, additions are fine (same rules as
/// [`check_backward_compatibility`], applied at the payload root).
pub fn check_payload_compatible(
    old_payload_yaml: &str,
    new_payload_yaml: &str,
) -> Result<(), String> {
    let old: Value = serde_yaml_ng::from_str(old_payload_yaml)
        .map_err(|e| format!("Failed to parse stored payload: {}", e))?;
    let new: Value = serde_yaml_ng::from_str(new_payload_yaml)
        .map_err(|e| format!("Failed to parse submitted payload: {}", e))?;
    check_schema_compatible("payload", &old, &new)
}

/// Check a consumer's SUB payload (its *expectation*) is satisfiable by a
/// Producer's PUB `contract` payload (ai/improvements.md #6, ADR-0006): every
/// property the expectation reads must exist in the contract with a compatible
/// type. A consumer expecting *less* than the contract guarantees is fine;
/// expecting a property, or a type, the contract does not guarantee is drift.
///
/// This is the role-swapped sibling of [`check_payload_compatible`]. The
/// contract plays the "new" schema (it must still provide everything) and the
/// expectation plays the "old" (what must remain available), so a property the
/// expectation reads but the contract lacks is reported as missing.
pub fn check_expectation_satisfied(
    contract_payload_yaml: &str,
    expectation_payload_yaml: &str,
) -> Result<(), String> {
    let contract: Value = serde_yaml_ng::from_str(contract_payload_yaml)
        .map_err(|e| format!("Failed to parse contract payload: {}", e))?;
    let expectation: Value = serde_yaml_ng::from_str(expectation_payload_yaml)
        .map_err(|e| format!("Failed to parse expectation payload: {}", e))?;
    check_schema_compatible("payload", &expectation, &contract)
}

/// Two message payloads are semantically equal if their parsed YAML values are
/// equal (ignoring formatting and key-order differences).
pub fn payloads_equal(a_yaml: &str, b_yaml: &str) -> bool {
    match (
        serde_yaml_ng::from_str::<Value>(a_yaml),
        serde_yaml_ng::from_str::<Value>(b_yaml),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Classify the SemVer impact of an AsyncAPI change between two endpoint
/// snippets: breaking → `Major`, additive (new messages/properties) → `Minor`,
/// other textual change → `Patch`, identical → `None`.
pub fn analyze_impact(old_yaml: &str, new_yaml: &str) -> Impact {
    if check_backward_compatibility(old_yaml, new_yaml).is_err() {
        return Impact::Major;
    }
    // Role-swap: if treating the new document as the "old" one flags a removal,
    // then the real new document added a message/property -> a minor change.
    if check_backward_compatibility(new_yaml, old_yaml).is_err() {
        return Impact::Minor;
    }
    if old_yaml != new_yaml {
        return Impact::Patch;
    }
    Impact::None
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

    // --- item #20: PUB message extraction and contract helpers ---

    #[test]
    fn extract_pub_messages_v2_named_publish_only() {
        let yaml = r#"
asyncapi: 2.6.0
info: { title: T, version: 1.0.0 }
channels:
  orders:
    publish:
      message:
        name: OrderPlaced
        payload: { type: object }
    subscribe:
      message:
        name: OrderShipped
        payload: { type: object }
"#;
        let msgs = extract_pub_messages(yaml).unwrap();
        // Only the publish message is harvested; subscribe is ignored.
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].channel, "orders");
        assert_eq!(msgs[0].message_name, "OrderPlaced");
        assert!(!msgs[0].deprecated);
    }

    #[test]
    fn extract_pub_messages_v2_oneof_and_title_fallback() {
        let yaml = r#"
asyncapi: 2.6.0
info: { title: T, version: 1.0.0 }
channels:
  events:
    publish:
      message:
        oneOf:
          - name: Created
            payload: { type: object }
          - title: Updated
            payload: { type: object }
          - payload: { type: object }
"#;
        let mut msgs = extract_pub_messages(yaml).unwrap();
        msgs.sort_by(|a, b| a.message_name.cmp(&b.message_name));
        // The unnamed (no name/title) variant is skipped.
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].message_name, "Created");
        assert_eq!(msgs[1].message_name, "Updated");
    }

    #[test]
    fn extract_pub_messages_v3_send_only_with_address() {
        let yaml = r#"
asyncapi: 3.0.0
info: { title: T, version: 1.0.0 }
channels:
  OrderCreated:
    address: orders.created
    messages:
      OrderMessage:
        name: OrderPlaced
        payload: { type: object }
  UserSignup:
    address: user/signedup
    messages:
      UserMessage:
        name: UserSignedUp
        payload: { type: object }
operations:
  publishOrder:
    action: send
    channel: { $ref: '#/channels/OrderCreated' }
  consumeUser:
    action: receive
    channel: { $ref: '#/channels/UserSignup' }
"#;
        let msgs = extract_pub_messages(yaml).unwrap();
        // Only the `send` operation's message, keyed by channel address.
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].channel, "orders.created");
        assert_eq!(msgs[0].message_name, "OrderPlaced");
    }

    #[test]
    fn extract_pub_messages_skips_unnamed() {
        let yaml = r#"
asyncapi: 2.6.0
info: { title: T, version: 1.0.0 }
channels:
  orders:
    publish:
      message:
        payload: { type: object }
"#;
        assert!(extract_pub_messages(yaml).unwrap().is_empty());
    }

    #[test]
    fn extract_pub_messages_marks_deprecated() {
        let yaml = r#"
asyncapi: 2.6.0
info: { title: T, version: 1.0.0 }
channels:
  orders:
    publish:
      deprecated: true
      message:
        name: OrderPlaced
        payload: { type: object }
"#;
        let msgs = extract_pub_messages(yaml).unwrap();
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].deprecated);
    }

    #[test]
    fn check_payload_compatible_accepts_additions_rejects_removals() {
        let old = "type: object\nproperties:\n  id: { type: string }\n";
        let widened =
            "type: object\nproperties:\n  id: { type: string }\n  name: { type: string }\n";
        assert_eq!(check_payload_compatible(old, widened), Ok(()));

        let removed = "type: object\nproperties:\n  other: { type: string }\n";
        assert!(check_payload_compatible(old, removed).is_err());

        let retyped = "type: object\nproperties:\n  id: { type: integer }\n";
        assert!(check_payload_compatible(old, retyped).is_err());

        // Removing a property that was marked deprecated is allowed.
        let old_dep = "type: object\nproperties:\n  id: { type: string }\n  legacy: { type: string, deprecated: true }\n";
        assert_eq!(check_payload_compatible(old_dep, old), Ok(()));
    }

    #[test]
    fn payloads_equal_ignores_formatting() {
        let a = "type: object\nproperties:\n  id: { type: string }\n";
        let b = "properties:\n  id:\n    type: string\ntype: object\n";
        assert!(payloads_equal(a, b));

        let c = "type: object\nproperties:\n  id: { type: integer }\n";
        assert!(!payloads_equal(a, c));
    }

    #[test]
    fn analyze_impact_classifies_changes() {
        let base = r#"
asyncapi: 2.6.0
info: { title: T, version: 1.0.0 }
channels:
  orders:
    publish:
      message:
        name: OrderPlaced
        payload:
          type: object
          properties:
            id: { type: string }
"#;
        assert_eq!(analyze_impact(base, base), Impact::None);

        let added = base.replace(
            "            id: { type: string }\n",
            "            id: { type: string }\n            name: { type: string }\n",
        );
        assert_eq!(analyze_impact(base, &added), Impact::Minor);

        let removed = base.replace("            id: { type: string }\n", "");
        assert_eq!(analyze_impact(base, &removed), Impact::Major);

        let doc_only = base.replace("title: T", "title: Titled");
        assert_eq!(analyze_impact(base, &doc_only), Impact::Patch);
    }

    // --- SUB harvesting (ai/improvements.md #6, ADR-0006) ---

    #[test]
    fn extract_sub_messages_v2_named_subscribe_only() {
        // Same fixture as the PUB test: harvesting SUB must pick the mirror
        // operation — the subscribe message, never the publish one.
        let yaml = r#"
asyncapi: 2.6.0
info: { title: T, version: 1.0.0 }
channels:
  orders:
    publish:
      message:
        name: OrderPlaced
        payload: { type: object }
    subscribe:
      message:
        name: OrderShipped
        payload: { type: object }
"#;
        let msgs = extract_sub_messages(yaml).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].channel, "orders");
        assert_eq!(msgs[0].message_name, "OrderShipped");
    }

    #[test]
    fn extract_sub_messages_empty_when_only_publish() {
        let yaml = r#"
asyncapi: 2.6.0
info: { title: T, version: 1.0.0 }
channels:
  orders:
    publish:
      message:
        name: OrderPlaced
        payload: { type: object }
"#;
        assert!(extract_sub_messages(yaml).unwrap().is_empty());
    }

    #[test]
    fn extract_sub_messages_v3_receive_only_with_address() {
        let yaml = r#"
asyncapi: 3.0.0
info: { title: T, version: 1.0.0 }
channels:
  OrderShipped:
    address: orders.shipped
    messages:
      ShipMessage:
        name: OrderShipped
        payload: { type: object }
operations:
  publishOrder:
    action: send
    channel: { $ref: '#/channels/OrderShipped' }
  consumeShipped:
    action: receive
    channel: { $ref: '#/channels/OrderShipped' }
"#;
        let msgs = extract_sub_messages(yaml).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].channel, "orders.shipped");
        assert_eq!(msgs[0].message_name, "OrderShipped");
    }

    #[test]
    fn expectation_reading_a_subset_is_satisfied() {
        let contract =
            "type: object\nproperties:\n  id: { type: string }\n  name: { type: string }\n";
        let expectation = "type: object\nproperties:\n  id: { type: string }\n";
        assert!(check_expectation_satisfied(contract, expectation).is_ok());
    }

    #[test]
    fn expectation_equal_to_contract_is_satisfied() {
        let schema = "type: object\nproperties:\n  id: { type: string }\n";
        assert!(check_expectation_satisfied(schema, schema).is_ok());
    }

    #[test]
    fn expectation_of_a_property_the_contract_lacks_is_drift() {
        let contract = "type: object\nproperties:\n  id: { type: string }\n";
        let expectation =
            "type: object\nproperties:\n  id: { type: string }\n  ssn: { type: string }\n";
        let err = check_expectation_satisfied(contract, expectation).unwrap_err();
        assert!(
            err.contains("ssn"),
            "error should name the missing property: {err}"
        );
    }

    #[test]
    fn expectation_of_an_incompatible_type_is_drift() {
        let contract = "type: object\nproperties:\n  id: { type: string }\n";
        let expectation = "type: object\nproperties:\n  id: { type: integer }\n";
        assert!(check_expectation_satisfied(contract, expectation).is_err());
    }

    #[test]
    fn expectation_drift_is_detected_in_a_nested_property() {
        let contract = "type: object\nproperties:\n  meta:\n    type: object\n    properties:\n      a: { type: string }\n";
        let expectation = "type: object\nproperties:\n  meta:\n    type: object\n    properties:\n      b: { type: string }\n";
        let err = check_expectation_satisfied(contract, expectation).unwrap_err();
        assert!(
            err.contains('b'),
            "error should name the nested missing property: {err}"
        );
    }
}
