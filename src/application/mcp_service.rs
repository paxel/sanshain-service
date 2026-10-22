//! The catalog answers an AI client gets over MCP (https://paxel.github.io/sanshain-service/developer/mcp.html): services,
//! one service in depth, endpoint search, stored documents, the graph, diffs
//! and blame. Only catalog data — never users, roles, settings, auth
//! configuration, audit or observability data.
//!
//! Everything here is scope-aware the way the reports are: `dev` is the
//! accumulated activity, `main` and a sanshain-branch are pin sets, optionally
//! at a past instant. The shapes are plain data; the transport (rate limit,
//! size budget, JSON framing) is the presentation layer's.

use super::report_service::{ReportScope, generate_scoped_report, get_graph_scope, get_periphery};
use super::spec_service::{get_endpoint_yaml, get_full_spec, list_producer_endpoints};
use crate::domain::models::*;
use crate::domain::ports::SpecRepository;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use tracing::instrument;

/// The selector a tool was computed for, echoed on every answer so a scoped
/// view can never masquerade as the whole picture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScopeStamp {
    pub scope: String,
    pub at: Option<String>,
}

impl ScopeStamp {
    /// The stored catalog as it is now: what the unscoped tools describe.
    pub fn dev() -> Self {
        ScopeStamp {
            scope: "dev".to_string(),
            at: None,
        }
    }
}

/// `scope` (`dev` default, `main`, or a sanshain-branch name) and an optional
/// instant into the selector grammar the reports speak (`main@<instant>`).
/// The dev scope has no timeline, so `dev` with an instant is refused.
pub fn parse_scope(
    scope: Option<&str>,
    at: Option<&str>,
) -> Result<(ReportScope, ScopeStamp), AppError> {
    let name = scope.unwrap_or("dev");
    if name.is_empty() {
        return Err(AppError::BadRequest("scope must not be empty".to_string()));
    }
    let selector = match at {
        Some(at) if !at.is_empty() => format!("{name}@{at}"),
        _ => name.to_string(),
    };
    let parsed = ReportScope::parse(Some(&selector))?;
    Ok((
        parsed,
        ScopeStamp {
            scope: name.to_string(),
            at: at.filter(|a| !a.is_empty()).map(str::to_string),
        },
    ))
}

/// One version line of a Producer as a scope carries it.
#[derive(Debug, Clone, Serialize)]
pub struct ServiceVersionLine {
    pub api_type: ApiType,
    pub version: SemVer,
    pub stability: Option<Stability>,
    pub last_provided_at: Option<String>,
}

/// A Producer in the scope, with what the listing needs to pick one.
#[derive(Debug, Clone, Serialize)]
pub struct ProducerEntry {
    pub name: String,
    pub domain: Option<String>,
    pub icon: Option<String>,
    pub api_types: Vec<ApiType>,
    pub versions: Vec<ServiceVersionLine>,
    /// Endpoints across the scope's version lines.
    pub endpoints: usize,
    /// Distinct Consumers requiring it in the scope.
    pub consumers: usize,
    /// Nobody in the scope requires it.
    pub unrequired: bool,
}

/// A Consumer in the scope: who it requires from, and how much.
#[derive(Debug, Clone, Serialize)]
pub struct ConsumerEntry {
    pub name: String,
    /// Endpoints it requires in the scope.
    pub requires: usize,
    pub producers: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceList {
    #[serde(flatten)]
    pub stamp: ScopeStamp,
    pub producers: Vec<ProducerEntry>,
    pub consumers: Vec<ConsumerEntry>,
}

/// Tool `list_services`: every Producer and Consumer in the scope.
#[instrument(skip_all)]
pub async fn list_services(
    repo: &impl SpecRepository,
    scope: Option<&str>,
) -> Result<ServiceList, AppError> {
    let (scope, stamp) = parse_scope(scope, None)?;
    let graph_scope = get_graph_scope(repo, &scope).await?;
    let report = generate_scoped_report(repo, scope).await?;
    let metadata: BTreeMap<String, (Option<String>, Option<String>)> = repo
        .list_producers_detailed()
        .await?
        .into_iter()
        .map(|p| (p.name, (p.domain, p.icon)))
        .collect();
    let endpoint_counts: BTreeMap<(String, ApiType, SemVer), i64> = repo
        .list_all_spec_versions()
        .await?
        .into_iter()
        .map(|(name, meta, count)| ((name, meta.api_type, meta.version), count))
        .collect();

    let mut consumers_of: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut requires_of: BTreeMap<&str, (usize, BTreeSet<&str>)> = BTreeMap::new();
    for edge in &report.dependency_graph {
        consumers_of
            .entry(edge.service.as_str())
            .or_default()
            .insert(edge.client.as_str());
        let entry = requires_of.entry(edge.client.as_str()).or_default();
        entry.0 += 1;
        entry.1.insert(edge.service.as_str());
    }
    for sub in &report.harvested_subscriptions {
        if let Some(owner) = &sub.owner {
            consumers_of
                .entry(owner.as_str())
                .or_default()
                .insert(sub.client.as_str());
            let entry = requires_of.entry(sub.client.as_str()).or_default();
            entry.0 += 1;
            entry.1.insert(owner.as_str());
        }
    }

    let producers = graph_scope
        .producers
        .iter()
        .map(|p| {
            let (domain, icon) = metadata.get(&p.name).cloned().unwrap_or_default();
            let consumers = consumers_of.get(p.name.as_str()).map_or(0, BTreeSet::len);
            ProducerEntry {
                name: p.name.clone(),
                domain,
                icon,
                api_types: p.versions.iter().map(|v| v.api_type).collect(),
                versions: p
                    .versions
                    .iter()
                    .map(|v| ServiceVersionLine {
                        api_type: v.api_type,
                        version: v.version,
                        stability: v.stability,
                        last_provided_at: v.last_provided_at.clone(),
                    })
                    .collect(),
                endpoints: p
                    .versions
                    .iter()
                    .map(|v| {
                        endpoint_counts
                            .get(&(p.name.clone(), v.api_type, v.version))
                            .copied()
                            .unwrap_or(0) as usize
                    })
                    .sum(),
                consumers,
                unrequired: consumers == 0,
            }
        })
        .collect();
    let consumers = requires_of
        .into_iter()
        .map(|(name, (requires, producers))| ConsumerEntry {
            name: name.to_string(),
            requires,
            producers: producers.into_iter().map(str::to_string).collect(),
        })
        .collect();
    Ok(ServiceList {
        stamp,
        producers,
        consumers,
    })
}

/// A version line of the described service, with whether a newer version of
/// the same line exists.
#[derive(Debug, Clone, Serialize)]
pub struct DescribedVersion {
    pub api_type: ApiType,
    pub version: SemVer,
    pub stability: Stability,
    pub provided_by: String,
    pub last_provided_at: String,
    pub endpoint_count: i64,
    pub superseded: bool,
    /// What the build behind it said about why (empty when nothing).
    pub origin: VersionOrigin,
}

#[derive(Debug, Clone, Serialize)]
pub struct DescribedEndpoint {
    pub api_type: ApiType,
    pub version: SemVer,
    pub method: String,
    pub path: String,
    pub deprecated: bool,
    /// What the Producer wrote about it, one line, when it wrote anything.
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EndpointRef {
    pub method: String,
    pub path: String,
}

/// A Consumer of the described service and what it pins.
#[derive(Debug, Clone, Serialize)]
pub struct DescribedConsumer {
    pub name: String,
    pub api_type: ApiType,
    pub version: Option<SemVer>,
    pub endpoints: Vec<EndpointRef>,
}

/// What the described service requires from others.
#[derive(Debug, Clone, Serialize)]
pub struct DescribedRequirement {
    pub producer: String,
    pub api_type: ApiType,
    pub version: Option<SemVer>,
    pub method: String,
    pub path: String,
}

/// A Peripheral Resource the service depends on, and who shares it.
#[derive(Debug, Clone, Serialize)]
pub struct DescribedResource {
    pub resource: String,
    pub shared_with: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceDescription {
    #[serde(flatten)]
    pub stamp: ScopeStamp,
    pub name: String,
    pub domain: Option<String>,
    pub icon: Option<String>,
    pub is_producer: bool,
    pub is_consumer: bool,
    pub versions: Vec<DescribedVersion>,
    /// The endpoints of the latest version of each line.
    pub endpoints: Vec<DescribedEndpoint>,
    pub consumers: Vec<DescribedConsumer>,
    pub requires: Vec<DescribedRequirement>,
    pub periphery: Vec<DescribedResource>,
    /// Owner, description, source location and links from the service's
    /// catalog-info (#48); `None` when no push described it.
    pub context: Option<SystemContext>,
}

/// Tool `describe_service`: everything about one Producer or Consumer.
#[instrument(skip_all)]
pub async fn describe_service(
    repo: &impl SpecRepository,
    name: &str,
    scope: Option<&str>,
) -> Result<ServiceDescription, AppError> {
    let (scope, stamp) = parse_scope(scope, None)?;
    let is_producer = repo.find_service(name).await?.is_some();
    let is_consumer = repo.list_consumers().await?.iter().any(|c| c == name);
    if !is_producer && !is_consumer {
        return Err(AppError::NotFound(format!(
            "no service '{name}': it is neither a Producer nor a Consumer — list_services names the known ones"
        )));
    }
    let report = generate_scoped_report(repo, scope).await?;
    let periphery = get_periphery(repo, &parse_scope(stamp.scope.as_str().into(), None)?.0).await?;

    let (domain, icon) = repo
        .list_producers_detailed()
        .await?
        .into_iter()
        .find(|p| p.name == name)
        .map(|p| (p.domain, p.icon))
        .unwrap_or_default();

    let mut versions = Vec::new();
    let mut endpoints = Vec::new();
    if is_producer {
        let all = super::admin_service::list_producer_versions(repo, name, None).await?;
        let mut latest: BTreeMap<ApiType, SemVer> = BTreeMap::new();
        for v in &all {
            let slot = latest.entry(v.api_type).or_insert(v.version);
            if v.version > *slot {
                *slot = v.version;
            }
        }
        for v in &all {
            versions.push(DescribedVersion {
                api_type: v.api_type,
                version: v.version,
                stability: v.stability,
                provided_by: v.provided_by.clone(),
                last_provided_at: v.updated_at.clone(),
                endpoint_count: v.endpoint_count,
                superseded: latest.get(&v.api_type).is_some_and(|l| *l > v.version),
                origin: v.origin.clone(),
            });
        }
        for (api_type, version) in latest {
            for e in list_producer_endpoints(repo, name, api_type, version).await? {
                endpoints.push(DescribedEndpoint {
                    api_type,
                    version,
                    summary: crate::descriptions::endpoint_summary(api_type, &e.yaml_content),
                    method: e.method,
                    path: e.path,
                    deprecated: e.deprecated,
                });
            }
        }
    }

    let mut consumers: BTreeMap<(String, ApiType, Option<SemVer>), Vec<EndpointRef>> =
        BTreeMap::new();
    let mut requires = Vec::new();
    for edge in &report.dependency_graph {
        if edge.service == name {
            consumers
                .entry((edge.client.clone(), edge.api_type, Some(edge.version)))
                .or_default()
                .push(EndpointRef {
                    method: edge.method.clone(),
                    path: edge.path.clone(),
                });
        }
        if edge.client == name {
            requires.push(DescribedRequirement {
                producer: edge.service.clone(),
                api_type: edge.api_type,
                version: Some(edge.version),
                method: edge.method.clone(),
                path: edge.path.clone(),
            });
        }
    }
    for sub in &report.harvested_subscriptions {
        if sub.owner.as_deref() == Some(name) {
            consumers
                .entry((sub.client.clone(), ApiType::AsyncApi, None))
                .or_default()
                .push(EndpointRef {
                    method: "SUBSCRIBE".to_string(),
                    path: sub.channel.clone(),
                });
        }
        if sub.client == name
            && let Some(owner) = &sub.owner
        {
            requires.push(DescribedRequirement {
                producer: owner.clone(),
                api_type: ApiType::AsyncApi,
                version: None,
                method: "SUBSCRIBE".to_string(),
                path: sub.channel.clone(),
            });
        }
    }
    let consumers = consumers
        .into_iter()
        .map(
            |((consumer, api_type, version), endpoints)| DescribedConsumer {
                name: consumer,
                api_type,
                version,
                endpoints,
            },
        )
        .collect();

    let mine: Vec<&PeripheralDependency> = periphery.iter().filter(|p| p.system == name).collect();
    let periphery = mine
        .iter()
        .map(|p| DescribedResource {
            resource: p.resource.clone(),
            shared_with: periphery
                .iter()
                .filter(|o| o.resource == p.resource && o.system != name)
                .map(|o| o.system.clone())
                .collect(),
        })
        .collect();

    Ok(ServiceDescription {
        stamp,
        name: name.to_string(),
        domain,
        icon,
        is_producer,
        is_consumer,
        versions,
        endpoints,
        consumers,
        requires,
        periphery,
        context: repo.get_system_context(name).await?,
    })
}

/// Tool `find_endpoint`: endpoints across all Producers whose path, channel
/// or method name contains `query` (case-insensitive; `*` is a wildcard).
#[instrument(skip_all)]
pub async fn find_endpoint(
    repo: &impl SpecRepository,
    search: &EndpointSearch,
) -> Result<Vec<EndpointSearchHit>, AppError> {
    if search.query.trim().is_empty() {
        return Err(AppError::BadRequest(
            "query must not be empty — give part of a path, channel or method name".to_string(),
        ));
    }
    Ok(repo
        .search_endpoints(search)
        .await?
        .into_iter()
        .map(|mut hit| {
            hit.summary = crate::descriptions::endpoint_summary(hit.api_type, &hit.yaml_content);
            hit
        })
        .collect())
}

/// One endpoint's structure for `endpoint_structure`: the latest version of
/// the line unless `version` names one.
pub async fn endpoint_structure(
    repo: &impl SpecRepository,
    service: &str,
    api_type: ApiType,
    version: Option<SemVer>,
    endpoint: (&str, &str),
    with_examples: bool,
) -> Result<super::structure_service::StructureView, AppError> {
    let version = match version {
        Some(v) => v,
        None => latest_version(repo, service, api_type).await?,
    };
    super::structure_service::structure_version(
        repo,
        service,
        super::structure_service::StructureRequest {
            api_type,
            version,
            endpoint: Some(endpoint),
            with_examples,
        },
    )
    .await
}

/// The highest version of a Producer's line, for calls that omit the version.
async fn latest_version(
    repo: &impl SpecRepository,
    producername: &str,
    api_type: ApiType,
) -> Result<SemVer, AppError> {
    let service_id = repo.find_service(producername).await?.ok_or_else(|| {
        AppError::NotFound(format!(
            "no Producer '{producername}' — list_services names the known ones"
        ))
    })?;
    repo.list_spec_versions(service_id)
        .await?
        .into_iter()
        .filter(|v| v.api_type == api_type)
        .map(|v| v.version)
        .max()
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "Producer '{producername}' has no {} versions",
                api_type.as_str()
            ))
        })
}

/// A stored document or endpoint snippet, with its line count so a client
/// can page through it.
#[derive(Debug, Clone, Serialize)]
pub struct SpecText {
    #[serde(flatten)]
    pub stamp: ScopeStamp,
    pub service: String,
    pub api_type: ApiType,
    pub version: SemVer,
    pub stability: Stability,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    pub text: String,
}

/// Tool `get_spec`: the stored document of a version (the line's latest when
/// `version` is omitted), or one endpoint's snippet with `path` and `method`.
#[instrument(skip_all)]
pub async fn get_spec_text(
    repo: &impl SpecRepository,
    service: &str,
    api_type: ApiType,
    version: Option<SemVer>,
    endpoint: Option<(&str, &str)>,
) -> Result<SpecText, AppError> {
    let version = match version {
        Some(v) => v,
        None => latest_version(repo, service, api_type).await?,
    };
    let (_sid, entry) =
        super::require_service::resolve_pin(repo, service, api_type, version).await?;
    let text = match endpoint {
        None => get_full_spec(repo, service, api_type, version).await?,
        Some((path, method)) => {
            let view = get_endpoint_yaml(repo, service, version, api_type, path, method).await?;
            view.yaml.ok_or_else(|| {
                AppError::NotFound(format!(
                    "{} version {} of '{}' has no endpoint {} {} — describe_service lists its endpoints",
                    api_type.as_str(),
                    version,
                    service,
                    method.to_uppercase(),
                    path
                ))
            })?
        }
    };
    Ok(SpecText {
        stamp: ScopeStamp::dev(),
        service: service.to_string(),
        api_type,
        version,
        stability: entry.stability,
        path: endpoint.map(|(p, _)| p.to_string()),
        method: endpoint.map(|(_, m)| m.to_string()),
        text,
    })
}

/// The endpoints (method and path) of a version, for an over-budget summary.
#[instrument(skip_all)]
pub async fn endpoint_refs(
    repo: &impl SpecRepository,
    service: &str,
    api_type: ApiType,
    version: SemVer,
) -> Result<Vec<EndpointRef>, AppError> {
    Ok(list_producer_endpoints(repo, service, api_type, version)
        .await?
        .into_iter()
        .map(|e| EndpointRef {
            method: e.method,
            path: e.path,
        })
        .collect())
}

/// What a graph node is in the scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Producer,
    Consumer,
    Both,
    /// Referenced by an edge but not provided in the scope.
    Missing,
    Resource,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphNode {
    pub service: String,
    pub kind: NodeKind,
    pub provides: Vec<ServiceVersionLine>,
    pub requires: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub api_type: Option<ApiType>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub version: Option<SemVer>,
    pub stability: Option<Stability>,
    /// The pin is below the line's highest version in the scope.
    pub outdated: bool,
    /// The pin is a snapshot below the line's highest GA.
    pub superseded: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub harvested: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub peripheral: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Graph {
    #[serde(flatten)]
    pub stamp: ScopeStamp,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

/// How `get_graph` narrows the picture.
#[derive(Debug, Clone, Default)]
pub struct GraphFilter {
    pub service: Option<String>,
    pub api_type: Option<ApiType>,
    pub include_periphery: bool,
}

/// Tool `get_graph`: the dependency graph of a scope as nodes and edges.
#[instrument(skip_all)]
pub async fn get_graph(
    repo: &impl SpecRepository,
    scope: Option<&str>,
    at: Option<&str>,
    filter: &GraphFilter,
) -> Result<Graph, AppError> {
    let (scope, stamp) = parse_scope(scope, at)?;
    let graph_scope = get_graph_scope(repo, &scope).await?;
    let periphery = if filter.include_periphery {
        get_periphery(repo, &scope).await?
    } else {
        Vec::new()
    };
    let report = generate_scoped_report(repo, scope).await?;

    // The line's highest version and highest GA in the scope, per producer.
    let mut highest: BTreeMap<(String, ApiType), SemVer> = BTreeMap::new();
    let mut highest_ga: BTreeMap<(String, ApiType), SemVer> = BTreeMap::new();
    for p in &graph_scope.producers {
        for v in &p.versions {
            let key = (p.name.clone(), v.api_type);
            let slot = highest.entry(key.clone()).or_insert(v.version);
            if v.version > *slot {
                *slot = v.version;
            }
            if v.stability == Some(Stability::Ga) {
                let slot = highest_ga.entry(key).or_insert(v.version);
                if v.version > *slot {
                    *slot = v.version;
                }
            }
        }
    }

    let mut edges: Vec<GraphEdge> = report
        .dependency_graph
        .iter()
        .filter(|d| filter.api_type.is_none_or(|t| t == d.api_type))
        .map(|d| {
            let key = (d.service.clone(), d.api_type);
            GraphEdge {
                from: d.client.clone(),
                to: d.service.clone(),
                api_type: Some(d.api_type),
                method: Some(d.method.clone()),
                path: Some(d.path.clone()),
                version: Some(d.version),
                stability: Some(d.stability),
                outdated: highest.get(&key).is_some_and(|h| *h > d.version),
                superseded: d.stability == Stability::Snapshot
                    && highest_ga.get(&key).is_some_and(|g| *g > d.version),
                harvested: false,
                peripheral: false,
            }
        })
        .collect();
    if filter.api_type.is_none_or(|t| t == ApiType::AsyncApi) {
        for sub in &report.harvested_subscriptions {
            if let Some(owner) = &sub.owner {
                edges.push(GraphEdge {
                    from: sub.client.clone(),
                    to: owner.clone(),
                    api_type: Some(ApiType::AsyncApi),
                    method: Some("SUBSCRIBE".to_string()),
                    path: Some(sub.channel.clone()),
                    version: None,
                    stability: None,
                    outdated: false,
                    superseded: false,
                    harvested: true,
                    peripheral: false,
                });
            }
        }
    }
    for p in &periphery {
        edges.push(GraphEdge {
            from: p.system.clone(),
            to: p.resource.clone(),
            api_type: None,
            method: None,
            path: None,
            version: None,
            stability: None,
            outdated: false,
            superseded: false,
            harvested: false,
            peripheral: true,
        });
    }

    let producers: BTreeSet<String> = graph_scope
        .producers
        .iter()
        .map(|p| p.name.clone())
        .collect();
    let resources: BTreeSet<String> = periphery.iter().map(|p| p.resource.clone()).collect();
    let consumers: BTreeSet<String> = edges
        .iter()
        .filter(|e| !e.peripheral)
        .map(|e| e.from.clone())
        .collect();
    let mut names: BTreeSet<String> = producers.clone();
    names.extend(edges.iter().map(|e| e.from.clone()));
    names.extend(edges.iter().map(|e| e.to.clone()));
    if filter.api_type.is_some() {
        // A producer that has no edge of the asked type still appears if it
        // provides that type; others drop out with their edges.
        names.retain(|n| {
            resources.contains(n)
                || edges.iter().any(|e| e.from == *n || e.to == *n)
                || graph_scope.producers.iter().any(|p| {
                    p.name == *n
                        && p.versions
                            .iter()
                            .any(|v| Some(v.api_type) == filter.api_type)
                })
        });
    }
    if let Some(focus) = &filter.service {
        if !names.contains(focus) {
            return Err(AppError::NotFound(format!(
                "no service '{focus}' in scope {} — list_services names the known ones",
                stamp.scope
            )));
        }
        edges.retain(|e| e.from == *focus || e.to == *focus);
        let mut keep: BTreeSet<String> = BTreeSet::new();
        keep.insert(focus.clone());
        for e in &edges {
            keep.insert(e.from.clone());
            keep.insert(e.to.clone());
        }
        names = keep;
    }

    let nodes = names
        .into_iter()
        .map(|n| {
            let kind = if resources.contains(&n) {
                NodeKind::Resource
            } else {
                match (producers.contains(&n), consumers.contains(&n)) {
                    (true, true) => NodeKind::Both,
                    (true, false) => NodeKind::Producer,
                    (false, true) => NodeKind::Consumer,
                    (false, false) => NodeKind::Missing,
                }
            };
            let provides = graph_scope
                .producers
                .iter()
                .filter(|p| p.name == n)
                .flat_map(|p| p.versions.iter())
                .filter(|v| filter.api_type.is_none_or(|t| t == v.api_type))
                .map(|v| ServiceVersionLine {
                    api_type: v.api_type,
                    version: v.version,
                    stability: v.stability,
                    last_provided_at: v.last_provided_at.clone(),
                })
                .collect();
            let requires: BTreeSet<String> = edges
                .iter()
                .filter(|e| e.from == n)
                .map(|e| e.to.clone())
                .collect();
            GraphNode {
                service: n,
                kind,
                provides,
                requires: requires.into_iter().collect(),
            }
        })
        .collect();
    Ok(Graph {
        stamp,
        nodes,
        edges,
    })
}

/// Counts per node kind, for an over-budget graph answer.
pub fn graph_counts(graph: &Graph) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for node in &graph.nodes {
        let key = match node.kind {
            NodeKind::Producer => "producer",
            NodeKind::Consumer => "consumer",
            NodeKind::Both => "both",
            NodeKind::Missing => "missing",
            NodeKind::Resource => "resource",
        };
        *counts.entry(key.to_string()).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::mock_repo::MockRepo;
    use crate::domain::ports::{RecordDependencyParams, UpsertSpecVersion};

    async fn seed() -> MockRepo {
        let repo = MockRepo::new();
        let sid = repo.ensure_service("orders").await.unwrap();
        let endpoints = vec![
            EndpointRecord {
                id: None,
                api_type: ApiType::OpenApi,
                path: "/orders/{id}".to_string(),
                normalized_path: "/orders/{}".to_string(),
                method: "GET".to_string(),
                yaml_content: "get: {}".to_string(),
                deprecated: false,
            },
            EndpointRecord {
                id: None,
                api_type: ApiType::OpenApi,
                path: "/orders".to_string(),
                normalized_path: "/orders".to_string(),
                method: "POST".to_string(),
                yaml_content: "post: {}".to_string(),
                deprecated: true,
            },
        ];
        for version in ["1.0.0", "1.1.0"] {
            let content = format!("openapi: 3.0.0\ninfo:\n  version: {version}\n");
            repo.upsert_spec_version(UpsertSpecVersion {
                outbox: Vec::new(),
                service_id: sid,
                api_type: ApiType::OpenApi,
                version: version.parse().unwrap(),
                stability: Stability::Ga,
                content: &content,
                content_hash: version,
                provided_by: "ci",
                expected_prior_hash: None,
                now_iso: "2026-10-01T00:00:00Z",
                endpoints: endpoints.clone(),
            })
            .await
            .unwrap();
        }
        let v1 = repo
            .find_spec_version(sid, ApiType::OpenApi, "1.0.0".parse().unwrap())
            .await
            .unwrap()
            .unwrap();
        let cid = repo.ensure_client("web").await.unwrap();
        repo.record_dependency(RecordDependencyParams {
            client_id: cid,
            spec_version_id: v1.id,
            api_type: ApiType::OpenApi,
            path: "/orders/{id}",
            normalized_path: "/orders/{}",
            method: "GET",
        })
        .await
        .unwrap();
        repo
    }

    #[test]
    fn scope_parsing_stamps_what_was_asked() {
        let (_, stamp) = parse_scope(None, None).unwrap();
        assert_eq!(stamp, ScopeStamp::dev());
        let (scope, stamp) = parse_scope(Some("main"), Some("2026-01-01T00:00:00Z")).unwrap();
        assert!(matches!(scope, ReportScope::Main { at: Some(_) }));
        assert_eq!(stamp.at.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert!(parse_scope(Some("dev"), Some("2026-01-01T00:00:00Z")).is_err());
        assert!(parse_scope(Some(""), None).is_err());
    }

    #[tokio::test]
    async fn list_services_counts_endpoints_and_consumers() {
        let repo = seed().await;
        let list = list_services(&repo, None).await.unwrap();
        assert_eq!(list.stamp, ScopeStamp::dev());
        assert_eq!(list.producers.len(), 1);
        let orders = &list.producers[0];
        assert_eq!(orders.name, "orders");
        assert_eq!(orders.versions.len(), 1, "one line, its highest version");
        assert_eq!(orders.versions[0].version.to_string(), "1.1.0");
        assert_eq!(orders.endpoints, 2);
        assert_eq!(orders.consumers, 1);
        assert!(!orders.unrequired);
        assert_eq!(list.consumers.len(), 1);
        assert_eq!(list.consumers[0].name, "web");
        assert_eq!(list.consumers[0].producers, vec!["orders"]);
    }

    #[tokio::test]
    async fn describe_service_reports_lines_endpoints_and_consumers() {
        let repo = seed().await;
        let d = describe_service(&repo, "orders", None).await.unwrap();
        assert!(d.is_producer && !d.is_consumer);
        assert_eq!(d.versions.len(), 2);
        let v1 = d
            .versions
            .iter()
            .find(|v| v.version.to_string() == "1.0.0")
            .unwrap();
        assert!(v1.superseded);
        let v11 = d
            .versions
            .iter()
            .find(|v| v.version.to_string() == "1.1.0")
            .unwrap();
        assert!(!v11.superseded);
        assert_eq!(d.endpoints.len(), 2);
        assert!(d.endpoints.iter().all(|e| e.version.to_string() == "1.1.0"));
        assert_eq!(d.consumers.len(), 1);
        assert_eq!(d.consumers[0].name, "web");
        assert_eq!(d.consumers[0].endpoints[0].path, "/orders/{id}");

        let c = describe_service(&repo, "web", None).await.unwrap();
        assert!(c.is_consumer && !c.is_producer);
        assert_eq!(c.requires.len(), 1);
        assert_eq!(c.requires[0].producer, "orders");

        let err = describe_service(&repo, "nobody", None).await.unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn find_endpoint_refuses_an_empty_query() {
        let repo = seed().await;
        let err = find_endpoint(
            &repo,
            &EndpointSearch {
                query: "  ".to_string(),
                method: None,
                api_type: None,
                all_versions: false,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
        let hits = find_endpoint(
            &repo,
            &EndpointSearch {
                query: "ORD*".to_string(),
                method: Some("get".to_string()),
                api_type: None,
                all_versions: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].version.to_string(), "1.1.0");
        assert_eq!(hits[0].consumer_count, 0, "the pin is on 1.0.0");
    }

    #[tokio::test]
    async fn get_spec_text_defaults_to_the_latest_version_and_names_missing_endpoints() {
        let repo = seed().await;
        let doc = get_spec_text(&repo, "orders", ApiType::OpenApi, None, None)
            .await
            .unwrap();
        assert_eq!(doc.version.to_string(), "1.1.0");
        assert!(doc.text.contains("version: 1.1.0"));
        let snippet = get_spec_text(
            &repo,
            "orders",
            ApiType::OpenApi,
            Some("1.0.0".parse().unwrap()),
            Some(("/orders", "post")),
        )
        .await
        .unwrap();
        assert_eq!(snippet.text, "post: {}");
        let err = get_spec_text(
            &repo,
            "orders",
            ApiType::OpenApi,
            None,
            Some(("/nope", "get")),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
        let err = get_spec_text(&repo, "orders", ApiType::Proto, None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn get_graph_flags_outdated_pins_and_narrows_to_a_service() {
        let repo = seed().await;
        let graph = get_graph(&repo, None, None, &GraphFilter::default())
            .await
            .unwrap();
        assert_eq!(graph.edges.len(), 1);
        assert!(graph.edges[0].outdated, "pinned 1.0.0 while 1.1.0 exists");
        assert!(!graph.edges[0].superseded, "a GA pin is never superseded");
        let kinds: BTreeMap<&str, NodeKind> = graph
            .nodes
            .iter()
            .map(|n| (n.service.as_str(), n.kind))
            .collect();
        assert_eq!(kinds["orders"], NodeKind::Producer);
        assert_eq!(kinds["web"], NodeKind::Consumer);
        assert_eq!(graph_counts(&graph)["consumer"], 1);

        let narrowed = get_graph(
            &repo,
            None,
            None,
            &GraphFilter {
                service: Some("web".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(narrowed.nodes.len(), 2);
        let err = get_graph(
            &repo,
            None,
            None,
            &GraphFilter {
                service: Some("ghost".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));

        let proto_only = get_graph(
            &repo,
            None,
            None,
            &GraphFilter {
                api_type: Some(ApiType::Proto),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(proto_only.edges.is_empty());
        assert!(proto_only.nodes.is_empty());
    }
}
