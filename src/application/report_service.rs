use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use crate::domain::models::*;
use crate::domain::ports::SpecRepository;
use tracing::instrument;
use crate::openapi;

#[instrument(skip_all)]
pub async fn generate_report(
    repo: &impl SpecRepository,
    branch: &str,
) -> Result<DependencyReport, AppError> {
    let mut report = repo.get_report(branch).await?;
    report.service_tags = repo.get_all_service_tags().await?;
    Ok(report)
}

#[instrument(skip_all)]
pub async fn generate_merged_report(
    repo: &impl SpecRepository,
    branch: &str,
    target: &str,
) -> Result<MergedDependencyReport, AppError> {
    let branch_report = generate_report(repo, branch).await?;
    let target_report = generate_report(repo, target).await?;

    // Build sets of (client, service, api_type, path, method) for dedup
    type DepKey = (String, String, String, String, String);
    let branch_deps: HashSet<DepKey> = branch_report
        .dependency_graph
        .iter()
        .map(|d| {
            (
                d.client.clone(),
                d.service.clone(),
                format!("{:?}", d.api_type),
                d.path.clone(),
                d.method.clone(),
            )
        })
        .collect();
    let target_deps: HashSet<DepKey> = target_report
        .dependency_graph
        .iter()
        .map(|d| {
            (
                d.client.clone(),
                d.service.clone(),
                format!("{:?}", d.api_type),
                d.path.clone(),
                d.method.clone(),
            )
        })
        .collect();

    let mut merged_graph = Vec::new();

    // Branch deps
    for dep in &branch_report.dependency_graph {
        let key = (
            dep.client.clone(),
            dep.service.clone(),
            format!("{:?}", dep.api_type),
            dep.path.clone(),
            dep.method.clone(),
        );
        let source = if target_deps.contains(&key) {
            NodeSource::Both
        } else {
            NodeSource::Branch
        };
        merged_graph.push(MergedDependencyInfo {
            api_type: dep.api_type,
            client: dep.client.clone(),
            service: dep.service.clone(),
            path: dep.path.clone(),
            method: dep.method.clone(),
            source,
        });
    }

    // Target-only deps
    for dep in &target_report.dependency_graph {
        let key = (
            dep.client.clone(),
            dep.service.clone(),
            format!("{:?}", dep.api_type),
            dep.path.clone(),
            dep.method.clone(),
        );
        if !branch_deps.contains(&key) {
            merged_graph.push(MergedDependencyInfo {
                api_type: dep.api_type,
                client: dep.client.clone(),
                service: dep.service.clone(),
                path: dep.path.clone(),
                method: dep.method.clone(),
                source: NodeSource::Target,
            });
        }
    }

    // Build node_sources: collect all service/client names and determine source
    let mut branch_nodes: HashSet<String> = HashSet::new();
    let mut target_nodes: HashSet<String> = HashSet::new();

    for dep in &branch_report.dependency_graph {
        branch_nodes.insert(dep.client.clone());
        branch_nodes.insert(dep.service.clone());
    }
    for ep in &branch_report.unused_endpoints {
        branch_nodes.insert(ep.service.clone());
    }

    for dep in &target_report.dependency_graph {
        target_nodes.insert(dep.client.clone());
        target_nodes.insert(dep.service.clone());
    }
    for ep in &target_report.unused_endpoints {
        target_nodes.insert(ep.service.clone());
    }

    let all_nodes: HashSet<&String> = branch_nodes.iter().chain(target_nodes.iter()).collect();
    let mut node_sources = HashMap::new();
    for name in all_nodes {
        let in_branch = branch_nodes.contains(name);
        let in_target = target_nodes.contains(name);
        let source = match (in_branch, in_target) {
            (true, true) => NodeSource::Both,
            (true, false) => NodeSource::Branch,
            (false, true) => NodeSource::Target,
            _ => NodeSource::Both,
        };
        node_sources.insert(name.clone(), source);
    }

    // Detect conflicts: endpoints that exist on both branches with incompatible definitions
    let mut conflicts = Vec::new();

    // Collect (service, api_type, path, method) from both branch endpoints
    type EpKey = (String, String, String, String);
    let mut branch_endpoints: HashMap<EpKey, ()> = HashMap::new();
    for dep in &branch_report.dependency_graph {
        branch_endpoints.insert(
            (
                dep.service.clone(),
                format!("{:?}", dep.api_type),
                dep.path.clone(),
                dep.method.clone(),
            ),
            (),
        );
    }
    for ep in &branch_report.unused_endpoints {
        branch_endpoints.insert(
            (
                ep.service.clone(),
                format!("{:?}", ep.api_type),
                ep.path.clone(),
                ep.method.clone(),
            ),
            (),
        );
    }

    let mut target_endpoint_keys: HashSet<EpKey> = HashSet::new();
    for dep in &target_report.dependency_graph {
        target_endpoint_keys.insert((
            dep.service.clone(),
            format!("{:?}", dep.api_type),
            dep.path.clone(),
            dep.method.clone(),
        ));
    }
    for ep in &target_report.unused_endpoints {
        target_endpoint_keys.insert((
            ep.service.clone(),
            format!("{:?}", ep.api_type),
            ep.path.clone(),
            ep.method.clone(),
        ));
    }

    // For endpoints present on both, fetch YAML and check compatibility
    for (service, api_type_str, path, method) in branch_endpoints.keys() {
        if !target_endpoint_keys.contains(&(
            service.clone(),
            api_type_str.clone(),
            path.clone(),
            method.clone(),
        )) {
            continue;
        }

        let api_type = ApiType::from_str(api_type_str).unwrap_or_default();
        if api_type != ApiType::OpenApi {
            continue; // Only OpenAPI has compatibility checking
        }

        // Find service ID and fetch endpoint YAML from both branches
        if let Ok(Some(sid)) = repo.find_service(service).await {
            let branch_ep = repo
                .find_endpoint(sid, branch, api_type, path, method)
                .await;
            let target_ep = repo
                .find_endpoint(sid, target, api_type, path, method)
                .await;

            if let (Ok(Some((_, branch_yaml))), Ok(Some((_, target_yaml)))) = (branch_ep, target_ep)
                && branch_yaml != target_yaml
                && let Err(reason) =
                    openapi::check_backward_compatibility(&target_yaml, &branch_yaml)
            {
                conflicts.push(ConflictInfo {
                    service: service.clone(),
                    api_type,
                    path: path.clone(),
                    method: method.clone(),
                    description: reason,
                });
            }
        }
    }

    // Merge service_tags from both
    let mut service_tags = branch_report.service_tags;
    for (k, v) in target_report.service_tags {
        service_tags.entry(k).or_insert(v);
    }

    Ok(MergedDependencyReport {
        branch: branch.to_string(),
        target: target.to_string(),
        dependency_graph: merged_graph,
        unused_endpoints: branch_report.unused_endpoints,
        missing_endpoints: branch_report.missing_endpoints,
        conflicts,
        service_tags,
        node_sources,
    })
}

pub fn render_report_markdown(report: &DependencyReport) -> String {
    let mut md = String::new();
    md.push_str(&format!(
        "# Sanshain Dependency Report: Branch `{}`\n\n",
        report.branch
    ));

    md.push_str("| Client | Service | Type | Path | Method |\n");
    md.push_str("| --- | --- | --- | --- | --- |\n");

    for dep in &report.dependency_graph {
        md.push_str(&format!(
            "| {} | {} | {:?} | `{}` | `{}` |\n",
            dep.client, dep.service, dep.api_type, dep.path, dep.method
        ));
    }

    md
}

pub fn render_isolation_report(report: &DependencyReport) -> String {
    use std::collections::{BTreeMap, BTreeSet};

    let mut md = String::new();
    md.push_str("# Service Isolation Report\n\n");

    if report.dependency_graph.is_empty() {
        md.push_str("No dependencies found.\n");
        return md;
    }

    // Collect unique connections: (source, target, protocol)
    let mut connections: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();

    for dep in &report.dependency_graph {
        match dep.api_type {
            ApiType::AsyncApi => {
                let kafka = "KAFKA".to_string();
                let protocol = "https".to_string();
                connections
                    .entry(dep.client.clone())
                    .or_default()
                    .insert((kafka.clone(), protocol.clone()));
                connections
                    .entry(dep.service.clone())
                    .or_default()
                    .insert((kafka, protocol));
            }
            ApiType::OpenApi | ApiType::Proto => {
                connections
                    .entry(dep.client.clone())
                    .or_default()
                    .insert((dep.service.clone(), "https".to_string()));
            }
        }
    }

    for (service, targets) in &connections {
        md.push_str(&format!("## Service: {}\n\n", service));
        md.push_str("| Target | Protocol | Port |\n");
        md.push_str("| --- | --- | --- |\n");
        for (target, protocol) in targets {
            md.push_str(&format!("| {} | {} | |\n", target, protocol));
        }
        md.push('\n');
    }

    md
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_dep(client: &str, service: &str, api_type: ApiType) -> DependencyInfo {
        DependencyInfo {
            client: client.to_string(),
            service: service.to_string(),
            api_type,
            path: "/test".to_string(),
            method: "GET".to_string(),
        }
    }

    fn make_report(deps: Vec<DependencyInfo>) -> DependencyReport {
        DependencyReport {
            branch: "main".to_string(),
            dependency_graph: deps,
            service_tags: std::collections::HashMap::new(),
            missing_endpoints: vec![],
            unused_endpoints: vec![],
        }
    }

    #[test]
    fn empty_report() {
        let report = make_report(vec![]);
        let result = render_isolation_report(&report);
        assert!(result.contains("No dependencies found."));
        assert!(!result.contains("| Target |"));
    }

    #[test]
    fn openapi_dependencies() {
        let report = make_report(vec![make_dep(
            "order-service",
            "user-service",
            ApiType::OpenApi,
        )]);
        let result = render_isolation_report(&report);
        assert!(result.contains("## Service: order-service"));
        assert!(result.contains("| user-service | https | |"));
    }

    #[test]
    fn asyncapi_dependencies_route_through_kafka() {
        let report = make_report(vec![make_dep(
            "order-service",
            "notification-service",
            ApiType::AsyncApi,
        )]);
        let result = render_isolation_report(&report);
        assert!(result.contains("## Service: order-service"));
        assert!(result.contains("## Service: notification-service"));
        assert!(result.contains("| KAFKA | https | |"));
        assert!(!result.contains("| notification-service |"));
        assert!(!result.contains("| order-service |"));
    }

    #[test]
    fn proto_dependencies() {
        let report = make_report(vec![make_dep("gateway", "auth-service", ApiType::Proto)]);
        let result = render_isolation_report(&report);
        assert!(result.contains("## Service: gateway"));
        assert!(result.contains("| auth-service | https | |"));
    }

    #[test]
    fn mixed_protocols() {
        let report = make_report(vec![
            make_dep("order-service", "user-service", ApiType::OpenApi),
            make_dep("order-service", "event-bus", ApiType::AsyncApi),
            make_dep("gateway", "auth-service", ApiType::Proto),
        ]);
        let result = render_isolation_report(&report);
        assert!(result.contains("## Service: order-service"));
        assert!(result.contains("| user-service | https | |"));
        assert!(result.contains("| KAFKA | https | |"));
        assert!(result.contains("## Service: gateway"));
        assert!(result.contains("| auth-service | https | |"));
        assert!(result.contains("## Service: event-bus"));
    }

    #[test]
    fn deduplication() {
        let report = make_report(vec![
            make_dep("order-service", "user-service", ApiType::OpenApi),
            make_dep("order-service", "user-service", ApiType::OpenApi),
        ]);
        let result = render_isolation_report(&report);
        let count = result.matches("| user-service | https | |").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn multiple_asyncapi_same_pair_deduplicated() {
        let report = make_report(vec![
            make_dep("svc-a", "svc-b", ApiType::AsyncApi),
            make_dep("svc-a", "svc-b", ApiType::AsyncApi),
        ]);
        let result = render_isolation_report(&report);
        let kafka_count = result.matches("| KAFKA | https | |").count();
        assert_eq!(kafka_count, 2);
    }
}
