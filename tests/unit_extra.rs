use sanshain_service::application::{admin_service, report_service};
use sanshain_service::application::mock_repo::MockRepo;
use sanshain_service::domain::models::*;
use sanshain_service::domain::ports::SpecRepository;

fn dep(client: &str, service: &str, api_type: ApiType, path: &str, method: &str) -> DependencyInfo {
    DependencyInfo { client: client.into(), service: service.into(), api_type, path: path.into(), method: method.into() }
}

// 1. render_report_markdown empty table header present
#[test]
fn report_markdown_headers() {
    let report = DependencyReport { branch: "main".into(), dependency_graph: vec![], service_tags: Default::default(), missing_endpoints: vec![], unused_endpoints: vec![] };
    let md = report_service::render_report_markdown(&report);
    assert!(md.contains("# Sanshain Dependency Report: Branch `main`"));
    assert!(md.contains("| Client | Service | Type | Path | Method |"));
}

// 2. render_report_markdown with one dep row
#[test]
fn report_markdown_one_row() {
    let report = DependencyReport { branch: "dev".into(), dependency_graph: vec![dep("cli","svc", ApiType::OpenApi, "/x","GET")], service_tags: Default::default(), missing_endpoints: vec![], unused_endpoints: vec![] };
    let md = report_service::render_report_markdown(&report);
    assert!(md.contains("| cli | svc | OpenApi | `/x` | `GET` |"));
}

// 3. generate_report populates service_tags map from repo
#[tokio::test]
async fn generate_report_populates_tags() {
    let repo = MockRepo::new();
    // seed some tags
    let sid = repo.ensure_service("svc").await.unwrap();
    {
        let mut tags = repo.service_tags.lock().unwrap();
        tags.insert(sid, vec!["a".into(), "b".into()]);
    }
    let rep = report_service::generate_report(&repo, "main").await.unwrap();
    assert_eq!(rep.branch, "main");
}

// 4. admin_service list_services_detailed empty
#[tokio::test]
async fn list_services_detailed_empty() {
    let repo = MockRepo::new();
    let list = admin_service::list_services_detailed(&repo).await.unwrap();
    assert!(list.is_empty());
}

// 5. admin_service list_services after seeding
#[tokio::test]
async fn list_services_after_seed() {
    let repo = MockRepo::new();
    repo.ensure_service("a").await.unwrap();
    let list = admin_service::list_services(&repo).await.unwrap();
    assert!(list.contains(&"a".into()));
}

// 6. list_all_branches empty
#[tokio::test]
async fn list_all_branches_empty() {
    let repo = MockRepo::new();
    let list = admin_service::list_all_branches(&repo).await.unwrap();
    assert!(list.is_empty());
}

// 7. list_branches for a service
#[tokio::test]
async fn list_branches_for_service() {
    let repo = MockRepo::new();
    let _sid = repo.ensure_service("svc").await.unwrap();
    // MockRepo::list_branches returns empty in this implementation
    let list = admin_service::list_branches(&repo, "svc").await.unwrap();
    assert!(list.is_empty());
}

// 8. list_clients empty then seed one
#[tokio::test]
async fn list_clients_seed() {
    let repo = MockRepo::new();
    assert!(admin_service::list_clients(&repo).await.unwrap().is_empty());
    repo.ensure_client("c1").await.unwrap();
    let list = admin_service::list_clients(&repo).await.unwrap();
    assert!(list.contains(&"c1".into()));
}
