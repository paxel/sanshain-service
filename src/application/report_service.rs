use crate::domain::models::*;
use crate::domain::ports::SpecRepository;

pub async fn generate_report(repo: &impl SpecRepository, branch: &str) -> Result<DependencyReport, AppError> {
    let mut report = repo.get_report(branch).await?;
    report.service_tags = repo.get_all_service_tags().await?;
    Ok(report)
}

pub fn render_report_markdown(report: &DependencyReport) -> String {
    let mut md = String::new();
    md.push_str(&format!("# Sanshain Dependency Report: Branch `{}`\n\n", report.branch));
    
    md.push_str("| Client | Service | Type | Path | Method |\n");
    md.push_str("| --- | --- | --- | --- | --- |\n");
    
    for dep in &report.dependency_graph {
        md.push_str(&format!("| {} | {} | {:?} | `{}` | `{}` |\n", dep.client, dep.service, dep.api_type, dep.path, dep.method));
    }
    
    md
}

pub fn render_isolation_report(report: &DependencyReport) -> String {
    let mut md = String::new();
    md.push_str("# Service Isolation Report\n\n");
    
    let mut consumers: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for dep in &report.dependency_graph {
        consumers.entry(dep.service.clone()).or_default().push(dep.client.clone());
    }

    if consumers.is_empty() {
        md.push_str("No dependencies found.\n");
        return md;
    }

    let mut services: Vec<String> = consumers.keys().cloned().collect();
    services.sort();

    for svc in services {
        let mut clients = consumers.get(&svc).unwrap().clone();
        clients.sort();
        clients.dedup();
        md.push_str(&format!("## Service: {}\n", svc));
        md.push_str("  - Consumed by:\n");
        for client in clients {
            md.push_str(&format!("    - {}\n", client));
        }
        md.push('\n');
    }
    md
}
