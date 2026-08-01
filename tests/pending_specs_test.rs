//! Held Provides.
//!
//! Outside onboarding, a breaking change on a protected branch is retained
//! rather than discarded. The claims under test are that it is kept, that it is
//! *not* applied, that repeated pushes collapse to one entry rather than
//! accumulating, and that a Producer fixing the problem itself takes the held
//! entry with it.

use sanshain_service::application::{admin_service, spec_service};
use sanshain_service::domain::models::{ApiType, AppError};
use sanshain_service::domain::ports::SpecRepository;
use sanshain_service::infrastructure::sqlite_repository::SqliteSpecRepository;
use sqlx::sqlite::SqlitePoolOptions;

const V1: &str = r#"
openapi: 3.0.0
info: { title: Orders, version: 1.0.0 }
paths:
  /orders:
    get:
      responses:
        "200": { description: ok }
  /legacy:
    get:
      responses:
        "200": { description: ok }
"#;

/// Drops `/legacy` without deprecating it first — refused on a protected branch.
const V2_BREAKING: &str = r#"
openapi: 3.0.0
info: { title: Orders, version: 1.0.0 }
paths:
  /orders:
    get:
      responses:
        "200": { description: ok }
"#;

/// Also drops `/legacy`, and differs from `V2_BREAKING`, so "latest wins" is
/// observable in what ends up held.
const V3_ALSO_BREAKING: &str = r#"
openapi: 3.0.0
info: { title: Orders, version: 1.0.0 }
paths:
  /orders:
    get:
      responses:
        "200": { description: still ok }
"#;

/// Additive: a new endpoint, nothing removed. Accepted on a protected branch.
const V2_COMPATIBLE: &str = r#"
openapi: 3.0.0
info: { title: Orders, version: 1.0.0 }
paths:
  /orders:
    get:
      responses:
        "200": { description: ok }
  /legacy:
    get:
      responses:
        "200": { description: ok }
  /orders/{id}:
    get:
      responses:
        "200": { description: ok }
"#;

// `cfg(test)` is always true in this crate; the attribute marks these helpers as
// test code so clippy's `allow-*-in-tests` exemptions apply to them.
#[cfg(test)]
async fn repo() -> SqliteSpecRepository {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("in-memory database");
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.expect("migrations");
    repo
}

#[cfg(test)]
async fn provide(
    repo: &SqliteSpecRepository,
    producer: &str,
    spec: &str,
) -> Result<sanshain_service::domain::models::ProvideResponse, AppError> {
    spec_service::provide_spec(
        repo,
        producer,
        "master",
        ApiType::OpenApi,
        spec,
        None,
        false,
    )
    .await
}

#[tokio::test]
async fn a_breaking_change_is_held_and_reported_with_its_id() {
    let repo = repo().await;
    provide(&repo, "orders", V1).await.expect("first provide");

    let err = provide(&repo, "orders", V2_BREAKING)
        .await
        .expect_err("a breaking change is refused");
    let pending_id = match err {
        AppError::Quarantined { pending_id, .. } => pending_id,
        other => panic!("expected the submission to be held, got {other:?}"),
    };

    let held = repo
        .get_pending_spec(pending_id)
        .await
        .expect("lookup")
        .expect("the submission should be retained");
    assert_eq!(held.producer, "orders");
    assert_eq!(held.branch, "master");
    assert_eq!(held.api_type, ApiType::OpenApi);
    assert_eq!(
        held.content, V2_BREAKING,
        "the whole submission is kept, not a summary of it"
    );
    assert!(
        !held.reason.is_empty(),
        "the reason it was refused must be kept with it"
    );
}

/// Held means held: the branch must answer as if nothing was submitted.
#[tokio::test]
async fn a_held_submission_is_not_applied() {
    let repo = repo().await;
    let first = provide(&repo, "orders", V1).await.expect("first provide");
    provide(&repo, "orders", V2_BREAKING)
        .await
        .expect_err("refused");

    let service = repo
        .find_service("orders")
        .await
        .expect("lookup")
        .expect("orders");
    assert!(
        repo.find_endpoint(service, "master", ApiType::OpenApi, "/legacy", "GET")
            .await
            .expect("lookup")
            .is_some(),
        "the endpoint the held submission removes must still be live"
    );

    let branch = repo
        .find_branch(service, "master")
        .await
        .expect("lookup")
        .expect("master");
    let (version, _) = repo
        .get_spec_version(service, branch)
        .await
        .expect("version")
        .expect("a version exists");
    assert_eq!(
        version.to_string(),
        first.version.to_string(),
        "a held submission must not move the version"
    );
}

/// CI pushes on every commit. Without replacement the inbox would fill with the
/// same problem, and the entry eventually reviewed would be stale.
#[tokio::test]
async fn repeated_breaking_pushes_collapse_to_one_entry() {
    let repo = repo().await;
    provide(&repo, "orders", V1).await.expect("first provide");

    provide(&repo, "orders", V2_BREAKING)
        .await
        .expect_err("refused");
    provide(&repo, "orders", V3_ALSO_BREAKING)
        .await
        .expect_err("refused again");

    let held = repo.list_pending_specs().await.expect("listing");
    assert_eq!(held.len(), 1, "one entry per Producer, branch and API type");
    assert_eq!(
        held[0].content, V3_ALSO_BREAKING,
        "the latest submission wins, so what is held is at most one push old"
    );
}

/// The silent-revert case: without this rule an approver could later apply a
/// stale submission over the top of the change that fixed the problem.
#[tokio::test]
async fn a_successful_provide_discards_what_was_held() {
    let repo = repo().await;
    provide(&repo, "orders", V1).await.expect("first provide");
    provide(&repo, "orders", V2_BREAKING)
        .await
        .expect_err("refused");
    assert_eq!(repo.list_pending_specs().await.expect("listing").len(), 1);

    provide(&repo, "orders", V2_COMPATIBLE)
        .await
        .expect("a compatible change is accepted");

    assert!(
        repo.list_pending_specs().await.expect("listing").is_empty(),
        "the Producer fixed it themselves, so the held submission is dead"
    );
}

#[tokio::test]
async fn holding_is_recorded_in_the_audit_log_under_the_rejection_type() {
    let repo = repo().await;
    provide(&repo, "orders", V1).await.expect("first provide");
    provide(&repo, "orders", V2_BREAKING)
        .await
        .expect_err("refused");

    let logs = repo.get_recent_audit_logs(50).await.expect("audit logs");
    let entry = logs
        .iter()
        .find(|e| e.action == "QUARANTINED_SPEC")
        .expect("the hold should be audited");
    assert_eq!(entry.service.as_deref(), Some("orders"));
    assert_eq!(entry.branch.as_deref(), Some("master"));
    assert_eq!(
        entry.action_type.as_deref(),
        Some("REJECT"),
        "reusing the type keeps the existing timeline filter working"
    );
}

/// Onboarding applies the change, so there is nothing to hold.
#[tokio::test]
async fn a_producer_in_onboarding_holds_nothing() {
    let repo = repo().await;
    provide(&repo, "orders", V1).await.expect("first provide");
    admin_service::set_producer_onboarding(&repo, "orders", true)
        .await
        .expect("onboarding on");

    provide(&repo, "orders", V2_BREAKING)
        .await
        .expect("onboarding accepts it");

    assert!(
        repo.list_pending_specs().await.expect("listing").is_empty(),
        "an accepted change is applied, not held"
    );
}

/// A dry run asks whether a Provide *would* be refused. Nothing was submitted.
#[tokio::test]
async fn a_dry_run_holds_nothing() {
    let repo = repo().await;
    provide(&repo, "orders", V1).await.expect("first provide");

    let err = spec_service::provide_spec_dry_run(
        &repo,
        "orders",
        "master",
        ApiType::OpenApi,
        V2_BREAKING,
        false,
    )
    .await
    .expect_err("the dry run reports the refusal");
    assert!(
        matches!(err, AppError::BreakingChange(_)),
        "a dry run has nothing to hold, so it reports a plain refusal"
    );
    assert!(repo.list_pending_specs().await.expect("listing").is_empty());
}

#[tokio::test]
async fn holds_are_kept_apart_per_producer() {
    let repo = repo().await;
    provide(&repo, "orders", V1).await.expect("orders");
    provide(&repo, "billing", V1).await.expect("billing");
    provide(&repo, "orders", V2_BREAKING)
        .await
        .expect_err("orders refused");
    provide(&repo, "billing", V2_BREAKING)
        .await
        .expect_err("billing refused");

    let held = repo.list_pending_specs().await.expect("listing");
    assert_eq!(held.len(), 2);
    let mut producers: Vec<String> = held.into_iter().map(|p| p.producer).collect();
    producers.sort();
    assert_eq!(producers, vec!["billing".to_string(), "orders".to_string()]);
}

#[tokio::test]
async fn a_held_entry_can_be_discarded() {
    let repo = repo().await;
    provide(&repo, "orders", V1).await.expect("first provide");
    let err = provide(&repo, "orders", V2_BREAKING)
        .await
        .expect_err("refused");
    let pending_id = match err {
        AppError::Quarantined { pending_id, .. } => pending_id,
        other => panic!("expected a held submission, got {other:?}"),
    };

    assert!(repo.delete_pending_spec(pending_id).await.expect("discard"));
    assert!(
        !repo
            .delete_pending_spec(pending_id)
            .await
            .expect("discarding twice reports no change")
    );
    assert!(
        repo.get_pending_spec(pending_id)
            .await
            .expect("lookup")
            .is_none()
    );
}
