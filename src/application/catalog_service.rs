//! Periphery from a system's Backstage `catalog-info.yaml` (#46): the
//! Peripheral Resources outside Sanshain a Producer or Consumer depends on,
//! pushed by its build and drawn in the network graph.

use super::{now_iso, run_cpu_bound};
use crate::domain::models::*;
use crate::domain::ports::SpecRepository;
use tracing::instrument;

/// Record the periphery the system's `catalog-info.yaml` declares, replacing
/// what the system pushed before. Always updates the dev set; a trunk push
/// updates the trunk set too (the dev view is all activity, trunk included).
/// A file Sanshain cannot read as the system's Component is rejected rather
/// than recorded as "no periphery", so a name mismatch fails the build.
#[instrument(skip_all, fields(system = %system, trunk))]
pub async fn push_catalog(
    repo: &impl SpecRepository,
    system: &str,
    catalog_info_yaml: &str,
    trunk: bool,
) -> Result<CatalogPushResult, AppError> {
    let system = system.trim().to_string();
    if system.is_empty() {
        return Err(AppError::BadRequest(
            "a catalog push needs a non-empty system name".to_string(),
        ));
    }
    let yaml = catalog_info_yaml.to_string();
    let parse_for = system.clone();
    let resources = run_cpu_bound(move || {
        crate::backstage::component_resource_dependencies(&yaml, &parse_for)
            .map_err(|e| AppError::BadRequest(e.to_string()))
    })
    .await?;
    repo.reconcile_periphery(&system, trunk, &resources, &now_iso())
        .await?;
    metrics::counter!("sanshain_catalog_pushes_total").increment(1);
    Ok(CatalogPushResult {
        system,
        trunk,
        resources,
    })
}

/// Every Producer and Consumer name — who counts as a participant.
#[instrument(skip_all)]
pub(crate) async fn participants(repo: &impl SpecRepository) -> Result<Vec<String>, AppError> {
    let mut names = repo.list_producers().await?;
    names.extend(repo.list_consumers().await?);
    names.sort();
    names.dedup();
    Ok(names)
}

/// Close the dev and trunk periphery of every `candidate` that is no longer
/// a Producer or a Consumer, so a deleted system leaves no stray node behind.
/// Callers pass only names that *were* participants before their delete.
/// Closed, not deleted: the timeline keeps it, and a sanshain-branch's copy
/// is never touched (ADR-0005 #26B).
#[instrument(skip_all)]
pub(crate) async fn close_periphery_of_departed(
    repo: &impl SpecRepository,
    candidates: &[String],
) -> Result<(), AppError> {
    let remaining = participants(repo).await?;
    let now = now_iso();
    for system in candidates {
        if !remaining.contains(system) {
            repo.reconcile_periphery(system, true, &[], &now).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::mock_repo::MockRepo;

    fn catalog(name: &str, depends_on: &[&str]) -> String {
        let refs: String = depends_on.iter().map(|r| format!("    - {r}\n")).collect();
        format!(
            "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: {name}\nspec:\n  dependsOn:\n{refs}"
        )
    }

    #[tokio::test]
    async fn a_dev_push_records_the_dev_set_only() {
        let repo = MockRepo::new();
        let result = push_catalog(&repo, "orders", &catalog("orders", &["orders-db"]), false)
            .await
            .unwrap();
        assert_eq!(
            result,
            CatalogPushResult {
                system: "orders".to_string(),
                trunk: false,
                resources: vec!["resource:default/orders-db".to_string()],
            }
        );
        let dev = repo
            .list_periphery(PeripheryTrack::Dev, None)
            .await
            .unwrap();
        assert_eq!(
            dev,
            vec![PeripheralDependency {
                system: "orders".to_string(),
                resource: "resource:default/orders-db".to_string(),
            }]
        );
        let trunk = repo
            .list_periphery(PeripheryTrack::Trunk, None)
            .await
            .unwrap();
        assert!(trunk.is_empty());
    }

    #[tokio::test]
    async fn a_trunk_push_records_trunk_and_dev() {
        let repo = MockRepo::new();
        push_catalog(&repo, "orders", &catalog("orders", &["orders-db"]), true)
            .await
            .unwrap();
        for track in [PeripheryTrack::Dev, PeripheryTrack::Trunk] {
            let set = repo.list_periphery(track, None).await.unwrap();
            assert_eq!(set.len(), 1, "{track:?}");
            assert_eq!(set[0].resource, "resource:default/orders-db");
        }
    }

    #[tokio::test]
    async fn a_push_replaces_the_previous_set_and_keeps_history() {
        let repo = MockRepo::new();
        push_catalog(
            &repo,
            "orders",
            &catalog("orders", &["old-db", "cache"]),
            true,
        )
        .await
        .unwrap();
        push_catalog(
            &repo,
            "orders",
            &catalog("orders", &["new-db", "cache"]),
            true,
        )
        .await
        .unwrap();
        let current: Vec<String> = repo
            .list_periphery(PeripheryTrack::Trunk, None)
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.resource)
            .collect();
        assert_eq!(
            current,
            vec![
                "resource:default/cache".to_string(),
                "resource:default/new-db".to_string(),
            ]
        );
        // The replaced resource is closed, not deleted.
        let rows = repo.periphery.lock().unwrap();
        let old: Vec<_> = rows
            .iter()
            .filter(|r| r.resource == "resource:default/old-db")
            .collect();
        assert_eq!(old.len(), 2);
        assert!(old.iter().all(|r| r.valid_to.is_some()));
    }

    #[tokio::test]
    async fn an_empty_depends_on_clears_the_set() {
        let repo = MockRepo::new();
        push_catalog(&repo, "orders", &catalog("orders", &["orders-db"]), false)
            .await
            .unwrap();
        let empty =
            "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: orders\n";
        let result = push_catalog(&repo, "orders", empty, false).await.unwrap();
        assert!(result.resources.is_empty());
        assert!(
            repo.list_periphery(PeripheryTrack::Dev, None)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_mismatched_component_is_a_bad_request_and_records_nothing() {
        let repo = MockRepo::new();
        let err = push_catalog(&repo, "orders", &catalog("billing", &["db"]), false)
            .await
            .unwrap_err();
        match err {
            AppError::BadRequest(msg) => assert!(msg.contains("no Component named 'orders'")),
            other => panic!("expected BadRequest, got {other:?}"),
        }
        assert!(repo.periphery.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn only_candidates_without_a_role_lose_their_periphery() {
        let repo = MockRepo::new();
        repo.ensure_service("users").await.unwrap();
        for system in ["users", "gone", "catalog-only"] {
            push_catalog(&repo, system, &catalog(system, &["db"]), true)
                .await
                .unwrap();
        }
        // `catalog-only` was never a participant, so it is no candidate.
        close_periphery_of_departed(&repo, &["users".to_string(), "gone".to_string()])
            .await
            .unwrap();
        for track in [PeripheryTrack::Dev, PeripheryTrack::Trunk] {
            let systems: Vec<String> = repo
                .list_periphery(track, None)
                .await
                .unwrap()
                .into_iter()
                .map(|p| p.system)
                .collect();
            assert_eq!(
                systems,
                vec!["catalog-only".to_string(), "users".to_string()],
                "{track:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_blank_system_name_is_refused() {
        let repo = MockRepo::new();
        assert!(matches!(
            push_catalog(&repo, "  ", &catalog("orders", &[]), false).await,
            Err(AppError::BadRequest(_))
        ));
    }
}
