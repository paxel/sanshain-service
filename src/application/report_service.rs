use crate::domain::models::*;
use crate::domain::ports::SpecRepository;
use tracing::instrument;

/// The graph a report describes (ADR-0005): the accumulated dev activity
/// (default), the current trunk pin set, or a sanshain-branch — optionally at
/// a past instant (`<branch>@<rfc3339>`).
pub enum ReportScope {
    Dev,
    Main,
    Branch { name: String, at: Option<String> },
}

impl ReportScope {
    /// `scope=dev|main|<branch>[@rfc3339]`. A trailing `@<instant>` is only
    /// split off when it parses as a date, so branch names containing `@`
    /// keep working.
    pub fn parse(raw: Option<&str>) -> ReportScope {
        match raw {
            None | Some("dev") => ReportScope::Dev,
            Some("main") => ReportScope::Main,
            Some(other) => {
                if let Some((name, at)) = other.rsplit_once('@')
                    && chrono::DateTime::parse_from_rfc3339(at).is_ok()
                {
                    return ReportScope::Branch {
                        name: name.to_string(),
                        at: Some(at.to_string()),
                    };
                }
                ReportScope::Branch {
                    name: other.to_string(),
                    at: None,
                }
            }
        }
    }
}

/// Project a pin set into the dependency-graph shape reports render. The
/// stability is looked up from the stored version line; a dangling pin
/// (deleted version) reports as GA — reports state pins, not resolutions.
async fn pins_as_dependency_graph(
    repo: &impl SpecRepository,
    pins: Vec<TrunkPinInfo>,
) -> Result<Vec<DependencyInfo>, AppError> {
    let mut graph = Vec::with_capacity(pins.len());
    for pin in pins {
        let stability = match repo.find_service(&pin.service).await? {
            Some(sid) => repo
                .find_spec_version(sid, pin.api_type, pin.version)
                .await?
                .map(|v| v.stability)
                .unwrap_or(Stability::Ga),
            None => Stability::Ga,
        };
        graph.push(DependencyInfo {
            api_type: pin.api_type,
            client: pin.client,
            service: pin.service,
            version: pin.version,
            stability,
            path: pin.path,
            method: pin.method,
            deprecated: false,
        });
    }
    Ok(graph)
}

/// A report of the selected graph. Dev is byte-identical to the unscoped
/// report; main and branch scopes replace the dependency graph with the
/// scope's pin set (the dev-only overlays are emptied — they describe
/// recorded activity, not a pin set).
pub async fn generate_scoped_report(
    repo: &impl SpecRepository,
    scope: ReportScope,
) -> Result<DependencyReport, AppError> {
    let mut report = generate_report(repo).await?;
    match scope {
        ReportScope::Dev => {}
        ReportScope::Main => {
            let pins = repo.list_current_trunk_pins().await?;
            report.dependency_graph = pins_as_dependency_graph(repo, pins).await?;
            report.missing_endpoints = Vec::new();
            report.unused_endpoints = Vec::new();
            report.scope_label = Some("main".to_string());
        }
        ReportScope::Branch { name, at } => {
            let branch = repo
                .find_branch(&name)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("no sanshain-branch '{name}'")))?;
            let pins = repo.list_branch_pins(branch.id, at.as_deref()).await?;
            report.dependency_graph = pins_as_dependency_graph(repo, pins).await?;
            report.missing_endpoints = Vec::new();
            report.unused_endpoints = Vec::new();
            report.scope_label = Some(match at {
                Some(at) => format!("{name}@{at}"),
                None => name,
            });
        }
    }
    Ok(report)
}

#[instrument(skip_all)]
pub async fn generate_report(repo: &impl SpecRepository) -> Result<DependencyReport, AppError> {
    let mut report = repo.get_report().await?;
    report.service_tags = repo.get_all_service_tags().await?;
    report.trunk_graph = repo.list_current_trunk_pins().await?;
    // The staleness boundary for the main graph (ADR-0004): trunk entries not
    // refreshed since half the TTL are highlighted as forgotten-in-progress.
    let ttl_days = super::admin_service::get_trunk_max_age_days(repo).await?;
    if ttl_days > 0 {
        let boundary = chrono::Utc::now() - chrono::Duration::hours(ttl_days as i64 * 12);
        report.trunk_stale_before =
            Some(boundary.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    }
    Ok(report)
}

pub fn render_report_markdown(report: &DependencyReport) -> String {
    let mut md = String::new();
    md.push_str("# Sanshain Dependency Report\n\n");
    if let Some(scope) = &report.scope_label {
        md.push_str(&format!("Scope: {scope}\n\n"));
    }

    md.push_str("| Consumer | Producer | Type | Version | Stability | Path | Method |\n");
    md.push_str("| --- | --- | --- | --- | --- | --- | --- |\n");

    for dep in &report.dependency_graph {
        md.push_str(&format!(
            "| {} | {} | {:?} | {} | {} | `{}` | `{}` |\n",
            dep.client,
            dep.service,
            dep.api_type,
            dep.version,
            dep.stability.as_str(),
            dep.path,
            dep.method
        ));
    }

    md
}

pub fn render_isolation_report(report: &DependencyReport) -> String {
    use std::collections::{BTreeMap, BTreeSet};

    let mut md = String::new();
    md.push_str("# Service Isolation Report\n\n");
    if let Some(scope) = &report.scope_label {
        md.push_str(&format!("Scope: {scope}\n\n"));
    }

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
            version: SemVer::new(1, 2, 0),
            stability: Stability::Ga,
            path: "/test".to_string(),
            method: "GET".to_string(),
            deprecated: false,
        }
    }

    fn make_report(deps: Vec<DependencyInfo>) -> DependencyReport {
        DependencyReport {
            dependency_graph: deps,
            service_tags: std::collections::HashMap::new(),
            missing_endpoints: vec![],
            unused_endpoints: vec![],
            trunk_graph: vec![],
            trunk_stale_before: None,
            scope_label: None,
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
    fn markdown_report_names_version_and_stability() {
        let report = make_report(vec![make_dep(
            "order-service",
            "user-service",
            ApiType::OpenApi,
        )]);
        let result = render_report_markdown(&report);
        assert!(
            result.contains(
                "| order-service | user-service | OpenApi | 1.2.0 | ga | `/test` | `GET` |"
            )
        );
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
