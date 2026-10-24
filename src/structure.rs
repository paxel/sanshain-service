//! The structure view of an endpoint: its parameters, bodies, responses and
//! messages as a tree of named nodes, every `$ref` inlined, `allOf` merged and
//! `oneOf`/`anyOf` kept as variants, so a reader sees the contract without the
//! YAML around it. OpenAPI and AsyncAPI (2.x and 3.x) endpoint snippets, as the
//! split stores them; proto stays text.
//!
//! A node's `key` is its identity among its siblings, the same across versions,
//! so the path of keys from the endpoint down names "the same node" in every
//! version of a line. [`annotate`] uses that to tell, per node, the version that
//! last changed the node's own fields.

use crate::domain::models::{ApiType, SemVer};
use crate::yaml::Value;
use serde::Serialize;
use serde_json::Value as Json;
use std::collections::{BTreeSet, HashMap};

/// Nesting deeper than this is cut with a note; real DTOs never get there.
const MAX_DEPTH: usize = 48;
/// A `$ref` chain longer than this is treated as unresolvable.
const MAX_REF_HOPS: usize = 32;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Endpoint,
    #[default]
    Group,
    Parameter,
    Header,
    Body,
    Response,
    Message,
    Member,
    Items,
    Variants,
    Variant,
    /// A `$ref` back to a schema already open above: the tree stops here.
    Cycle,
    /// Something the tree cannot show (an external `$ref`, an Avro payload).
    Note,
}

/// Which side of the exchange a body is on: a request example leaves out
/// `readOnly` members, a response example `writeOnly` ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Request,
    Response,
    Message,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Node {
    pub key: String,
    pub kind: NodeKind,
    pub name: String,
    /// The endpoint's method and path as stored, on endpoint nodes only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// `string (email)`, `Address`, `array of Tag`.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub required: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub deprecated: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub nullable: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub read_only: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub write_only: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Json>,
    /// The spec's example; on a body without one, filled by [`fill_examples`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example: Option<Json>,
    /// True when `example` was built from the schema, not written in the spec.
    #[serde(skip_serializing_if = "is_false")]
    pub example_generated: bool,
    #[serde(rename = "enum", skip_serializing_if = "Vec::is_empty")]
    pub enum_values: Vec<Json>,
    /// `min 1`, `max length 64`, `pattern ^[a-z]+$`, …
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<String>,
    /// The `allOf` part a merged member came from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// The discriminator property of a `oneOf`/`anyOf`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discriminator: Option<String>,
    /// The version that last changed this node's own fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed_in: Option<SemVer>,
    /// The newest change below this node, when newer than `changed_in`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inside: Option<SemVer>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
    /// The children were left out of this answer; ask for the endpoint alone.
    #[serde(skip_serializing_if = "is_false")]
    pub children_omitted: bool,
    /// The JSON type behind `type_name`, for building examples.
    #[serde(skip)]
    json_type: Option<String>,
    #[serde(skip)]
    format: Option<String>,
    #[serde(skip)]
    direction: Option<Direction>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Node {
    fn new(kind: NodeKind, key: impl Into<String>, name: impl Into<String>) -> Self {
        Node {
            key: key.into(),
            kind,
            name: name.into(),
            ..Default::default()
        }
    }
}

/// The endpoint node of one stored endpoint snippet. `method` and
/// `normalized_path` are the stored identity (`GET` + `/orders/{}`, `PUB` +
/// a channel), which keys the node across versions; `path` is shown.
pub fn endpoint_node(
    api_type: ApiType,
    method: &str,
    path: &str,
    normalized_path: &str,
    snippet: &str,
) -> Result<Node, String> {
    let root: Value =
        crate::yaml::from_str(snippet).map_err(|e| format!("unreadable snippet: {e}"))?;
    let walker = Walker { root: &root };
    let mut node = Node::new(
        NodeKind::Endpoint,
        format!("{method} {normalized_path}"),
        format!("{method} {path}"),
    );
    node.method = Some(method.to_string());
    node.path = Some(path.to_string());
    node.summary = crate::descriptions::endpoint_summary(api_type, snippet);
    match api_type {
        ApiType::OpenApi => walker.openapi_operation(&mut node, method)?,
        ApiType::AsyncApi => walker.asyncapi_operation(&mut node, method)?,
        ApiType::Proto => {
            return Err("the structure view covers OpenAPI and AsyncAPI".to_string());
        }
    }
    Ok(node)
}

/// The endpoint node standing in for a snippet the tree cannot be built from,
/// with the reason as its description.
pub fn unreadable_endpoint(
    method: &str,
    path: &str,
    normalized_path: &str,
    reason: String,
) -> Node {
    let mut node = Node::new(
        NodeKind::Endpoint,
        format!("{method} {normalized_path}"),
        format!("{method} {path}"),
    );
    node.method = Some(method.to_string());
    node.path = Some(path.to_string());
    node.children
        .push(Walker::note("unreadable", "Not shown", reason));
    node
}

struct Walker<'a> {
    root: &'a Value,
}

/// The members an object schema and its `allOf` parts declare, in order,
/// with the part each came from; cycles and unresolvable parts as nodes.
#[derive(Default)]
struct Collected<'a> {
    props: Vec<(String, &'a Value, Option<String>)>,
    required: BTreeSet<String>,
    extra: Vec<Node>,
}

/// A value with its `$ref`s followed: the target and the last schema name.
struct Resolved<'a> {
    value: &'a Value,
    name: Option<String>,
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn bool_of(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn json_of(v: &Value) -> Option<Json> {
    serde_json::to_value(v).ok()
}

fn text_of(v: &Value, key: &str) -> Option<String> {
    str_of(v, key)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn last_segment(pointer: &str) -> String {
    pointer
        .rsplit('/')
        .next()
        .unwrap_or(pointer)
        .replace("~1", "/")
        .replace("~0", "~")
}

impl<'a> Walker<'a> {
    /// The first example `v` carries: `example`, else the first of
    /// `examples`, an OpenAPI map of `{value}` objects (each possibly a `$ref`
    /// to `#/components/examples`) or a JSON Schema list. A message's examples
    /// are `{payload, headers}` objects, of which the payload counts.
    fn first_example(&self, v: &'a Value, message: bool) -> Option<Json> {
        if let Some(example) = v.get("example") {
            return json_of(example);
        }
        let first = match v.get("examples")? {
            Value::Array(list) => list.first()?,
            Value::Object(map) => {
                let entry = map.values().next()?;
                let entry = self.resolve(entry).ok()?.value;
                entry.get("value").unwrap_or(entry)
            }
            _ => return None,
        };
        let first = if message {
            first.get("payload").unwrap_or(first)
        } else {
            first
        };
        json_of(first)
    }

    fn pointer(&self, pointer: &str) -> Option<&'a Value> {
        let path = pointer.strip_prefix("#/")?;
        let mut node = self.root;
        for raw in path.split('/') {
            let key = raw.replace("~1", "/").replace("~0", "~");
            node = node.get(key.as_str())?;
        }
        Some(node)
    }

    /// Follow `$ref`s to their target. An external or dangling ref is an
    /// error naming it.
    fn resolve(&self, value: &'a Value) -> Result<Resolved<'a>, String> {
        let mut current = value;
        let mut name = None;
        for _ in 0..MAX_REF_HOPS {
            let Some(reference) = str_of(current, "$ref") else {
                return Ok(Resolved {
                    value: current,
                    name,
                });
            };
            let target = self
                .pointer(reference)
                .ok_or_else(|| reference.to_string())?;
            name = Some(last_segment(reference));
            current = target;
        }
        Err(str_of(current, "$ref").unwrap_or("$ref").to_string())
    }

    fn note(key: &str, name: &str, text: String) -> Node {
        let mut node = Node::new(NodeKind::Note, key, name);
        node.description = Some(text);
        node
    }

    // --- OpenAPI ---------------------------------------------------------

    fn openapi_operation(&self, node: &mut Node, method: &str) -> Result<(), String> {
        let verb = method.to_lowercase();
        let paths = self
            .root
            .get("paths")
            .and_then(Value::as_object)
            .ok_or("the snippet has no paths")?;
        let (item, operation) = paths
            .values()
            .filter_map(|item| self.resolve(item).ok().map(|r| r.value))
            .find_map(|item| item.get(verb.as_str()).map(|op| (item, op)))
            .ok_or_else(|| format!("the snippet has no {method} operation"))?;
        node.description = text_of(operation, "description");
        node.deprecated = bool_of(operation, "deprecated");

        let parameters = self.openapi_parameters(item, operation);
        if !parameters.is_empty() {
            let mut group = Node::new(NodeKind::Group, "parameters", "Parameters");
            for location in ["path", "query", "header", "cookie"] {
                let of_location: Vec<Node> = parameters
                    .iter()
                    .filter(|(loc, _)| loc == location)
                    .map(|(_, p)| p.clone())
                    .collect();
                if of_location.is_empty() {
                    continue;
                }
                let mut by_location = Node::new(NodeKind::Group, location, location);
                by_location.children = of_location;
                group.children.push(by_location);
            }
            node.children.push(group);
        }

        if let Some(body) = operation.get("requestBody") {
            node.children.push(self.openapi_request_body(body));
        }
        if let Some(responses) = operation.get("responses").and_then(Value::as_object) {
            let mut group = Node::new(NodeKind::Group, "responses", "Responses");
            for (status, response) in responses {
                group.children.push(self.openapi_response(status, response));
            }
            node.children.push(group);
        }
        Ok(())
    }

    /// Path-level parameters, overridden by operation-level ones of the same
    /// name and location, in declaration order.
    fn openapi_parameters(&self, item: &'a Value, operation: &'a Value) -> Vec<(String, Node)> {
        let mut out: Vec<(String, Node)> = Vec::new();
        let lists = [item.get("parameters"), operation.get("parameters")];
        for list in lists.into_iter().flatten().filter_map(Value::as_array) {
            for raw in list {
                let (location, param) = match self.resolve(raw) {
                    Ok(r) => (
                        str_of(r.value, "in").unwrap_or("query").to_string(),
                        self.openapi_parameter(r.value, NodeKind::Parameter),
                    ),
                    Err(reference) => (
                        "query".to_string(),
                        Self::note(
                            &reference,
                            &reference,
                            format!("{reference} is outside this document"),
                        ),
                    ),
                };
                if let Some(slot) = out
                    .iter_mut()
                    .find(|(loc, p)| *loc == location && p.key == param.key)
                {
                    slot.1 = param;
                } else {
                    out.push((location, param));
                }
            }
        }
        out
    }

    /// A parameter or header object: the schema's node under the parameter's
    /// own name, description, required and example.
    fn openapi_parameter(&self, param: &'a Value, kind: NodeKind) -> Node {
        let name = str_of(param, "name").unwrap_or("").to_string();
        self.openapi_parameter_named(param, kind, &name)
    }

    fn openapi_parameter_named(&self, param: &'a Value, kind: NodeKind, name: &str) -> Node {
        let schema = param.get("schema").or_else(|| {
            param
                .get("content")
                .and_then(Value::as_object)
                .and_then(|c| c.values().next())
                .and_then(|m| m.get("schema"))
        });
        let mut node = match schema {
            Some(schema) => self.schema_node(name, name, schema, &mut Vec::new(), 0),
            None => Node::new(kind, name, name),
        };
        node.kind = kind;
        if let Some(description) = text_of(param, "description") {
            node.description = Some(description);
        }
        node.required = bool_of(param, "required");
        node.deprecated |= bool_of(param, "deprecated");
        if let Some(example) = self.first_example(param, false) {
            node.example = Some(example);
        }
        node
    }

    fn openapi_request_body(&self, raw: &'a Value) -> Node {
        let mut group = Node::new(NodeKind::Group, "request", "Request body");
        let body = match self.resolve(raw) {
            Ok(r) => r.value,
            Err(reference) => {
                group.children.push(Self::note(
                    "ref",
                    &reference,
                    format!("{reference} is outside this document"),
                ));
                return group;
            }
        };
        group.description = text_of(body, "description");
        group.required = bool_of(body, "required");
        group.children = self.openapi_content(body, Direction::Request);
        group
    }

    fn openapi_response(&self, status: &str, raw: &'a Value) -> Node {
        let mut node = Node::new(NodeKind::Response, status, status);
        let response = match self.resolve(raw) {
            Ok(r) => r.value,
            Err(reference) => {
                node.children.push(Self::note(
                    "ref",
                    &reference,
                    format!("{reference} is outside this document"),
                ));
                return node;
            }
        };
        node.description = text_of(response, "description");
        if let Some(headers) = response.get("headers").and_then(Value::as_object) {
            let mut group = Node::new(NodeKind::Group, "headers", "Headers");
            for (name, header) in headers {
                let name = name.as_str();
                let header_node = match self.resolve(header) {
                    Ok(r) => self.openapi_parameter_named(r.value, NodeKind::Header, name),
                    Err(reference) => {
                        Self::note(name, name, format!("{reference} is outside this document"))
                    }
                };
                group.children.push(header_node);
            }
            node.children.push(group);
        }
        node.children
            .extend(self.openapi_content(response, Direction::Response));
        node
    }

    /// One body node per media type of a `content` map.
    fn openapi_content(&self, owner: &'a Value, direction: Direction) -> Vec<Node> {
        let Some(content) = owner.get("content").and_then(Value::as_object) else {
            return Vec::new();
        };
        content
            .iter()
            .map(|(media, entry)| {
                let media = media.as_str();
                let mut body = match entry.get("schema") {
                    Some(schema) => self.schema_node(media, media, schema, &mut Vec::new(), 0),
                    None => Node::new(NodeKind::Body, media, media),
                };
                body.kind = NodeKind::Body;
                body.direction = Some(direction);
                if let Some(example) = self.first_example(entry, false) {
                    body.example = Some(example);
                }
                body
            })
            .collect()
    }

    // --- AsyncAPI --------------------------------------------------------

    fn asyncapi_operation(&self, node: &mut Node, method: &str) -> Result<(), String> {
        let v3 = str_of(self.root, "asyncapi").is_some_and(|v| v.starts_with("3."));
        if v3 {
            self.asyncapi_v3(node)
        } else {
            self.asyncapi_v2(node, method)
        }
    }

    fn asyncapi_v2(&self, node: &mut Node, method: &str) -> Result<(), String> {
        let op_key = if method.eq_ignore_ascii_case("SUB") {
            "subscribe"
        } else {
            "publish"
        };
        let channel = self
            .root
            .get("channels")
            .and_then(Value::as_object)
            .and_then(|c| c.values().next())
            .ok_or("the snippet has no channel")?;
        let channel = self.resolve(channel).map(|r| r.value).unwrap_or(channel);
        let operation = channel
            .get(op_key)
            .ok_or_else(|| format!("the channel has no {op_key} operation"))?;
        node.description =
            text_of(operation, "description").or_else(|| text_of(channel, "description"));
        node.deprecated = bool_of(operation, "deprecated");
        self.asyncapi_channel_parameters(node, channel);
        if let Some(message) = operation.get("message") {
            let message = self.resolve(message).map(|r| r.value).unwrap_or(message);
            match message.get("oneOf").and_then(Value::as_array) {
                Some(variants) => {
                    for (index, variant) in variants.iter().enumerate() {
                        node.children
                            .push(self.asyncapi_message(&index.to_string(), variant));
                    }
                }
                None => node
                    .children
                    .push(self.asyncapi_message("0", operation.get("message").unwrap_or(message))),
            }
        }
        Ok(())
    }

    fn asyncapi_v3(&self, node: &mut Node) -> Result<(), String> {
        let operation = self
            .root
            .get("operations")
            .and_then(Value::as_object)
            .and_then(|o| o.values().next())
            .ok_or("the snippet has no operation")?;
        let channel = operation
            .get("channel")
            .and_then(|c| self.resolve(c).ok())
            .map(|r| r.value);
        node.description = text_of(operation, "description")
            .or_else(|| channel.and_then(|c| text_of(c, "description")));
        node.deprecated = bool_of(operation, "deprecated");
        if let Some(channel) = channel {
            self.asyncapi_channel_parameters(node, channel);
        }
        let listed = operation.get("messages").and_then(Value::as_array);
        match listed {
            Some(messages) if !messages.is_empty() => {
                for (index, message) in messages.iter().enumerate() {
                    node.children
                        .push(self.asyncapi_message(&index.to_string(), message));
                }
            }
            _ => {
                let messages = channel
                    .and_then(|c| c.get("messages"))
                    .and_then(Value::as_object);
                for (key, message) in messages.into_iter().flatten() {
                    let key = key.as_str();
                    node.children.push(self.asyncapi_message(key, message));
                }
            }
        }
        Ok(())
    }

    fn asyncapi_channel_parameters(&self, node: &mut Node, channel: &'a Value) {
        let Some(parameters) = channel.get("parameters").and_then(Value::as_object) else {
            return;
        };
        let mut group = Node::new(NodeKind::Group, "parameters", "Parameters");
        for (name, raw) in parameters {
            let name = name.as_str();
            let param = self.resolve(raw).map(|r| r.value).unwrap_or(raw);
            // 2.x parameters carry a schema; 3.x ones carry enum/default inline.
            let mut child = match param.get("schema") {
                Some(schema) => self.schema_node(name, name, schema, &mut Vec::new(), 0),
                None => self.schema_node(name, name, param, &mut Vec::new(), 0),
            };
            child.kind = NodeKind::Parameter;
            if let Some(description) = text_of(param, "description") {
                child.description = Some(description);
            }
            child.required = true;
            group.children.push(child);
        }
        node.children.push(group);
    }

    /// A message: its name, description, and its headers and payload bodies.
    fn asyncapi_message(&self, fallback_key: &str, raw: &'a Value) -> Node {
        let (message, ref_name) = match self.resolve(raw) {
            Ok(r) => (r.value, r.name),
            Err(reference) => {
                return Self::note(
                    fallback_key,
                    &reference,
                    format!("{reference} is outside this document"),
                );
            }
        };
        let key = ref_name
            .clone()
            .or_else(|| text_of(message, "name"))
            .or_else(|| text_of(message, "messageId"))
            .or_else(|| text_of(message, "title"))
            .unwrap_or_else(|| fallback_key.to_string());
        let mut node = Node::new(NodeKind::Message, key.clone(), key);
        node.summary = text_of(message, "summary").or_else(|| text_of(message, "title"));
        node.description = text_of(message, "description");
        node.deprecated = bool_of(message, "deprecated");
        let format = str_of(message, "schemaFormat").map(str::to_string);
        if let Some(headers) = message.get("headers") {
            node.children.push(self.payload_node(
                "headers",
                "Headers",
                headers,
                format.as_deref(),
                None,
            ));
        }
        if let Some(payload) = message.get("payload") {
            let name = match str_of(message, "contentType") {
                Some(content_type) => format!("Payload ({content_type})"),
                None => "Payload".to_string(),
            };
            let mut body = self.payload_node(
                "payload",
                &name,
                payload,
                format.as_deref(),
                self.first_example(message, true),
            );
            body.direction = Some(Direction::Message);
            node.children.push(body);
        }
        node
    }

    /// A message payload or headers schema. A 3.x multi-format schema
    /// (`{schemaFormat, schema}`) or a 2.x `schemaFormat` that is not JSON
    /// Schema (Avro, Protobuf, RAML, XSD) becomes a note.
    fn payload_node(
        &self,
        key: &str,
        name: &str,
        raw: &'a Value,
        message_format: Option<&str>,
        example: Option<Json>,
    ) -> Node {
        let (schema, format) = match (raw.get("schemaFormat"), raw.get("schema")) {
            (Some(format), Some(schema)) => (schema, format.as_str()),
            _ => (raw, message_format),
        };
        if let Some(format) = format {
            let lower = format.to_lowercase();
            if ["avro", "protobuf", "raml", "xsd", "xml"]
                .iter()
                .any(|f| lower.contains(f))
            {
                return Self::note(
                    key,
                    name,
                    format!("A {format} schema; the YAML view shows it."),
                );
            }
        }
        let mut node = self.schema_node(key, name, schema, &mut Vec::new(), 0);
        node.kind = NodeKind::Body;
        if node.example.is_none() {
            node.example = example;
        }
        node
    }

    // --- JSON Schema -----------------------------------------------------

    /// The node of a schema under `name`: type, description, flags, values,
    /// constraints, and its members, items or variants as children. `stack`
    /// holds the schema names open above, to stop a recursive `$ref`.
    fn schema_node(
        &self,
        key: &str,
        name: &str,
        raw: &'a Value,
        stack: &mut Vec<String>,
        depth: usize,
    ) -> Node {
        if depth > MAX_DEPTH {
            return Self::note(key, name, "Nested too deep to show.".to_string());
        }
        let resolved = match self.resolve(raw) {
            Ok(r) => r,
            Err(reference) => {
                let mut node = Self::note(
                    key,
                    name,
                    format!(
                        "{reference} is outside this document; the YAML view shows the reference."
                    ),
                );
                node.kind = NodeKind::Member;
                node.type_name = Some(reference);
                return node;
            }
        };
        let schema = resolved.value;
        if let Some(ref_name) = &resolved.name
            && stack.contains(ref_name)
        {
            let mut node = Node::new(NodeKind::Cycle, key, name);
            node.type_name = Some(ref_name.clone());
            node.description = Some(format!("↻ {ref_name}, see above"));
            return node;
        }
        let pushed = resolved.name.clone();
        if let Some(n) = &pushed {
            stack.push(n.clone());
        }

        let mut node = Node::new(NodeKind::Member, key, name);
        // A description next to the `$ref` (3.1, or a sibling the author
        // kept) wins over the target's.
        node.description = text_of(raw, "description").or_else(|| text_of(schema, "description"));
        node.deprecated = bool_of(schema, "deprecated") || bool_of(raw, "deprecated");
        node.read_only = bool_of(schema, "readOnly");
        node.write_only = bool_of(schema, "writeOnly");
        node.nullable = bool_of(schema, "nullable");
        node.default = schema.get("default").and_then(json_of);
        node.example = self.first_example(schema, false);
        if let Some(values) = schema.get("enum").and_then(Value::as_array) {
            node.enum_values = values.iter().filter_map(json_of).collect();
        }
        if let Some(constant) = schema.get("const").and_then(json_of) {
            node.enum_values = vec![constant];
        }
        node.constraints = constraints(schema);
        node.format = str_of(schema, "format").map(str::to_string);
        node.json_type = json_type(schema, &mut node.nullable);

        // allOf with no own properties and a single part reads as that part.
        let all_of = schema.get("allOf").and_then(Value::as_array);
        if let Some(parts) = all_of
            && parts.len() == 1
            && schema.get("properties").is_none()
        {
            let inner = self.schema_node(key, name, &parts[0], stack, depth + 1);
            node.type_name = inner.type_name.clone();
            node.json_type = node.json_type.take().or(inner.json_type.clone());
            node.format = node.format.take().or(inner.format.clone());
            node.description = node.description.take().or(inner.description.clone());
            node.example = node.example.take().or(inner.example.clone());
            node.default = node.default.take().or(inner.default.clone());
            if node.enum_values.is_empty() {
                node.enum_values = inner.enum_values.clone();
            }
            node.children = inner.children;
            node.deprecated |= inner.deprecated;
            if let Some(n) = &resolved.name {
                node.type_name = Some(n.clone());
            }
            if pushed.is_some() {
                stack.pop();
            }
            return node;
        }

        let is_object = node.json_type.as_deref() == Some("object")
            || schema.get("properties").is_some()
            || all_of.is_some();
        if is_object {
            node.json_type = Some("object".to_string());
            node.children = self.members(schema, stack, depth);
            if let Some(additional) = schema.get("additionalProperties")
                && additional.as_object().is_some()
            {
                let mut child = self.schema_node("{*}", "(any key)", additional, stack, depth + 1);
                child.kind = NodeKind::Member;
                node.children.push(child);
            }
        }
        if node.json_type.as_deref() == Some("array")
            && let Some(items) = schema.get("items")
        {
            let mut child = self.schema_node("[]", "items", items, stack, depth + 1);
            child.kind = NodeKind::Items;
            node.type_name = Some(format!(
                "array of {}",
                child.type_name.clone().unwrap_or_else(|| "any".to_string())
            ));
            node.children.push(child);
        }
        for (keyword, label) in [("oneOf", "one of"), ("anyOf", "any of")] {
            if let Some(variants) = schema.get(keyword).and_then(Value::as_array) {
                node.children
                    .push(self.variants(keyword, label, schema, variants, stack, depth));
            }
        }

        if node.type_name.is_none() {
            node.type_name = type_label(&resolved.name, &node);
        }
        if pushed.is_some() {
            stack.pop();
        }
        node
    }

    /// The members of an object schema, `allOf` parts merged in order: a part
    /// that is a named schema marks its members with that name as `origin`.
    fn members(&self, schema: &'a Value, stack: &mut Vec<String>, depth: usize) -> Vec<Node> {
        let mut found = Collected::default();
        self.collect_members(schema, None, stack, &mut found, 0);
        let Collected {
            props,
            required,
            extra,
        } = found;
        let mut out: Vec<Node> = props
            .into_iter()
            .map(|(name, prop, origin)| {
                let mut child = self.schema_node(&name, &name, prop, stack, depth + 1);
                if child.kind != NodeKind::Cycle && child.kind != NodeKind::Note {
                    child.kind = NodeKind::Member;
                }
                child.required = required.contains(&name);
                child.origin = origin;
                child
            })
            .collect();
        out.extend(extra);
        out
    }

    fn collect_members(
        &self,
        schema: &'a Value,
        origin: Option<String>,
        stack: &mut Vec<String>,
        found: &mut Collected<'a>,
        hops: usize,
    ) {
        if hops > MAX_DEPTH {
            return;
        }
        if let Some(parts) = schema.get("allOf").and_then(Value::as_array) {
            for part in parts {
                match self.resolve(part) {
                    Ok(r) => {
                        if let Some(n) = &r.name
                            && stack.contains(n)
                        {
                            let mut cycle = Node::new(NodeKind::Cycle, n.as_str(), n.as_str());
                            cycle.description = Some(format!("↻ {n}, see above"));
                            found.extra.push(cycle);
                            continue;
                        }
                        let part_origin = r.name.clone().or_else(|| origin.clone());
                        if let Some(n) = &r.name {
                            stack.push(n.clone());
                        }
                        self.collect_members(r.value, part_origin, stack, found, hops + 1);
                        if r.name.is_some() {
                            stack.pop();
                        }
                    }
                    Err(reference) => found.extra.push(Self::note(
                        &reference,
                        &reference,
                        format!("{reference} is outside this document"),
                    )),
                }
            }
        }
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            for (name, prop) in properties {
                let name = name.as_str();
                if let Some(slot) = found.props.iter_mut().find(|(n, _, _)| n == name) {
                    slot.1 = prop;
                    slot.2 = origin.clone();
                } else {
                    found.props.push((name.to_string(), prop, origin.clone()));
                }
            }
        }
        if let Some(list) = schema.get("required").and_then(Value::as_array) {
            found
                .required
                .extend(list.iter().filter_map(Value::as_str).map(str::to_string));
        }
    }

    fn variants(
        &self,
        keyword: &str,
        label: &str,
        schema: &'a Value,
        variants: &'a [Value],
        stack: &mut Vec<String>,
        depth: usize,
    ) -> Node {
        let mut node = Node::new(NodeKind::Variants, keyword, label);
        node.discriminator = schema
            .get("discriminator")
            .and_then(|d| str_of(d, "propertyName").map(str::to_string));
        for (index, variant) in variants.iter().enumerate() {
            let key = str_of(variant, "$ref")
                .map(last_segment)
                .or_else(|| text_of(variant, "title"))
                .unwrap_or_else(|| index.to_string());
            let mut child = self.schema_node(&key, &key, variant, stack, depth + 1);
            if child.kind != NodeKind::Cycle && child.kind != NodeKind::Note {
                child.kind = NodeKind::Variant;
            }
            node.children.push(child);
        }
        node.type_name = Some(
            node.children
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>()
                .join(" | "),
        );
        node
    }
}

/// The JSON type a schema declares, with 3.1's `[T, "null"]` read as a
/// nullable `T`.
fn json_type(schema: &Value, nullable: &mut bool) -> Option<String> {
    match schema.get("type")? {
        Value::String(t) => Some(t.clone()),
        Value::Array(types) => {
            let mut first = None;
            for t in types.iter().filter_map(Value::as_str) {
                if t == "null" {
                    *nullable = true;
                } else if first.is_none() {
                    first = Some(t.to_string());
                }
            }
            first
        }
        _ => None,
    }
}

/// `string (email)`, `Address`, `Status (string)`, `object`.
fn type_label(ref_name: &Option<String>, node: &Node) -> Option<String> {
    let base = node.json_type.as_deref().map(|t| match &node.format {
        Some(format) => format!("{t} ({format})"),
        None => t.to_string(),
    });
    match (ref_name, base) {
        (Some(name), Some(base)) if base != "object" => Some(format!("{name} ({base})")),
        (Some(name), _) => Some(name.clone()),
        (None, base) => base,
    }
}

/// The validation keywords of a schema in reading form.
fn constraints(schema: &Value) -> Vec<String> {
    let number = |key: &str| {
        schema.get(key).and_then(|v| match v {
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
    };
    let mut out = Vec::new();
    let exclusive_min = bool_of(schema, "exclusiveMinimum");
    let exclusive_max = bool_of(schema, "exclusiveMaximum");
    if let Some(min) = number("minimum") {
        out.push(format!("{} {min}", if exclusive_min { ">" } else { "min" }));
    } else if let Some(min) = number("exclusiveMinimum") {
        out.push(format!("> {min}"));
    }
    if let Some(max) = number("maximum") {
        out.push(format!("{} {max}", if exclusive_max { "<" } else { "max" }));
    } else if let Some(max) = number("exclusiveMaximum") {
        out.push(format!("< {max}"));
    }
    for (key, label) in [
        ("multipleOf", "multiple of"),
        ("minLength", "min length"),
        ("maxLength", "max length"),
        ("minItems", "min items"),
        ("maxItems", "max items"),
        ("minProperties", "min properties"),
        ("maxProperties", "max properties"),
    ] {
        if let Some(value) = number(key) {
            out.push(format!("{label} {value}"));
        }
    }
    if bool_of(schema, "uniqueItems") {
        out.push("unique items".to_string());
    }
    if let Some(pattern) = str_of(schema, "pattern") {
        out.push(format!("pattern {pattern}"));
    }
    out
}

// --- Examples --------------------------------------------------------------

/// Give every body without a spec example one built from its schema: a
/// member's `example`, else its `default`, else its first `enum` value, else
/// a placeholder for its type. `oneOf`/`anyOf` take the first variant; a
/// cycle is `null`.
pub fn fill_examples(node: &mut Node) {
    if node.kind == NodeKind::Body && node.example.is_none() {
        let direction = node.direction.unwrap_or(Direction::Message);
        node.example = Some(shape_sample(node, direction));
        node.example_generated = true;
    }
    for child in &mut node.children {
        fill_examples(child);
    }
}

fn sample(node: &Node, direction: Direction) -> Json {
    if let Some(example) = &node.example {
        return example.clone();
    }
    if let Some(default) = &node.default {
        return default.clone();
    }
    if let Some(first) = node.enum_values.first() {
        return first.clone();
    }
    shape_sample(node, direction)
}

fn shape_sample(node: &Node, direction: Direction) -> Json {
    match node.kind {
        NodeKind::Cycle | NodeKind::Note => return Json::Null,
        _ => {}
    }
    let variant = node
        .children
        .iter()
        .find(|c| c.kind == NodeKind::Variants)
        .and_then(|v| v.children.first())
        .map(|first| sample(first, direction));
    match node.json_type.as_deref() {
        Some("array") => {
            let item = node
                .children
                .iter()
                .find(|c| c.kind == NodeKind::Items)
                .map(|items| sample(items, direction))
                .unwrap_or(Json::Null);
            Json::Array(vec![item])
        }
        Some("string") => Json::String(string_placeholder(node.format.as_deref())),
        Some("integer") => Json::from(0),
        Some("number") => Json::from(0.0),
        Some("boolean") => Json::Bool(true),
        Some("null") => Json::Null,
        _ => {
            let mut object = serde_json::Map::new();
            for member in &node.children {
                let skip = !matches!(member.kind, NodeKind::Member | NodeKind::Cycle)
                    || (direction == Direction::Request && member.read_only)
                    || (direction == Direction::Response && member.write_only)
                    || member.key == "{*}";
                if skip {
                    continue;
                }
                object.insert(member.key.clone(), sample(member, direction));
            }
            match variant {
                Some(Json::Object(fields)) => {
                    for (k, v) in fields {
                        object.entry(k).or_insert(v);
                    }
                    Json::Object(object)
                }
                Some(other) if object.is_empty() => other,
                _ => Json::Object(object),
            }
        }
    }
}

fn string_placeholder(format: Option<&str>) -> String {
    match format {
        Some("date-time") => "2026-01-01T12:00:00Z",
        Some("date") => "2026-01-01",
        Some("time") => "12:00:00",
        Some("email") => "user@example.com",
        Some("uuid") => "00000000-0000-0000-0000-000000000000",
        Some("uri") | Some("url") => "https://example.com",
        Some("hostname") => "example.com",
        Some("ipv4") => "192.0.2.1",
        Some("ipv6") => "2001:db8::1",
        Some("byte") => "U2Fuc2hhaW4=",
        Some("binary") => "",
        _ => "string",
    }
    .to_string()
}

// --- Last changed ----------------------------------------------------------

/// Annotate the newest tree of a version line with, per node, the version
/// that last changed the node's own fields (`changed_in`) and, when newer,
/// the newest change below it (`inside`). `history` runs oldest first and
/// ends with the version being viewed; a node is "the same" in two versions
/// when its path of keys is, and it counts as changed when its own fields
/// differ or it was absent in the previous version.
pub fn annotate(history: Vec<(SemVer, Vec<Node>)>) -> Vec<Node> {
    let mut since: HashMap<String, SemVer> = HashMap::new();
    let mut previous: HashMap<String, String> = HashMap::new();
    let mut last = Vec::new();
    for (version, nodes) in history {
        let mut current: HashMap<String, String> = HashMap::new();
        for node in &nodes {
            fingerprints(node, "", &mut current);
        }
        for (path, print) in &current {
            if previous.get(path) != Some(print) {
                since.insert(path.clone(), version);
            }
        }
        previous = current;
        last = nodes;
    }
    for node in &mut last {
        apply(node, "", &since);
    }
    last
}

const SEPARATOR: char = '\u{1f}';

fn fingerprints(node: &Node, parent: &str, out: &mut HashMap<String, String>) {
    let path = format!("{parent}{SEPARATOR}{}", node.key);
    let mut own = node.clone();
    own.children = Vec::new();
    own.changed_in = None;
    own.inside = None;
    let print = serde_json::to_string(&own).unwrap_or_default();
    for child in &node.children {
        fingerprints(child, &path, out);
    }
    out.insert(path, print);
}

/// Set `changed_in` and `inside`; answers the newest version in the subtree.
fn apply(node: &mut Node, parent: &str, since: &HashMap<String, SemVer>) -> Option<SemVer> {
    let path = format!("{parent}{SEPARATOR}{}", node.key);
    node.changed_in = since.get(&path).copied();
    let mut newest_below: Option<SemVer> = None;
    for child in &mut node.children {
        let child_newest = apply(child, &path, since);
        newest_below = newest_below.max(child_newest);
    }
    node.inside = newest_below.filter(|below| Some(*below) > node.changed_in);
    node.changed_in.max(newest_below)
}

/// How many nodes a tree has, itself included.
pub fn node_count(node: &Node) -> usize {
    1 + node.children.iter().map(node_count).sum::<usize>()
}

/// Drop every endpoint's children, keeping `changed_in` and `inside`: the
/// whole-document answer, whose endpoints are opened one by one.
pub fn headers_only(nodes: &mut [Node]) {
    for node in nodes {
        node.children_omitted = !node.children.is_empty();
        node.children = Vec::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> SemVer {
        s.parse().unwrap()
    }

    fn child<'n>(node: &'n Node, key: &str) -> &'n Node {
        node.children
            .iter()
            .find(|c| c.key == key)
            .unwrap_or_else(|| panic!("no child {key} under {}", node.key))
    }

    fn at<'n>(node: &'n Node, keys: &[&str]) -> &'n Node {
        keys.iter().fold(node, |n, k| child(n, k))
    }

    const ORDERS: &str = r##"
openapi: 3.0.3
info: {title: Orders, version: 1.0.0}
paths:
  /orders/{id}:
    parameters:
      - name: id
        in: path
        required: true
        description: The order id.
        schema: {type: string, format: uuid}
    post:
      summary: Replace an order
      description: Replaces the whole order.
      parameters:
        - name: dryRun
          in: query
          schema: {type: boolean, default: false}
        - $ref: '#/components/parameters/Trace'
      requestBody:
        required: true
        content:
          application/json:
            schema: {$ref: '#/components/schemas/Order'}
      responses:
        '200':
          description: The stored order.
          headers:
            ETag:
              description: Version tag.
              schema: {type: string}
          content:
            application/json:
              schema: {$ref: '#/components/schemas/Order'}
        '404':
          description: No such order.
components:
  parameters:
    Trace:
      name: X-Trace
      in: header
      schema: {type: string, maxLength: 64}
  schemas:
    Base:
      type: object
      required: [id]
      properties:
        id: {type: string, readOnly: true, description: Server-assigned.}
    Order:
      description: An order.
      allOf:
        - $ref: '#/components/schemas/Base'
        - type: object
          required: [items]
          properties:
            status:
              type: string
              enum: [open, paid]
            secret: {type: string, writeOnly: true}
            items:
              type: array
              minItems: 1
              items: {$ref: '#/components/schemas/Line'}
            parent: {$ref: '#/components/schemas/Order'}
            payment:
              oneOf:
                - $ref: '#/components/schemas/Card'
                - $ref: '#/components/schemas/Invoice'
              discriminator: {propertyName: kind}
    Line:
      type: object
      properties:
        sku: {type: string, example: ABC-1}
        quantity: {type: integer, minimum: 1, default: 1}
    Card:
      type: object
      properties:
        kind: {type: string}
        last4: {type: string, pattern: '^[0-9]{4}$'}
    Invoice:
      type: object
      properties:
        kind: {type: string}
        due: {type: string, format: date}
"##;

    fn orders() -> Node {
        endpoint_node(
            ApiType::OpenApi,
            "POST",
            "/orders/{id}",
            "/orders/{}",
            ORDERS,
        )
        .unwrap()
    }

    #[test]
    fn an_openapi_endpoint_lists_parameters_by_location_body_and_responses() {
        let node = orders();
        assert_eq!(node.key, "POST /orders/{}");
        assert_eq!(node.name, "POST /orders/{id}");
        assert_eq!(node.summary.as_deref(), Some("Replace an order"));
        assert_eq!(
            node.description.as_deref(),
            Some("Replaces the whole order.")
        );
        let keys: Vec<&str> = node.children.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, ["parameters", "request", "responses"]);

        let id = at(&node, &["parameters", "path", "id"]);
        assert_eq!(id.kind, NodeKind::Parameter);
        assert!(id.required);
        assert_eq!(id.type_name.as_deref(), Some("string (uuid)"));
        assert_eq!(id.description.as_deref(), Some("The order id."));
        let dry_run = at(&node, &["parameters", "query", "dryRun"]);
        assert_eq!(dry_run.default, Some(Json::Bool(false)));
        let trace = at(&node, &["parameters", "header", "X-Trace"]);
        assert_eq!(trace.constraints, ["max length 64"]);

        let request = child(&node, "request");
        assert!(request.required);
        let body = child(request, "application/json");
        assert_eq!(body.kind, NodeKind::Body);
        assert_eq!(body.type_name.as_deref(), Some("Order"));
        assert_eq!(body.description.as_deref(), Some("An order."));

        let ok = at(&node, &["responses", "200"]);
        assert_eq!(ok.kind, NodeKind::Response);
        assert_eq!(ok.description.as_deref(), Some("The stored order."));
        let etag = at(ok, &["headers", "ETag"]);
        assert_eq!(etag.kind, NodeKind::Header);
        assert_eq!(etag.description.as_deref(), Some("Version tag."));
        assert!(
            child(&node, "responses")
                .children
                .iter()
                .any(|c| c.key == "404")
        );
    }

    #[test]
    fn all_of_is_merged_with_its_origin_and_refs_are_inlined() {
        let node = orders();
        let order = at(&node, &["request", "application/json"]);
        let names: Vec<&str> = order.children.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(
            names,
            ["id", "status", "secret", "items", "parent", "payment"]
        );
        let id = child(order, "id");
        assert_eq!(id.origin.as_deref(), Some("Base"));
        assert!(id.required && id.read_only);
        assert_eq!(id.description.as_deref(), Some("Server-assigned."));
        let status = child(order, "status");
        assert_eq!(status.origin, None);
        assert_eq!(status.enum_values, [Json::from("open"), Json::from("paid")]);
        let items = child(order, "items");
        assert!(items.required);
        assert_eq!(items.type_name.as_deref(), Some("array of Line"));
        assert_eq!(items.constraints, ["min items 1"]);
        let quantity = at(items, &["[]", "quantity"]);
        assert_eq!(quantity.constraints, ["min 1"]);
        assert_eq!(quantity.default, Some(Json::from(1)));
    }

    #[test]
    fn a_recursive_ref_stops_with_a_cycle_leaf() {
        let node = orders();
        let parent = at(&node, &["request", "application/json", "parent"]);
        assert_eq!(parent.kind, NodeKind::Cycle);
        assert_eq!(parent.description.as_deref(), Some("↻ Order, see above"));
        assert!(parent.children.is_empty());
    }

    #[test]
    fn one_of_keeps_named_variants_and_the_discriminator() {
        let node = orders();
        let payment = at(&node, &["request", "application/json", "payment"]);
        let variants = child(payment, "oneOf");
        assert_eq!(variants.kind, NodeKind::Variants);
        assert_eq!(variants.discriminator.as_deref(), Some("kind"));
        let keys: Vec<&str> = variants.children.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, ["Card", "Invoice"]);
        assert_eq!(
            at(variants, &["Card", "last4"]).constraints,
            ["pattern ^[0-9]{4}$"]
        );
    }

    #[test]
    fn examples_use_spec_values_then_defaults_then_placeholders_per_direction() {
        let mut node = orders();
        fill_examples(&mut node);
        let request = at(&node, &["request", "application/json"]);
        assert!(request.example_generated);
        let example = request.example.as_ref().unwrap();
        assert_eq!(
            *example,
            serde_json::json!({
                "status": "open",
                "secret": "string",
                "items": [{ "sku": "ABC-1", "quantity": 1 }],
                "parent": null,
                "payment": { "kind": "string", "last4": "string" }
            }),
            "a request leaves out readOnly members"
        );
        let response = at(&node, &["responses", "200", "application/json"]);
        let example = response.example.as_ref().unwrap();
        assert_eq!(example["id"], Json::from("string"));
        assert!(
            example.get("secret").is_none(),
            "a response leaves out writeOnly"
        );
    }

    #[test]
    fn a_spec_example_on_the_media_type_wins_over_the_built_one_refs_resolved() {
        let spec = r#"
openapi: 3.0.3
info: {title: T, version: 1.0.0}
paths:
  /t:
    get:
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema: {type: object, properties: {a: {type: integer}}}
              examples:
                one: {$ref: '#/components/examples/Seven'}
components:
  examples:
    Seven: {value: {a: 7}}
"#;
        let mut node = endpoint_node(ApiType::OpenApi, "GET", "/t", "/t", spec).unwrap();
        fill_examples(&mut node);
        let body = at(&node, &["responses", "200", "application/json"]);
        assert_eq!(body.example, Some(serde_json::json!({"a": 7})));
        assert!(!body.example_generated);
    }

    #[test]
    fn placeholders_are_fixed_for_formats() {
        assert_eq!(
            string_placeholder(Some("date-time")),
            "2026-01-01T12:00:00Z"
        );
        assert_eq!(string_placeholder(Some("email")), "user@example.com");
        assert_eq!(string_placeholder(None), "string");
    }

    #[test]
    fn an_external_ref_is_a_note_not_a_failure() {
        let spec = r#"
openapi: 3.0.3
info: {title: T, version: 1.0.0}
paths:
  /t:
    get:
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema: {$ref: 'common.yaml#/Thing'}
"#;
        let node = endpoint_node(ApiType::OpenApi, "GET", "/t", "/t", spec).unwrap();
        let body = at(&node, &["responses", "200", "application/json"]);
        assert_eq!(body.kind, NodeKind::Body);
        assert_eq!(body.type_name.as_deref(), Some("common.yaml#/Thing"));
        assert!(
            body.description
                .as_deref()
                .unwrap()
                .contains("outside this document")
        );
    }

    #[test]
    fn openapi_3_1_type_lists_read_as_nullable() {
        let spec = r#"
openapi: 3.1.0
info: {title: T, version: 1.0.0}
paths:
  /t:
    get:
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: object
                properties:
                  note: {type: [string, 'null']}
"#;
        let node = endpoint_node(ApiType::OpenApi, "GET", "/t", "/t", spec).unwrap();
        let note = at(&node, &["responses", "200", "application/json", "note"]);
        assert_eq!(note.type_name.as_deref(), Some("string"));
        assert!(note.nullable);
    }

    const ASYNC_V2: &str = r##"
asyncapi: 2.6.0
info: {title: Shop, version: 1.0.0}
channels:
  orders/{region}:
    description: Order events.
    parameters:
      region:
        description: The region.
        schema: {type: string, enum: [eu, us]}
    publish:
      summary: Order placed
      message:
        $ref: '#/components/messages/OrderPlaced'
components:
  messages:
    OrderPlaced:
      contentType: application/json
      description: Sent once per order.
      headers:
        type: object
        properties:
          traceId: {type: string}
      payload:
        type: object
        required: [orderId]
        properties:
          orderId: {type: string, format: uuid, description: The order.}
          total: {type: number}
"##;

    #[test]
    fn an_asyncapi_2_operation_shows_parameters_and_its_message() {
        let mut node = endpoint_node(
            ApiType::AsyncApi,
            "PUB",
            "orders/{region}",
            "orders/{}",
            ASYNC_V2,
        )
        .unwrap();
        assert_eq!(node.key, "PUB orders/{}");
        assert_eq!(node.summary.as_deref(), Some("Order placed"));
        let region = at(&node, &["parameters", "region"]);
        assert_eq!(region.description.as_deref(), Some("The region."));
        assert_eq!(region.enum_values, [Json::from("eu"), Json::from("us")]);
        let message = child(&node, "OrderPlaced");
        assert_eq!(message.kind, NodeKind::Message);
        assert_eq!(message.description.as_deref(), Some("Sent once per order."));
        let payload = child(message, "payload");
        assert_eq!(payload.name, "Payload (application/json)");
        assert!(child(payload, "orderId").required);
        assert!(child(child(message, "headers"), "traceId").kind == NodeKind::Member);
        fill_examples(&mut node);
        let payload = at(&node, &["OrderPlaced", "payload"]);
        assert_eq!(
            payload.example,
            Some(serde_json::json!({
                "orderId": "00000000-0000-0000-0000-000000000000",
                "total": 0.0
            }))
        );
    }

    #[test]
    fn an_asyncapi_3_operation_takes_its_listed_messages() {
        let spec = r##"
asyncapi: 3.0.0
info: {title: Shop, version: 1.0.0}
channels:
  orders:
    address: orders.v1
    messages:
      placed:
        name: OrderPlaced
        payload:
          type: object
          properties:
            id: {type: string}
      avro:
        payload:
          schemaFormat: application/vnd.apache.avro;version=1.9.0
          schema: {type: record, name: X, fields: []}
operations:
  send_orders:
    action: send
    description: Publishes orders.
    channel: {$ref: '#/channels/orders'}
    messages:
      - $ref: '#/channels/orders/messages/placed'
      - $ref: '#/channels/orders/messages/avro'
"##;
        let node = endpoint_node(ApiType::AsyncApi, "PUB", "orders.v1", "orders.v1", spec).unwrap();
        assert_eq!(node.description.as_deref(), Some("Publishes orders."));
        let keys: Vec<&str> = node.children.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, ["placed", "avro"]);
        assert_eq!(
            child(at(&node, &["placed", "payload"]), "id").kind,
            NodeKind::Member
        );
        let avro = at(&node, &["avro", "payload"]);
        assert_eq!(avro.kind, NodeKind::Note);
        assert!(avro.description.as_deref().unwrap().contains("avro"));
    }

    #[test]
    fn proto_is_refused() {
        let err =
            endpoint_node(ApiType::Proto, "Get", "svc", "svc", "syntax = \"proto3\";").unwrap_err();
        assert!(err.contains("OpenAPI and AsyncAPI"), "{err}");
    }

    fn member(key: &str, description: &str, children: Vec<Node>) -> Node {
        let mut node = Node::new(NodeKind::Member, key, key);
        node.description = Some(description.to_string());
        node.children = children;
        node
    }

    #[test]
    fn last_changed_is_per_node_own_fields_with_inside_for_newer_descendants() {
        let v1 = vec![member(
            "Order",
            "An order.",
            vec![
                member("id", "The id.", vec![]),
                member("total", "Sum.", vec![]),
            ],
        )];
        let v2 = vec![member(
            "Order",
            "An order.",
            vec![
                member("id", "The id.", vec![]),
                member("total", "The sum, tax included.", vec![]),
            ],
        )];
        let v3 = vec![member(
            "Order",
            "An order.",
            vec![
                member("id", "The id.", vec![]),
                member("total", "The sum, tax included.", vec![]),
                member("currency", "ISO 4217.", vec![]),
            ],
        )];
        let tree = annotate(vec![(v("1.0.0"), v1), (v("1.1.0"), v2), (v("1.2.0"), v3)]);
        let order = &tree[0];
        assert_eq!(order.changed_in, Some(v("1.0.0")));
        assert_eq!(order.inside, Some(v("1.2.0")));
        assert_eq!(child(order, "id").changed_in, Some(v("1.0.0")));
        assert_eq!(child(order, "id").inside, None);
        assert_eq!(child(order, "total").changed_in, Some(v("1.1.0")));
        assert_eq!(child(order, "currency").changed_in, Some(v("1.2.0")));
    }

    #[test]
    fn a_node_that_returns_counts_from_its_return() {
        let with = vec![member("Order", "x", vec![member("note", "n", vec![])])];
        let without = vec![member("Order", "x", vec![])];
        let tree = annotate(vec![
            (v("1.0.0"), with.clone()),
            (v("1.1.0"), without),
            (v("1.2.0"), with),
        ]);
        assert_eq!(child(&tree[0], "note").changed_in, Some(v("1.2.0")));
        assert_eq!(tree[0].changed_in, Some(v("1.0.0")));
    }

    #[test]
    fn a_reformatted_spec_changes_nothing() {
        let compact = ORDERS.to_string();
        let reformatted = ORDERS.replace(
            "{type: string, format: uuid}",
            "\n          type: string\n          format: uuid",
        );
        let a = endpoint_node(
            ApiType::OpenApi,
            "POST",
            "/orders/{id}",
            "/orders/{}",
            &compact,
        )
        .unwrap();
        let b = endpoint_node(
            ApiType::OpenApi,
            "POST",
            "/orders/{id}",
            "/orders/{}",
            &reformatted,
        )
        .unwrap();
        let tree = annotate(vec![(v("1.0.0"), vec![a]), (v("1.1.0"), vec![b])]);
        assert_eq!(tree[0].changed_in, Some(v("1.0.0")));
        assert_eq!(tree[0].inside, None);
    }

    #[test]
    fn required_membership_belongs_to_the_member() {
        let base = ORDERS.to_string();
        let more_required = ORDERS.replace("required: [items]", "required: [items, status]");
        let a = endpoint_node(
            ApiType::OpenApi,
            "POST",
            "/orders/{id}",
            "/orders/{}",
            &base,
        )
        .unwrap();
        let b = endpoint_node(
            ApiType::OpenApi,
            "POST",
            "/orders/{id}",
            "/orders/{}",
            &more_required,
        )
        .unwrap();
        let tree = annotate(vec![(v("1.0.0"), vec![a]), (v("1.1.0"), vec![b])]);
        let body = at(&tree[0], &["request", "application/json"]);
        assert_eq!(body.changed_in, Some(v("1.0.0")));
        assert_eq!(child(body, "status").changed_in, Some(v("1.1.0")));
        assert_eq!(child(body, "items").changed_in, Some(v("1.0.0")));
    }

    #[test]
    fn an_unreadable_snippet_keeps_its_endpoint_identity_with_a_note() {
        let node = unreadable_endpoint("GET", "/a/{id}", "/a/{}", "bad yaml".to_string());
        assert_eq!(node.key, "GET /a/{}");
        assert_eq!(node.name, "GET /a/{id}");
        assert_eq!(node.children[0].kind, NodeKind::Note);
        assert_eq!(node.children[0].description.as_deref(), Some("bad yaml"));
        assert_eq!(node_count(&node), 2);
    }

    #[test]
    fn headers_only_keeps_the_versions_and_flags_the_omission() {
        let mut nodes = annotate(vec![(v("1.0.0"), vec![orders()])]);
        headers_only(&mut nodes);
        assert!(nodes[0].children.is_empty());
        assert!(nodes[0].children_omitted);
        assert_eq!(nodes[0].changed_in, Some(v("1.0.0")));
    }
}
