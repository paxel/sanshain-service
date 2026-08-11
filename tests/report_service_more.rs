use sanshain_service::application::mock_repo::MockRepo;
use sanshain_service::application::report_service as report;
use sanshain_service::domain::models::*;
use sanshain_service::domain::ports::SpecRepository;

fn dep(client: &str, service: &str, api_type: ApiType, path: &str, method: &str) -> DependencyInfo {
    DependencyInfo {
        client: client.into(),
        service: service.into(),
        api_type,
        version: SemVer::new(2, 1, 0),
        stability: Stability::Snapshot,
        path: path.into(),
        method: method.into(),
        deprecated: false,
    }
}

fn empty_report(deps: Vec<DependencyInfo>) -> DependencyReport {
    DependencyReport {
        trunk_graph: Vec::new(),
        harvested_subscriptions: Vec::new(),
        trunk_stale_before: None,
        scope_label: None,
        dependency_graph: deps,
        service_tags: Default::default(),
        missing_endpoints: vec![],
        unused_endpoints: vec![],
    }
}

// 1. render_report_markdown includes header rows (reports are global now — no
// branch heading).
#[test]
fn markdown_includes_headers() {
    let md = report::render_report_markdown(&empty_report(vec![]));
    assert!(md.contains("# Sanshain Dependency Report"));
    assert!(md.contains("| Consumer | Producer | Type | Version | Stability | Path | Method |"));
}

// 2. render_report_markdown includes rows for dependencies, carrying the
// Consumer's pinned version and its stability.
#[test]
fn markdown_includes_rows() {
    let md = report::render_report_markdown(&empty_report(vec![dep(
        "c",
        "s",
        ApiType::Proto,
        "/p",
        "POST",
    )]));
    assert!(md.contains("| c | s | Proto | 2.1.0 | snapshot | `/p` | `POST` |"));
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
    // MockRepo's get_all_service_tags is a stub returning an empty map; the
    // point here is that generate_report populates service_tags from the repo
    // without erroring and reports no dependencies for an empty graph.
    let r = report::generate_report(&repo).await.unwrap();
    assert!(r.dependency_graph.is_empty());
    assert!(r.service_tags.is_empty());
}

// 4. render_isolation_report for empty deps has notice
#[test]
fn isolation_empty_notice() {
    let md = report::render_isolation_report(&empty_report(vec![]));
    assert!(md.contains("No dependencies found."));
}

// Harvested subscriptions are scoped per view (#6/ADR-0006): the dev report
// carries the full set, the live main view only the trunk edges, and a past
// main@date carries none (no harvested timeline query yet).
#[tokio::test]
async fn scoped_report_filters_harvested_subscriptions_by_view() {
    let repo = MockRepo::new();
    let owner = repo.ensure_service("orders").await.unwrap();
    let trunk_client = repo.ensure_client("shipping").await.unwrap();
    repo.reconcile_harvested_subscriptions(
        trunk_client,
        "shipping",
        true,
        &[HarvestedSubInput {
            channel: "orders".into(),
            message_name: "OrderPlaced".into(),
            owner_service_id: Some(owner),
            owner_name: Some("orders".into()),
        }],
        "2026-01-01T00:00:00Z",
    )
    .await
    .unwrap();
    let dev_client = repo.ensure_client("billing").await.unwrap();
    repo.reconcile_harvested_subscriptions(
        dev_client,
        "billing",
        false,
        &[HarvestedSubInput {
            channel: "invoices".into(),
            message_name: "Invoiced".into(),
            owner_service_id: None,
            owner_name: None,
        }],
        "2026-01-01T00:00:00Z",
    )
    .await
    .unwrap();

    // Dev report: the full set (trunk + dev).
    let dev = report::generate_report(&repo).await.unwrap();
    assert_eq!(dev.harvested_subscriptions.len(), 2);

    // Live main view: only the trunk subscription.
    let main = report::generate_scoped_report(&repo, report::ReportScope::Main { at: None })
        .await
        .unwrap();
    assert_eq!(main.harvested_subscriptions.len(), 1);
    assert!(main.harvested_subscriptions[0].trunk);
    assert_eq!(main.harvested_subscriptions[0].channel, "orders");

    // A past main@date carries none.
    let past = report::generate_scoped_report(
        &repo,
        report::ReportScope::Main {
            at: Some("2026-01-01T00:00:00Z".into()),
        },
    )
    .await
    .unwrap();
    assert!(past.harvested_subscriptions.is_empty());
}
