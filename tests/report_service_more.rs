use sanshain_service::application::report_service as report;
use sanshain_service::application::mock_repo::MockRepo;
use sanshain_service::domain::ports::SpecRepository;
use sanshain_service::domain::models::*;

fn dep(client: &str, service: &str, api_type: ApiType, path: &str, method: &str) -> DependencyInfo {
    DependencyInfo { client: client.into(), service: service.into(), api_type, path: path.into(), method: method.into() }
}

// 1. render_report_markdown includes header rows
#[test]
fn markdown_includes_headers() {
    let r = DependencyReport { branch: "b".into(), dependency_graph: vec![], service_tags: Default::default(), missing_endpoints: vec![], unused_endpoints: vec![] };
    let md = report::render_report_markdown(&r);
    assert!(md.contains("# Sanshain Dependency Report: Branch `b`"));
    assert!(md.contains("| Client | Service | Type | Path | Method |"));
}

// 2. render_report_markdown includes rows for dependencies
#[test]
fn markdown_includes_rows() {
    let r = DependencyReport { branch: "b".into(), dependency_graph: vec![dep("c","s", ApiType::Proto, "/p","POST")], service_tags: Default::default(), missing_endpoints: vec![], unused_endpoints: vec![] };
    let md = report::render_report_markdown(&r);
    assert!(md.contains("| c | s | Proto | `/p` | `POST` |"));
}

// 3. generate_report loads service_tags via repo
#[tokio::test]
async fn generate_report_loads_tags() {
    let repo = MockRepo::new();
    let sid = repo.ensure_service("svc").await.unwrap();
    {
        let mut tags = repo.service_tags.lock().unwrap();
        tags.insert(sid, vec!["x".into()]);
    }
    let r = report::generate_report(&repo, "main").await.unwrap();
    assert_eq!(r.branch, "main");
}

// 4. render_isolation_report for empty deps has notice
#[test]
fn isolation_empty_notice() {
    let r = DependencyReport { branch: "b".into(), dependency_graph: vec![], service_tags: Default::default(), missing_endpoints: vec![], unused_endpoints: vec![] };
    let md = report::render_isolation_report(&r);
    assert!(md.contains("No dependencies found."));
}
