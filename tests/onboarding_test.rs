//! Producer onboarding.
//!
//! The claim under test is narrow and specific: while onboarding is on, a
//! Provide to a protected branch is not refused for *any* of the five reasons it
//! could be — and everything protection records is still recorded. Each refusal
//! gets its own test, because relaxing only the compatibility checks is the
//! plausible half-fix that would leave a Producer blocked the first time it drops
//! an endpoint.

use sanshain_service::application::{admin_service, spec_service};
use sanshain_service::domain::models::ApiType;
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
        "200":
          description: ok
          content:
            application/json:
              schema:
                type: object
                properties:
                  id: { type: string }
                  total: { type: string }
  /legacy:
    get:
      responses:
        "200": { description: ok }
"#;

/// Removes `/legacy` without deprecating it, and retypes `total`. Both are
/// breaking, by two different rules.
const V2_BREAKING: &str = r#"
openapi: 3.0.0
info: { title: Orders, version: 1.0.0 }
paths:
  /orders:
    get:
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema:
                type: object
                properties:
                  id: { type: string }
                  total: { type: integer }
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
    spec: &str,
) -> Result<
    sanshain_service::domain::models::ProvideResponse,
    sanshain_service::domain::models::AppError,
> {
    spec_service::provide_spec(
        repo,
        "orders",
        "master",
        ApiType::OpenApi,
        spec,
        None,
        false,
    )
    .await
}

#[cfg(test)]
async fn onboard(repo: &SqliteSpecRepository, on: bool) {
    admin_service::set_producer_onboarding(repo, "orders", on)
        .await
        .expect("onboarding toggled");
}

#[tokio::test]
async fn a_protected_branch_still_refuses_breaking_changes_by_default() {
    let repo = repo().await;
    provide(&repo, V1).await.expect("first provide");

    let err = provide(&repo, V2_BREAKING)
        .await
        .expect_err("a breaking change must be refused when onboarding is off");
    assert!(
        matches!(
            err,
            sanshain_service::domain::models::AppError::Quarantined { .. }
        ),
        "expected the submission to be refused and held, got {err:?}"
    );
}

#[tokio::test]
async fn onboarding_accepts_a_breaking_change_on_a_protected_branch() {
    let repo = repo().await;
    provide(&repo, V1).await.expect("first provide");
    onboard(&repo, true).await;

    provide(&repo, V2_BREAKING)
        .await
        .expect("onboarding should accept the breaking change");

    // The removal actually took effect, rather than being quietly ignored.
    let branch = repo
        .find_branch(
            repo.find_service("orders")
                .await
                .expect("lookup")
                .expect("orders"),
            "master",
        )
        .await
        .expect("lookup")
        .expect("master");
    let live: Vec<String> = repo
        .get_endpoints_for_branch(branch)
        .await
        .expect("endpoints")
        .into_iter()
        .map(|e| e.path)
        .collect();
    assert!(
        !live.contains(&"/legacy".to_string()),
        "the removed endpoint should be gone from the live set"
    );
}

/// Removing a non-deprecated endpoint is refused by a different rule from the
/// compatibility check, and is the one a best-effort Producer hits first.
#[tokio::test]
async fn onboarding_allows_removing_a_non_deprecated_endpoint() {
    let repo = repo().await;
    provide(&repo, V1).await.expect("first provide");
    onboard(&repo, true).await;

    let removal_only = V1.replace(
        "  /legacy:\n    get:\n      responses:\n        \"200\": { description: ok }\n",
        "",
    );
    provide(&repo, &removal_only)
        .await
        .expect("onboarding should allow the removal");
}

/// A removed endpoint is soft-deleted, so re-introducing it is normally refused.
/// Onboarding must clear that too, or a Producer that deletes something by
/// mistake can never put it back.
#[tokio::test]
async fn onboarding_allows_re_introducing_a_removed_endpoint() {
    let repo = repo().await;
    provide(&repo, V1).await.expect("first provide");
    onboard(&repo, true).await;

    let removal_only = V1.replace(
        "  /legacy:\n    get:\n      responses:\n        \"200\": { description: ok }\n",
        "",
    );
    provide(&repo, &removal_only).await.expect("removal");
    provide(&repo, V1)
        .await
        .expect("onboarding should allow the endpoint back");
}

/// The point of onboarding over deleting the branch: the history survives.
#[tokio::test]
async fn onboarding_keeps_version_history_and_soft_deletes() {
    let repo = repo().await;
    provide(&repo, V1).await.expect("first provide");
    onboard(&repo, true).await;
    provide(&repo, V2_BREAKING).await.expect("breaking provide");

    let service = repo
        .find_service("orders")
        .await
        .expect("lookup")
        .expect("orders");
    let branch = repo
        .find_branch(service, "master")
        .await
        .expect("lookup")
        .expect("master");

    let (endpoint_id, _, _, _) = repo
        .find_endpoint(service, "master", ApiType::OpenApi, "/orders", "GET")
        .await
        .expect("lookup")
        .expect("/orders exists");
    let versions = repo
        .get_endpoint_versions(endpoint_id)
        .await
        .expect("versions");
    assert!(
        versions.len() >= 2,
        "the protected branch should still record a version per change, got {}",
        versions.len()
    );

    assert!(
        repo.is_endpoint_deleted(branch, ApiType::OpenApi, "/legacy", "GET")
            .await
            .expect("tombstone lookup"),
        "the removal should be a soft delete, not a hard one"
    );
}

#[tokio::test]
async fn a_breaking_change_accepted_while_onboarding_bumps_the_major_version() {
    let repo = repo().await;
    let first = provide(&repo, V1).await.expect("first provide");
    assert_eq!(first.version.to_string(), "1.0.0");

    onboard(&repo, true).await;
    let second = provide(&repo, V2_BREAKING).await.expect("breaking provide");
    assert_eq!(
        second.version.to_string(),
        "2.0.0",
        "the version must tell the truth about what happened"
    );
}

#[tokio::test]
async fn an_accepted_breaking_change_is_recorded_in_the_audit_log() {
    let repo = repo().await;
    provide(&repo, V1).await.expect("first provide");
    onboard(&repo, true).await;
    provide(&repo, V2_BREAKING).await.expect("breaking provide");

    let logs = repo.get_recent_audit_logs(50).await.expect("audit logs");
    let entry = logs
        .iter()
        .find(|e| e.action == "ACCEPTED_BREAKING")
        .expect("the accepted breaking change should be audited");
    assert_eq!(entry.service.as_deref(), Some("orders"));
    assert_eq!(entry.branch.as_deref(), Some("master"));
    assert!(
        !entry.details.is_empty(),
        "the entry must carry the reason the refusal would have given"
    );
}

#[tokio::test]
async fn turning_onboarding_off_restores_the_refusals() {
    let repo = repo().await;
    provide(&repo, V1).await.expect("first provide");
    onboard(&repo, true).await;
    provide(&repo, V2_BREAKING).await.expect("breaking provide");

    onboard(&repo, false).await;
    let further_break = V2_BREAKING.replace("/orders:", "/orders-v2:");
    let err = provide(&repo, &further_break)
        .await
        .expect_err("refusals should be back");
    assert!(matches!(
        err,
        sanshain_service::domain::models::AppError::Quarantined { .. }
    ));
}

/// Onboarding is per Producer, so relaxing one must not relax another.
#[tokio::test]
async fn onboarding_one_producer_does_not_affect_another() {
    let repo = repo().await;
    provide(&repo, V1).await.expect("orders first provide");
    spec_service::provide_spec(
        &repo,
        "billing",
        "master",
        ApiType::OpenApi,
        V1,
        None,
        false,
    )
    .await
    .expect("billing first provide");

    onboard(&repo, true).await;

    let err = spec_service::provide_spec(
        &repo,
        "billing",
        "master",
        ApiType::OpenApi,
        V2_BREAKING,
        None,
        false,
    )
    .await
    .expect_err("billing is not onboarding");
    assert!(matches!(
        err,
        sanshain_service::domain::models::AppError::Quarantined { .. }
    ));
}

#[tokio::test]
async fn onboarding_producers_are_listed() {
    let repo = repo().await;
    provide(&repo, V1).await.expect("orders");
    spec_service::provide_spec(
        &repo,
        "billing",
        "master",
        ApiType::OpenApi,
        V1,
        None,
        false,
    )
    .await
    .expect("billing");

    assert!(
        admin_service::list_onboarding_producers(&repo)
            .await
            .expect("list")
            .is_empty()
    );

    onboard(&repo, true).await;
    assert_eq!(
        admin_service::list_onboarding_producers(&repo)
            .await
            .expect("list"),
        vec!["orders".to_string()],
        "the listing is the only reminder that a never-expiring flag is on"
    );

    onboard(&repo, false).await;
    assert!(
        admin_service::list_onboarding_producers(&repo)
            .await
            .expect("list")
            .is_empty()
    );
}

#[tokio::test]
async fn onboarding_an_unknown_producer_is_not_found() {
    let repo = repo().await;
    assert!(matches!(
        admin_service::set_producer_onboarding(&repo, "nope", true).await,
        Err(sanshain_service::domain::models::AppError::NotFound(_))
    ));
}
