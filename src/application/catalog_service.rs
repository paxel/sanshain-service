//! What a system's Backstage `catalog-info.yaml` says about it, pushed by
//! its build: the Peripheral Resources outside Sanshain it depends on (#46),
//! drawn in the network graph, and its context — owner, description, source
//! location and links (#48) — shown on its node and its page.

use super::{now_iso, run_cpu_bound};
use crate::domain::models::*;
use crate::domain::ports::SpecRepository;
use tracing::instrument;

/// Record the periphery and the context the system's `catalog-info.yaml`
/// declares, replacing what the system pushed before. The periphery always
/// updates the dev set; a trunk push updates the trunk set too (the dev view
/// is all activity, trunk included). The context (#48) is one present-day
/// fact per system, whatever the stream. A file Sanshain cannot read as the
/// system's Component records nothing — not even "no periphery", so the
/// previous set and context stay — and answers `recorded: false` with the
/// reason as a warning: the graph is informational and never fails a build
/// (ADR-0007 amendment).
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
    let parsed =
        run_cpu_bound(move || Ok(crate::backstage::read_component(&yaml, &parse_for))).await?;
    let component = match parsed {
        Ok(component) => component,
        Err(e) => {
            return Ok(CatalogPushResult {
                system,
                trunk,
                resources: Vec::new(),
                recorded: false,
                warnings: vec![e.to_string()],
                changed: false,
            });
        }
    };
    let now = now_iso();
    let periphery_changed = repo
        .reconcile_periphery(&system, trunk, &component.resources, &now)
        .await?;
    let context_changed = repo
        .upsert_system_context(&system, &component.context, &now)
        .await?;
    metrics::counter!("sanshain_catalog_pushes_total").increment(1);
    Ok(CatalogPushResult {
        system,
        trunk,
        resources: component.resources,
        recorded: true,
        warnings: Vec::new(),
        changed: periphery_changed || context_changed,
    })
}

/// A Producer's context (#48), `None` inside when no push described it;
/// `NotFound` when there is no such Producer.
#[instrument(skip_all, fields(name = %name))]
pub async fn producer_context(
    repo: &impl SpecRepository,
    name: &str,
) -> Result<ParticipantContext, AppError> {
    if repo.find_service(name).await?.is_none() {
        return Err(AppError::NotFound(format!("Service not found: {name}")));
    }
    participant_context(repo, name).await
}

/// A Consumer's context (#48); `NotFound` when there is no such Consumer.
#[instrument(skip_all, fields(name = %name))]
pub async fn consumer_context(
    repo: &impl SpecRepository,
    name: &str,
) -> Result<ParticipantContext, AppError> {
    if !repo.list_consumers().await?.iter().any(|c| c == name) {
        return Err(AppError::NotFound(format!("Client not found: {name}")));
    }
    participant_context(repo, name).await
}

async fn participant_context(
    repo: &impl SpecRepository,
    name: &str,
) -> Result<ParticipantContext, AppError> {
    Ok(ParticipantContext {
        name: name.to_string(),
        context: repo.get_system_context(name).await?,
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

/// Close the dev and trunk periphery, and forget the context (#48), of every
/// `candidate` that is no longer a Producer or a Consumer, so a deleted
/// system leaves no stray node behind. Callers pass only names that *were*
/// participants before their delete. The periphery is closed, not deleted:
/// the timeline keeps it, and a sanshain-branch's copy is never touched
/// (ADR-0005 #26B). The context has no timeline, so its row goes.
#[instrument(skip_all)]
pub(crate) async fn close_catalog_of_departed(
    repo: &impl SpecRepository,
    candidates: &[String],
) -> Result<(), AppError> {
    let remaining = participants(repo).await?;
    let now = now_iso();
    for system in candidates {
        if !remaining.contains(system) {
            repo.reconcile_periphery(system, true, &[], &now).await?;
            repo.delete_system_context(system).await?;
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
                recorded: true,
                warnings: Vec::new(),
                changed: true,
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
    async fn a_mismatched_component_records_nothing_and_warns() {
        let repo = MockRepo::new();
        let result = push_catalog(&repo, "orders", &catalog("billing", &["db"]), false)
            .await
            .unwrap();
        assert!(!result.recorded);
        assert!(result.resources.is_empty());
        assert_eq!(result.warnings.len(), 1);
        assert!(
            result.warnings[0].contains("no Component named 'orders'"),
            "{:?}",
            result.warnings
        );
        assert!(repo.periphery.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_unreadable_file_keeps_the_previous_periphery() {
        let repo = MockRepo::new();
        push_catalog(&repo, "orders", &catalog("orders", &["orders-db"]), true)
            .await
            .unwrap();
        for bad in [
            "key: [unclosed",
            "just a string",
            &catalog("billing", &["x"]),
        ] {
            let result = push_catalog(&repo, "orders", bad, true).await.unwrap();
            assert!(!result.recorded, "{bad}");
            assert_eq!(result.warnings.len(), 1, "{bad}");
        }
        for track in [PeripheryTrack::Dev, PeripheryTrack::Trunk] {
            let set = repo.list_periphery(track, None).await.unwrap();
            assert_eq!(set.len(), 1, "{track:?}");
            assert_eq!(set[0].resource, "resource:default/orders-db");
        }
        let rows = repo.periphery.lock().unwrap();
        assert!(rows.iter().all(|r| r.valid_to.is_none()));
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
        close_catalog_of_departed(&repo, &["users".to_string(), "gone".to_string()])
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
    async fn a_push_records_the_context_and_an_identical_push_changes_nothing() {
        let repo = MockRepo::new();
        let described = "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: orders\n  description: Orders.\nspec:\n  owner: team-orders\n";
        let first = push_catalog(&repo, "orders", described, false)
            .await
            .unwrap();
        assert!(first.recorded && first.changed);
        assert_eq!(
            repo.get_system_context("orders").await.unwrap(),
            Some(SystemContext {
                owner: Some(SystemOwner {
                    kind: "group".to_string(),
                    name: "team-orders".to_string(),
                }),
                description: Some("Orders.".to_string()),
                ..SystemContext::default()
            })
        );
        let again = push_catalog(&repo, "orders", described, false)
            .await
            .unwrap();
        assert!(again.recorded && !again.changed);
        // A push without the fields replaces the context with an empty one.
        let bare = push_catalog(&repo, "orders", &catalog("orders", &[]), false)
            .await
            .unwrap();
        assert!(bare.changed);
        assert_eq!(
            repo.get_system_context("orders").await.unwrap(),
            Some(SystemContext::default())
        );
    }

    #[tokio::test]
    async fn an_unreadable_file_keeps_the_previous_context() {
        let repo = MockRepo::new();
        let described = "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: orders\n  description: Orders.\n";
        push_catalog(&repo, "orders", described, false)
            .await
            .unwrap();
        push_catalog(&repo, "orders", &catalog("billing", &[]), false)
            .await
            .unwrap();
        assert_eq!(
            repo.get_system_context("orders")
                .await
                .unwrap()
                .and_then(|c| c.description),
            Some("Orders.".to_string())
        );
    }

    #[tokio::test]
    async fn the_context_of_a_departed_participant_is_forgotten() {
        let repo = MockRepo::new();
        repo.ensure_service("users").await.unwrap();
        for system in ["users", "gone"] {
            push_catalog(&repo, system, &catalog(system, &["db"]), true)
                .await
                .unwrap();
        }
        close_catalog_of_departed(&repo, &["users".to_string(), "gone".to_string()])
            .await
            .unwrap();
        let remaining: Vec<String> = repo
            .list_system_contexts()
            .await
            .unwrap()
            .into_keys()
            .collect();
        assert_eq!(remaining, vec!["users".to_string()]);
    }

    #[tokio::test]
    async fn producer_and_consumer_context_answer_404_for_unknown_names() {
        let repo = MockRepo::new();
        repo.ensure_service("orders").await.unwrap();
        assert!(matches!(
            producer_context(&repo, "ghost").await,
            Err(AppError::NotFound(_))
        ));
        assert!(matches!(
            consumer_context(&repo, "orders").await,
            Err(AppError::NotFound(_))
        ));
        assert_eq!(
            producer_context(&repo, "orders").await.unwrap(),
            ParticipantContext {
                name: "orders".to_string(),
                context: None,
            }
        );
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
