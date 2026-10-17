//! The seven tools the MCP server offers (docs/mcp.md). Their names,
//! parameters and JSON schemas are the contract AI clients are written
//! against; the answers are compact JSON carrying the scope they were
//! computed for, and only catalog data.

use super::{ToolError, ToolOutput};
use crate::AppState;
use crate::application::mcp_service::{self, GraphFilter, ScopeStamp};
use crate::application::services;
use crate::application::text_diff::DiffLineOp;
use crate::domain::models::{ApiType, EndpointSearch, SemVer};
use rmcp::model::{JsonObject, Tool, ToolAnnotations};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// The API types a tool accepts, spelled as the catalog spells them.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ToolApiType {
    OpenApi,
    AsyncApi,
    Proto,
}

impl From<ToolApiType> for ApiType {
    fn from(t: ToolApiType) -> Self {
        match t {
            ToolApiType::OpenApi => ApiType::OpenApi,
            ToolApiType::AsyncApi => ApiType::AsyncApi,
            ToolApiType::Proto => ApiType::Proto,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListServicesParams {
    /// Which graph to describe: `dev` (default; everything ever provided or
    /// required), `main` (what trunk builds currently pin), or the name of a
    /// sanshain-branch (a release cut).
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DescribeServiceParams {
    /// The service's name, exactly as `list_services` answers it.
    pub name: String,
    /// `dev` (default), `main`, or a sanshain-branch name.
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindEndpointParams {
    /// Part of a path, channel or method name, matched case-insensitively
    /// as a substring; `*` matches anything (`/users/*/orders`).
    pub query: String,
    /// Keep only this HTTP verb / operation (`GET`, `PUBLISH`, ...).
    pub method: Option<String>,
    /// Keep only this API type.
    pub api_type: Option<ToolApiType>,
    /// Search every stored version instead of only the latest version of
    /// each Producer's line (default false).
    pub all_versions: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetSpecParams {
    /// The Producer's name.
    pub service: String,
    /// Which of the Producer's lines: `openapi`, `asyncapi` or `proto`.
    pub api_type: ToolApiType,
    /// `MAJOR.MINOR.PATCH`; the line's latest version when omitted.
    pub version: Option<String>,
    /// With `method`: answer only this endpoint's snippet instead of the
    /// whole document.
    pub path: Option<String>,
    /// With `path`: the endpoint's HTTP verb / operation.
    pub method: Option<String>,
    /// First line to answer (1-based line number; default 1).
    pub offset: Option<usize>,
    /// How many lines to answer from `offset` (default: all remaining).
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetGraphParams {
    /// `dev` (default), `main`, or a sanshain-branch name.
    pub scope: Option<String>,
    /// An RFC 3339 instant: the graph as it was then (`main` and branches
    /// only — `dev` has no timeline).
    pub at: Option<String>,
    /// Narrow to this service and its direct neighbours.
    pub service: Option<String>,
    /// Keep only edges and version lines of this API type.
    pub api_type: Option<ToolApiType>,
    /// Also answer the Peripheral Resources (databases, queues, buckets from
    /// Backstage) as `resource` nodes with `peripheral` edges (default false).
    pub include_periphery: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiffVersionsParams {
    /// The Producer's name.
    pub service: String,
    /// `openapi`, `asyncapi` or `proto`.
    pub api_type: ToolApiType,
    /// The older version, `MAJOR.MINOR.PATCH`.
    pub from: String,
    /// The newer version, `MAJOR.MINOR.PATCH`.
    pub to: String,
    /// Ignore whitespace-only differences (default false).
    pub ignore_whitespace: Option<bool>,
    /// With `method`: diff only this endpoint's snippet.
    pub path: Option<String>,
    /// With `path`: the endpoint's HTTP verb / operation.
    pub method: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlameParams {
    /// The Producer's name.
    pub service: String,
    /// `openapi`, `asyncapi` or `proto`.
    pub api_type: ToolApiType,
    /// The version to blame, `MAJOR.MINOR.PATCH`.
    pub version: String,
    /// With `method`: blame only this endpoint's snippet.
    pub path: Option<String>,
    /// With `path`: the endpoint's HTTP verb / operation.
    pub method: Option<String>,
}

/// The tool catalog, in the order clients see it. Fails only when a
/// parameter type's schema is not an object, which a unit test pins.
pub fn catalog() -> Result<Vec<Tool>, String> {
    Ok(vec![
        tool::<ListServicesParams>(
            "list_services",
            "Lists every Producer and Consumer in a graph scope: use it first to learn the names the \
             other tools take. For each Producer: its domain and icon, the API types it provides, its \
             version line per API type (version, stability `ga`/`snapshot`, when it was last provided), \
             how many endpoints it has, how many distinct Consumers require it, and `unrequired` when \
             nobody does. For each Consumer: how many endpoints it requires and from which Producers. \
             `scope` is `dev` (default: everything ever provided or required), `main` (what trunk \
             builds currently pin) or a sanshain-branch name (a release cut). The answer repeats the \
             `scope` it describes.",
        )?,
        tool::<DescribeServiceParams>(
            "describe_service",
            "Everything about one service, Producer or Consumer, by name. For a Producer: every \
             stored version line entry (api_type, version, stability, who provided it, when it was last \
             provided, endpoint count, and `superseded` when a newer version of that line exists), the \
             endpoints of the latest version of each line (api_type, method, path, deprecated), and who \
             requires what version of it. For any service: what it requires from others, its \
             periphery — the Backstage resources (databases, queues, buckets) it depends on and which \
             other systems share each — and its `context` from its Backstage catalog-info: owner \
             (kind and name), description, source location (the repository) and links (wiki, \
             dashboards, ticket board); `null` when the service pushed no catalog-info. Use it after \
             `list_services`, before `get_spec` or `find_endpoint`. Unknown names are refused with a \
             message. `scope` as in `list_services`.",
        )?,
        tool::<FindEndpointParams>(
            "find_endpoint",
            "Finds endpoints across all Producers whose path, channel or method name contains `query` \
             (case-insensitive substring; `*` is a wildcard, e.g. `/users/*/orders` or `order*`). By \
             default only the latest version of every Producer's line is searched; set `all_versions` to \
             search every stored version. Narrow with `method` (an HTTP verb or operation) and \
             `api_type`. Each hit names the Producer, api_type, version, stability, method, path, whether \
             the endpoint is deprecated, and how many Consumers pin it at that version. Use it to answer \
             \"who offers X?\" or \"where is the endpoint for Y?\" before `get_spec`.",
        )?,
        tool::<GetSpecParams>(
            "get_spec",
            "The stored specification text of a Producer's version: the whole document (OpenAPI/AsyncAPI \
             YAML or a .proto file) when `path` and `method` are omitted, or just that endpoint's snippet \
             when both are given. `version` defaults to the latest version of the line. Large documents \
             are answered as a summary (line count, byte count, the endpoints) with a hint; page through \
             them with `offset` (1-based line number) and `limit` (lines) — the answer carries \
             `next_offset` while more lines remain — or ask for one endpoint with `path` + `method`. \
             Unknown services, versions or endpoints are refused with a message.",
        )?,
        tool::<GetGraphParams>(
            "get_graph",
            "The dependency graph of a scope as nodes and edges. Nodes: `service`, `kind` (`producer`, \
             `consumer`, `both`, `missing` for a service that is required but not provided in the scope, \
             `resource` for a Peripheral Resource), the version lines it `provides`, and the services it \
             `requires`. Edges go from the requiring Consumer to the Producer with api_type, method, path, \
             the pinned version and its stability, `outdated` when the pin is below the line's latest \
             version in the scope and `superseded` when it is a snapshot below the latest GA; harvested \
             AsyncAPI subscriptions carry `harvested`, periphery edges `peripheral`. `scope` is `dev`, \
             `main` or a sanshain-branch, `at` an RFC 3339 instant for `main`/branches. `service` keeps \
             that node and its neighbours; `api_type` keeps one API type; `include_periphery` adds the \
             Backstage resources. A graph too large to answer comes back as counts per kind with a hint.",
        )?,
        tool::<DiffVersionsParams>(
            "diff_versions",
            "The unified diff between two versions of a Producer's line (`from` older, `to` newer), the \
             same text diff the web UI shows, plus how many lines were added and removed. Diff the whole \
             document, or one endpoint's snippet with `path` + `method`; `ignore_whitespace` drops \
             whitespace-only changes. Use it to answer \"what changed between 1.2.0 and 1.3.0?\" or \
             \"is this upgrade breaking?\". A diff too large to answer comes back as the counts with a \
             hint to narrow with `path` + `method`. Unknown versions are refused with a message.",
        )?,
        tool::<BlameParams>(
            "blame",
            "Which version last changed each part of a Producer's specification: the text of `version` \
             split into consecutive blocks (`start_line`, `lines`) each attributed to the version that \
             last changed those lines, with who provided that version and when. Blame the whole document \
             or one endpoint's snippet with `path` + `method`. Use it to answer \"since when has this \
             endpoint looked like this?\" or \"who introduced this field?\". When the text is too large \
             the blocks are answered without their lines (with `line_count`), and a hint to narrow.",
        )?,
    ])
}

/// The input schema of a parameter type: JSON Schema 2020-12 with every
/// subschema inlined (an enum reads as `enum`, not as a `$ref` an AI client
/// has to chase), the root an object, the root title and description
/// dropped (they name the Rust type, which is noise to the client).
fn input_schema<T: JsonSchema>() -> Result<JsonObject, String> {
    let settings = schemars::generate::SchemaSettings::draft2020_12().with(|s| {
        s.inline_subschemas = true;
    });
    let schema = settings.into_generator().into_root_schema_for::<T>();
    let Value::Object(mut object) =
        serde_json::to_value(schema).map_err(|e| format!("schema serialization failed: {e}"))?
    else {
        return Err("schema is not a JSON object".to_string());
    };
    if object.get("type") != Some(&Value::String("object".to_string())) {
        return Err("MCP tool parameters must be an object".to_string());
    }
    object.remove("title");
    object.remove("description");
    Ok(object)
}

fn tool<T: JsonSchema>(name: &'static str, description: &'static str) -> Result<Tool, String> {
    let schema = input_schema::<T>().map_err(|e| format!("{name}: {e}"))?;
    Ok(Tool::new(name, description, schema)
        .annotate(ToolAnnotations::new().read_only(true).idempotent(true)))
}

fn params<T: for<'de> Deserialize<'de>>(tool: &str, arguments: Value) -> Result<T, ToolError> {
    serde_json::from_value(arguments)
        .map_err(|e| ToolError(format!("invalid parameters for {tool}: {e}")))
}

fn version(raw: &str) -> Result<SemVer, ToolError> {
    raw.parse()
        .map_err(|e: String| ToolError(format!("{e} — versions are MAJOR.MINOR.PATCH")))
}

/// `path` and `method` go together: both name an endpoint, neither means
/// the whole document.
fn endpoint<'a>(
    path: Option<&'a str>,
    method: Option<&'a str>,
) -> Result<Option<(&'a str, &'a str)>, ToolError> {
    match (path, method) {
        (Some(p), Some(m)) => Ok(Some((p, m))),
        (None, None) => Ok(None),
        _ => Err(ToolError(
            "path and method go together: give both for one endpoint, or neither for the whole document"
                .to_string(),
        )),
    }
}

fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, ToolError> {
    serde_json::to_value(value)
        .map_err(|e| ToolError(format!("could not serialize the answer: {e}")))
}

/// Run `tool` with `arguments`, answering its output or the refusal the
/// client should read.
pub async fn dispatch(
    state: &AppState,
    tool: &str,
    arguments: Value,
) -> Result<ToolOutput, ToolError> {
    match tool {
        "list_services" => list_services(state, params(tool, arguments)?).await,
        "describe_service" => describe_service(state, params(tool, arguments)?).await,
        "find_endpoint" => find_endpoint(state, params(tool, arguments)?).await,
        "get_spec" => get_spec(state, params(tool, arguments)?).await,
        "get_graph" => get_graph(state, params(tool, arguments)?).await,
        "diff_versions" => diff_versions(state, params(tool, arguments)?).await,
        "blame" => blame(state, params(tool, arguments)?).await,
        _ => Err(ToolError(format!(
            "unknown tool '{tool}' — tools/list names the available ones"
        ))),
    }
}

async fn list_services(state: &AppState, p: ListServicesParams) -> Result<ToolOutput, ToolError> {
    let list = mcp_service::list_services(&state.repo, p.scope.as_deref()).await?;
    let over_budget = json!({
        "scope": list.stamp.scope, "at": list.stamp.at,
        "producers": list.producers.len(), "consumers": list.consumers.len(),
        "hint": "too many services to list at once: use describe_service for one service, or get_graph with service / api_type to narrow",
    });
    Ok(ToolOutput {
        body: to_value(&list)?,
        over_budget,
    })
}

async fn describe_service(
    state: &AppState,
    p: DescribeServiceParams,
) -> Result<ToolOutput, ToolError> {
    let d = mcp_service::describe_service(&state.repo, &p.name, p.scope.as_deref()).await?;
    let over_budget = json!({
        "scope": d.stamp.scope, "at": d.stamp.at, "name": d.name,
        "versions": d.versions.len(), "endpoints": d.endpoints.len(),
        "consumers": d.consumers.len(), "requires": d.requires.len(), "periphery": d.periphery.len(),
        "hint": "too much to answer at once: use find_endpoint with a query for its endpoints, get_spec with path + method for one of them, or get_graph with service for its neighbours",
    });
    Ok(ToolOutput {
        body: to_value(&d)?,
        over_budget,
    })
}

async fn find_endpoint(state: &AppState, p: FindEndpointParams) -> Result<ToolOutput, ToolError> {
    let search = EndpointSearch {
        query: p.query.clone(),
        method: p.method,
        api_type: p.api_type.map(ApiType::from),
        all_versions: p.all_versions.unwrap_or(false),
    };
    let hits = mcp_service::find_endpoint(&state.repo, &search).await?;
    let stamp = ScopeStamp::dev();
    let mut per_producer: std::collections::BTreeMap<&str, usize> = Default::default();
    for hit in &hits {
        *per_producer.entry(hit.producer.as_str()).or_insert(0) += 1;
    }
    let over_budget = json!({
        "scope": stamp.scope, "at": stamp.at, "query": p.query,
        "total": hits.len(), "per_producer": per_producer,
        "hint": "too many hits to answer: narrow the query, or set method / api_type",
    });
    let body = json!({
        "scope": stamp.scope, "at": stamp.at, "query": p.query,
        "total": hits.len(), "hits": hits,
    });
    Ok(ToolOutput { body, over_budget })
}

async fn get_spec(state: &AppState, p: GetSpecParams) -> Result<ToolOutput, ToolError> {
    let endpoint = endpoint(p.path.as_deref(), p.method.as_deref())?;
    let version = p.version.as_deref().map(self::version).transpose()?;
    let api_type = ApiType::from(p.api_type);
    let doc =
        mcp_service::get_spec_text(&state.repo, &p.service, api_type, version, endpoint).await?;
    let lines: Vec<&str> = doc.text.lines().collect();
    let total = lines.len();
    let offset = p.offset.unwrap_or(1).max(1);
    let start = (offset - 1).min(total);
    let end = match p.limit {
        Some(limit) => start.saturating_add(limit).min(total),
        None => total,
    };
    let next_offset = (end < total).then_some(end + 1);
    let body = json!({
        "scope": doc.stamp.scope, "at": doc.stamp.at,
        "service": doc.service, "api_type": doc.api_type, "version": doc.version,
        "stability": doc.stability, "path": doc.path, "method": doc.method,
        "lines": total, "bytes": doc.text.len(),
        "offset": offset, "limit": p.limit, "next_offset": next_offset,
        "text": lines[start..end].join("\n"),
    });
    let endpoints =
        mcp_service::endpoint_refs(&state.repo, &doc.service, doc.api_type, doc.version).await?;
    let over_budget = json!({
        "scope": doc.stamp.scope, "at": doc.stamp.at,
        "service": doc.service, "api_type": doc.api_type, "version": doc.version,
        "path": doc.path, "method": doc.method,
        "summary": { "lines": total, "bytes": doc.text.len(), "endpoints": endpoints },
        "hint": "use offset/limit (line numbers) or path+method",
    });
    Ok(ToolOutput { body, over_budget })
}

async fn get_graph(state: &AppState, p: GetGraphParams) -> Result<ToolOutput, ToolError> {
    let filter = GraphFilter {
        service: p.service,
        api_type: p.api_type.map(ApiType::from),
        include_periphery: p.include_periphery.unwrap_or(false),
    };
    let graph =
        mcp_service::get_graph(&state.repo, p.scope.as_deref(), p.at.as_deref(), &filter).await?;
    let over_budget = json!({
        "scope": graph.stamp.scope, "at": graph.stamp.at,
        "nodes": mcp_service::graph_counts(&graph), "edges": graph.edges.len(),
        "hint": "the graph is too large to answer: narrow it with service (one node and its neighbours) or api_type",
    });
    Ok(ToolOutput {
        body: to_value(&graph)?,
        over_budget,
    })
}

async fn diff_versions(state: &AppState, p: DiffVersionsParams) -> Result<ToolOutput, ToolError> {
    let endpoint = endpoint(p.path.as_deref(), p.method.as_deref())?;
    let request = services::DiffRequest {
        api_type: ApiType::from(p.api_type),
        from: version(&p.from)?,
        to: version(&p.to)?,
        endpoint,
        options: services::DiffOptions {
            ignore_whitespace: p.ignore_whitespace.unwrap_or(false),
            context: Some(3),
        },
    };
    let diff = services::diff_versions(&state.repo, &p.service, &request).await?;
    let (added, removed) =
        diff.hunks
            .iter()
            .flat_map(|h| h.lines.iter())
            .fold((0usize, 0usize), |(a, r), line| match line.op {
                DiffLineOp::Add => (a + 1, r),
                DiffLineOp::Del => (a, r + 1),
                DiffLineOp::Ctx => (a, r),
            });
    let text = services::render_unified(
        &diff.hunks,
        &format!("{} {}", p.service, diff.from),
        &format!("{} {}", p.service, diff.to),
    );
    let stamp = ScopeStamp::dev();
    let header = json!({
        "scope": stamp.scope, "at": stamp.at,
        "service": p.service, "api_type": request.api_type,
        "from": diff.from, "to": diff.to, "path": p.path, "method": p.method,
        "added": added, "removed": removed,
    });
    let mut body = header.clone();
    body["diff"] = Value::String(text);
    let mut over_budget = header;
    over_budget["hint"] = Value::String(
        "the diff is too large to answer: narrow it to one endpoint with path + method".to_string(),
    );
    Ok(ToolOutput { body, over_budget })
}

async fn blame(state: &AppState, p: BlameParams) -> Result<ToolOutput, ToolError> {
    let endpoint = endpoint(p.path.as_deref(), p.method.as_deref())?;
    let api_type = ApiType::from(p.api_type);
    let version = version(&p.version)?;
    let blocks =
        services::blame_version(&state.repo, &p.service, api_type, version, endpoint).await?;
    let stamp = ScopeStamp::dev();
    let header = json!({
        "scope": stamp.scope, "at": stamp.at,
        "service": p.service, "api_type": api_type, "version": version,
        "path": p.path, "method": p.method,
    });
    let full: Vec<Value> = blocks
        .iter()
        .map(|b| {
            json!({
                "version": b.version, "stability": b.stability, "provided_by": b.provided_by,
                "date": b.updated_at, "start_line": b.start_line, "lines": b.lines,
            })
        })
        .collect();
    let bare: Vec<Value> = blocks
        .iter()
        .map(|b| {
            json!({
                "version": b.version, "stability": b.stability, "provided_by": b.provided_by,
                "date": b.updated_at, "start_line": b.start_line, "line_count": b.lines.len(),
            })
        })
        .collect();
    let mut body = header.clone();
    body["blocks"] = Value::Array(full);
    let mut over_budget = header;
    over_budget["blocks"] = Value::Array(bare);
    over_budget["hint"] = Value::String(
        "the text is too large to answer with its lines: narrow to one endpoint with path + method"
            .to_string(),
    );
    Ok(ToolOutput { body, over_budget })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_has_the_seven_tools_with_closed_schemas() {
        let tools = catalog().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert_eq!(
            names,
            [
                "list_services",
                "describe_service",
                "find_endpoint",
                "get_spec",
                "get_graph",
                "diff_versions",
                "blame"
            ]
        );
        for tool in &tools {
            assert_eq!(tool.input_schema["type"], "object", "{}", tool.name);
            assert_eq!(
                tool.input_schema["additionalProperties"],
                Value::Bool(false),
                "{} must refuse unknown parameters",
                tool.name
            );
            assert!(
                tool.description.as_ref().is_some_and(|d| d.len() > 200),
                "{} needs a real description",
                tool.name
            );
            assert_eq!(
                tool.annotations.as_ref().and_then(|a| a.read_only_hint),
                Some(true)
            );
        }
        let get_spec = &tools[3];
        let required = get_spec.input_schema["required"].as_array().unwrap();
        assert_eq!(required, &[json!("service"), json!("api_type")]);
        assert_eq!(
            get_spec.input_schema["properties"]["api_type"]["enum"],
            json!(["openapi", "asyncapi", "proto"])
        );
    }

    #[test]
    fn parameters_refuse_unknown_fields_and_bad_versions() {
        let err = params::<GetSpecParams>(
            "get_spec",
            json!({ "service": "a", "api_type": "openapi", "bogus": 1 }),
        )
        .unwrap_err();
        assert!(err.0.contains("unknown field"), "{}", err.0);
        let err =
            params::<GetSpecParams>("get_spec", json!({ "service": "a", "api_type": "soap" }))
                .unwrap_err();
        assert!(
            err.0.contains("invalid parameters for get_spec"),
            "{}",
            err.0
        );
        assert!(version("one").is_err());
        assert_eq!(version("v2").unwrap().to_string(), "2.0.0");
        assert!(endpoint(Some("/x"), None).is_err());
        assert_eq!(endpoint(None, None).unwrap(), None);
    }
}
