use crate::domain::models::*;
use crate::domain::ports::SpecRepository;

pub async fn generate_report(
    repo: &impl SpecRepository,
    branch: &str,
) -> Result<DependencyReport, AppError> {
    let mut report = repo.get_report(branch).await?;
    report.service_tags = repo.get_all_service_tags().await?;
    Ok(report)
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
