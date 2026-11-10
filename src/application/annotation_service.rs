//! Annotations (#91): project-declared entries in `sanshain.yaml` whose
//! values the build fills in. `scope: version` entries travel on a provide
//! and are stored with that version; `scope: system` entries travel on every
//! call naming the system and are stored once per system, the latest call
//! winning per name.

use super::now_iso;
use crate::domain::models::*;
use crate::domain::ports::SpecRepository;
use tracing::instrument;

/// Which call carries the annotations: only a provide has a version to
/// store `scope: version` entries with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnnotatedCall {
    Provide,
    /// require, require-bundle, catalog push.
    SystemOnly,
}

/// Validate a call's annotations and split them by scope. Out-of-bounds
/// entries, and `scope: version` entries on a call without a version, are a
/// `400` naming the entry, before anything is written.
pub fn accept_annotations(
    entries: &[AnnotationEntry],
    call: AnnotatedCall,
) -> Result<Annotations, AppError> {
    let annotations = Annotations::from_entries(entries)
        .map_err(|reason| AppError::BadRequest(format!("annotations: {reason}")))?;
    if call == AnnotatedCall::SystemOnly
        && let Some(first) = annotations.version.first()
    {
        return Err(AppError::BadRequest(format!(
            "annotations: '{}' has scope version, which travels only on a provide",
            first.name
        )));
    }
    Ok(annotations)
}

/// Store a system's annotations, the latest call winning per name.
#[instrument(skip_all, fields(system = %system))]
pub async fn record_system_annotations(
    repo: &impl SpecRepository,
    system: &str,
    annotations: &[Annotation],
) -> Result<(), AppError> {
    if annotations.is_empty() {
        return Ok(());
    }
    repo.upsert_system_annotations(system, annotations, &now_iso())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::mock_repo::MockRepo;

    fn entry(scope: AnnotationScope, name: &str, value: &str) -> AnnotationEntry {
        AnnotationEntry {
            scope,
            name: name.to_string(),
            value: value.to_string(),
        }
    }

    #[test]
    fn a_provide_splits_the_entries_by_scope() {
        let annotations = accept_annotations(
            &[
                entry(AnnotationScope::Version, "requirements", " https://x/1 "),
                entry(AnnotationScope::System, "jenkins", "https://j/payments"),
            ],
            AnnotatedCall::Provide,
        )
        .unwrap();
        assert_eq!(
            annotations.version,
            vec![Annotation {
                name: "requirements".to_string(),
                value: "https://x/1".to_string()
            }]
        );
        assert_eq!(annotations.system[0].name, "jenkins");
    }

    #[test]
    fn version_scope_on_a_call_without_a_version_is_refused_by_name() {
        let err = accept_annotations(
            &[entry(AnnotationScope::Version, "requirements", "x")],
            AnnotatedCall::SystemOnly,
        )
        .unwrap_err();
        match err {
            AppError::BadRequest(msg) => assert_eq!(
                msg,
                "annotations: 'requirements' has scope version, which travels only on a provide"
            ),
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[test]
    fn out_of_bounds_entries_are_refused_by_name() {
        let refused = |entries: Vec<AnnotationEntry>| match accept_annotations(
            &entries,
            AnnotatedCall::Provide,
        ) {
            Err(AppError::BadRequest(msg)) => msg,
            other => panic!("expected BadRequest, got {other:?}"),
        };
        assert_eq!(
            refused(vec![entry(AnnotationScope::System, "bad name", "x")]),
            "annotations: name 'bad name' is not of letters, digits, '-' and '_' (at most 40 characters)"
        );
        assert_eq!(
            refused(vec![entry(AnnotationScope::System, &"n".repeat(41), "x")]),
            format!(
                "annotations: name '{}' is not of letters, digits, '-' and '_' (at most 40 characters)",
                "n".repeat(41)
            )
        );
        assert_eq!(
            refused(vec![entry(AnnotationScope::System, "jenkins", "  ")]),
            "annotations: 'jenkins' has an empty value"
        );
        assert_eq!(
            refused(vec![entry(
                AnnotationScope::Version,
                "doc",
                &"v".repeat(2049)
            )]),
            "annotations: the value of 'doc' is longer than 2048 characters"
        );
        assert_eq!(
            refused(vec![
                entry(AnnotationScope::System, "a", "1"),
                entry(AnnotationScope::System, "a", "2"),
            ]),
            "annotations: 'a' appears twice in scope system"
        );
        let twenty_one: Vec<AnnotationEntry> = (0..21)
            .map(|i| entry(AnnotationScope::Version, &format!("n{i}"), "x"))
            .collect();
        assert_eq!(
            refused(twenty_one),
            "annotations: at most 20 annotations of scope version; 'n20' is one more"
        );
    }

    #[test]
    fn twenty_per_scope_are_fine_and_the_same_name_may_live_in_both_scopes() {
        let mut entries: Vec<AnnotationEntry> = (0..20)
            .map(|i| entry(AnnotationScope::Version, &format!("n{i}"), "x"))
            .collect();
        entries.push(entry(AnnotationScope::System, "n0", "y"));
        let annotations = accept_annotations(&entries, AnnotatedCall::Provide).unwrap();
        assert_eq!(annotations.version.len(), 20);
        assert_eq!(annotations.system.len(), 1);
    }

    #[tokio::test]
    async fn system_annotations_are_stored_per_name_and_the_latest_call_wins() {
        let repo = MockRepo::new();
        let a = |name: &str, value: &str| Annotation {
            name: name.to_string(),
            value: value.to_string(),
        };
        record_system_annotations(&repo, "payments", &[a("jenkins", "old"), a("wiki", "w")])
            .await
            .unwrap();
        record_system_annotations(&repo, "payments", &[a("jenkins", "new")])
            .await
            .unwrap();
        assert_eq!(
            repo.get_system_annotations("payments").await.unwrap(),
            vec![a("jenkins", "new"), a("wiki", "w")]
        );
    }
}
