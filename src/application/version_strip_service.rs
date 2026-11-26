//! The version strips at the top of the Producer and Consumer pages
//! (san-1032): per Producer version, who pins it in a graph scope; per
//! require-set version of a Consumer, the Pins it holds.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::report_service::{ReportScope, generate_scoped_report};
use crate::domain::models::{ApiType, AppError, RequireSet, SemVer, Stability};
use crate::domain::ports::SpecRepository;

/// One bar of a Producer's strip.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PinnedVersion {
    pub api_type: ApiType,
    pub version: SemVer,
    /// `None` for a version the scope still pins but the line no longer has.
    pub stability: Option<Stability>,
    /// The Consumers that pin it in the scope, sorted.
    pub consumers: Vec<String>,
}

/// A bar's key: API type (as text, for ordering) and version.
type BarKey = (&'static str, SemVer);
/// A bar: API type, the version's stability, and who pins it.
type Bar = (ApiType, Option<Stability>, BTreeSet<String>);

/// Every version of a Producer's lines with the Consumers that pin it in
/// `scope` (dev, main or a sanshain-branch, optionally at an instant); a
/// version nobody pins is a zero bar. `404` for an unknown Producer.
pub async fn producer_pinned_by(
    repo: &impl SpecRepository,
    producername: &str,
    scope: ReportScope,
) -> Result<Vec<PinnedVersion>, AppError> {
    let Some(service_id) = repo.find_service(producername).await? else {
        return Err(AppError::NotFound(format!(
            "Producer '{producername}' is unknown"
        )));
    };
    let mut bars: BTreeMap<BarKey, Bar> = BTreeMap::new();
    for meta in repo.list_spec_versions(service_id).await? {
        bars.insert(
            (meta.api_type.as_str(), meta.version),
            (meta.api_type, Some(meta.stability), BTreeSet::new()),
        );
    }
    let report = generate_scoped_report(repo, scope).await?;
    for edge in report
        .dependency_graph
        .into_iter()
        .filter(|e| e.service == producername)
    {
        bars.entry((edge.api_type.as_str(), edge.version))
            .or_insert_with(|| (edge.api_type, None, BTreeSet::new()))
            .2
            .insert(edge.client);
    }
    Ok(bars
        .into_iter()
        .map(
            |((_, version), (api_type, stability, consumers))| PinnedVersion {
                api_type,
                version,
                stability,
                consumers: consumers.into_iter().collect(),
            },
        )
        .collect())
}

/// A Consumer's require-set versions, ascending, with their Pins.
pub async fn consumer_require_sets(
    repo: &impl SpecRepository,
    consumername: &str,
) -> Result<Vec<RequireSet>, AppError> {
    Ok(repo.list_require_sets(consumername).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::mock_repo::MockRepo;
    use crate::domain::models::RequireSetPin;

    #[tokio::test]
    async fn require_sets_are_numbered_in_order_of_first_sight() {
        let repo = MockRepo::new();
        let pin = |producer: &str, version: &str| RequireSetPin {
            producer: producer.to_string(),
            api_type: ApiType::OpenApi,
            version: version.parse().unwrap(),
        };
        assert_eq!(
            repo.record_require_set("web", "aaa", &pin("orders", "2.3.0"), "t1")
                .await
                .unwrap(),
            1
        );
        repo.record_require_set("web", "aaa", &pin("billing", "1.0.0"), "t2")
            .await
            .unwrap();
        assert_eq!(
            repo.record_require_set("web", "bbb", &pin("orders", "2.5.0"), "t3")
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            repo.record_require_set("app", "bbb", &pin("orders", "2.5.0"), "t3")
                .await
                .unwrap(),
            1,
            "numbers are per Consumer"
        );
        let sets = consumer_require_sets(&repo, "web").await.unwrap();
        assert_eq!(sets.len(), 2);
        assert_eq!(sets[0].first_seen_at, "t1");
        assert_eq!(
            sets[0].pins,
            vec![pin("billing", "1.0.0"), pin("orders", "2.3.0")]
        );
        assert_eq!(sets[1].number, 2);
    }

    #[tokio::test]
    async fn an_unknown_producer_has_no_strip() {
        let repo = MockRepo::new();
        assert!(matches!(
            producer_pinned_by(&repo, "nobody", ReportScope::Dev).await,
            Err(AppError::NotFound(_))
        ));
    }
}
