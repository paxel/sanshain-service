//! Git branches on versions (san-1029). A build says which git branch it ran
//! on (and, on a pull or merge request build, where it merges); the admin
//! describes the company's branch naming as regular expressions with the
//! named groups `kind` and `target`. Sanshain reads a version's branch with
//! them whenever the origin is shown, so a changed pattern re-reads every
//! version. The reading is information only: it never changes what a
//! require serves.

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::domain::models::{AppError, BranchReading, VersionOrigin};
use crate::domain::ports::SpecRepository;

/// The settings key the rules are stored under, as JSON.
pub const BRANCH_RULES_SETTING: &str = "branch_rules";
/// At most this many naming patterns.
pub const MAX_BRANCH_PATTERNS: usize = 20;
/// A pattern is at most this many characters.
pub const MAX_BRANCH_PATTERN_CHARS: usize = 500;
/// The compiled size a pattern may take, so a pathological expression
/// cannot eat the memory of every page that shows a version.
const PATTERN_SIZE_LIMIT: usize = 256 * 1024;

/// The branch rules as the admin wrote them.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct BranchRules {
    /// Regular expressions with the named groups `kind` and `target`, tried
    /// in order; the first that matches a branch reads it.
    #[serde(default)]
    pub patterns: Vec<String>,
}

/// The rules ready to read branches with.
#[derive(Debug, Default)]
pub struct CompiledBranchRules {
    patterns: Vec<Regex>,
}

impl BranchRules {
    /// Trimmed, blank patterns dropped, each compiled once to prove it reads:
    /// an invalid one is refused by its position, so the admin learns which.
    pub fn normalized(&self) -> Result<BranchRules, String> {
        let patterns: Vec<String> = self
            .patterns
            .iter()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        if patterns.len() > MAX_BRANCH_PATTERNS {
            return Err(format!("at most {MAX_BRANCH_PATTERNS} branch patterns"));
        }
        let rules = BranchRules { patterns };
        rules.compile()?;
        Ok(rules)
    }

    /// Compile every pattern. A pattern without a `kind` or a `target` group
    /// is allowed: it then reads only the other.
    pub fn compile(&self) -> Result<CompiledBranchRules, String> {
        let mut patterns = Vec::with_capacity(self.patterns.len());
        for (index, pattern) in self.patterns.iter().enumerate() {
            let position = index.saturating_add(1);
            if pattern.chars().count() > MAX_BRANCH_PATTERN_CHARS {
                return Err(format!(
                    "branch pattern {position} is longer than {MAX_BRANCH_PATTERN_CHARS} characters"
                ));
            }
            let regex = RegexBuilder::new(pattern)
                .size_limit(PATTERN_SIZE_LIMIT)
                .build()
                .map_err(|e| {
                    format!("branch pattern {position} is not a regular expression: {e}")
                })?;
            if !regex
                .capture_names()
                .flatten()
                .any(|n| n == "kind" || n == "target")
            {
                return Err(format!(
                    "branch pattern {position} names neither a (?<kind>…) nor a (?<target>…) group"
                ));
            }
            patterns.push(regex);
        }
        Ok(CompiledBranchRules { patterns })
    }
}

impl CompiledBranchRules {
    /// Read a branch: the first pattern that matches gives the kind and the
    /// target; the runner's pull request target, when there was one, wins
    /// over the pattern's. `None` when neither says anything.
    pub fn read(&self, branch: &str, runner_target: Option<&str>) -> Option<BranchReading> {
        let captures = self.patterns.iter().find_map(|p| p.captures(branch));
        let group = |name: &str| {
            captures
                .as_ref()
                .and_then(|c| c.name(name))
                .map(|m| m.as_str().to_string())
                .filter(|v| !v.is_empty())
        };
        let reading = BranchReading {
            kind: group("kind"),
            target: runner_target
                .map(str::to_string)
                .or_else(|| group("target")),
        };
        (reading.kind.is_some() || reading.target.is_some()).then_some(reading)
    }

    /// Fill the reading of one origin; an origin without a branch gets none.
    pub fn read_origin(&self, origin: &mut VersionOrigin) {
        origin.branch_reading = match origin.git_branch.as_deref() {
            Some(branch) => self.read(branch, origin.git_target_branch.as_deref()),
            None => origin.git_target_branch.as_ref().map(|t| BranchReading {
                kind: None,
                target: Some(t.clone()),
            }),
        };
    }
}

/// The stored rules. Unreadable JSON (a hand-edited database) reads as no
/// rules, with a warning, so the pages that show versions keep working.
pub async fn get_branch_rules(repo: &impl SpecRepository) -> Result<BranchRules, AppError> {
    let Some(raw) = repo.get_setting(BRANCH_RULES_SETTING).await? else {
        return Ok(BranchRules::default());
    };
    if raw.trim().is_empty() {
        return Ok(BranchRules::default());
    }
    match serde_json::from_str::<BranchRules>(&raw) {
        Ok(rules) => Ok(rules),
        Err(e) => {
            warn!("The stored branch rules are not readable, ignoring them: {e}");
            Ok(BranchRules::default())
        }
    }
}

/// The stored rules, compiled. A stored pattern that no longer compiles
/// reads as no rules, with a warning.
pub async fn compiled_branch_rules(
    repo: &impl SpecRepository,
) -> Result<CompiledBranchRules, AppError> {
    let rules = get_branch_rules(repo).await?;
    Ok(rules.compile().unwrap_or_else(|e| {
        warn!("The stored branch rules do not compile, ignoring them: {e}");
        CompiledBranchRules::default()
    }))
}

/// Every stored origin by version id, each read with the stored rules: what
/// the pages and answers that show origins list.
pub async fn list_read_version_origins(
    repo: &impl SpecRepository,
) -> Result<std::collections::BTreeMap<i64, VersionOrigin>, AppError> {
    let rules = compiled_branch_rules(repo).await?;
    let mut origins = repo.list_version_origins().await?;
    for origin in origins.values_mut() {
        rules.read_origin(origin);
    }
    Ok(origins)
}

/// Store new rules after checking them; an invalid one is a 400 naming it.
pub async fn set_branch_rules(
    repo: &impl SpecRepository,
    rules: &BranchRules,
) -> Result<BranchRules, AppError> {
    let rules = rules
        .normalized()
        .map_err(|e| AppError::BadRequest(format!("branch rules: {e}")))?;
    let json = serde_json::to_string(&rules)
        .map_err(|e| AppError::Internal(format!("branch rules: {e}")))?;
    repo.set_setting(BRANCH_RULES_SETTING, &json).await?;
    Ok(rules)
}

/// What rules (saved or not) read from a sample branch: the settings page's
/// test field. The server reads it, because a browser's regular expressions
/// do not read like the service's.
pub fn test_branch_rules(
    rules: &BranchRules,
    branch: &str,
    runner_target: Option<&str>,
) -> Result<Option<BranchReading>, AppError> {
    let compiled = rules
        .normalized()
        .and_then(|r| r.compile())
        .map_err(|e| AppError::BadRequest(format!("branch rules: {e}")))?;
    let branch = branch.trim();
    if branch.is_empty() {
        return Err(AppError::BadRequest(
            "branch rules: name a branch to test".to_string(),
        ));
    }
    Ok(compiled.read(
        branch,
        runner_target.map(str::trim).filter(|t| !t.is_empty()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::mock_repo::MockRepo;

    fn rules(patterns: &[&str]) -> BranchRules {
        BranchRules {
            patterns: patterns.iter().map(|p| p.to_string()).collect(),
        }
    }

    const PATTERN: &str = r"^(?<kind>feature|bugfix|hotfix)/(?<target>release/[^/]+|main|master)/";

    #[test]
    fn the_first_matching_pattern_reads_kind_and_target() {
        let compiled = rules(&[PATTERN, r"^(?<kind>release)/"]).compile().unwrap();
        assert_eq!(
            compiled.read("feature/master/tdkydkytdk", None),
            Some(BranchReading {
                kind: Some("feature".to_string()),
                target: Some("master".to_string()),
            })
        );
        assert_eq!(
            compiled.read("bugfix/release/2.1/login", None),
            Some(BranchReading {
                kind: Some("bugfix".to_string()),
                target: Some("release/2.1".to_string()),
            })
        );
        assert_eq!(
            compiled.read("release/2.6", None),
            Some(BranchReading {
                kind: Some("release".to_string()),
                target: None,
            })
        );
        assert_eq!(compiled.read("wip-login", None), None);
    }

    #[test]
    fn the_runners_target_wins_over_the_patterns() {
        let compiled = rules(&[PATTERN]).compile().unwrap();
        let reading = compiled.read("feature/master/x", Some("develop")).unwrap();
        assert_eq!(reading.kind.as_deref(), Some("feature"));
        assert_eq!(reading.target.as_deref(), Some("develop"));
        // Without any pattern, the runner's target alone is a reading.
        let none = CompiledBranchRules::default();
        assert_eq!(
            none.read("x", Some("main")),
            Some(BranchReading {
                kind: None,
                target: Some("main".to_string()),
            })
        );
    }

    #[test]
    fn an_origin_is_read_from_its_branch() {
        let compiled = rules(&[PATTERN]).compile().unwrap();
        let mut origin = VersionOrigin {
            git_branch: Some("hotfix/main/crash".to_string()),
            ..VersionOrigin::default()
        };
        compiled.read_origin(&mut origin);
        assert_eq!(
            origin.branch_reading,
            Some(BranchReading {
                kind: Some("hotfix".to_string()),
                target: Some("main".to_string()),
            })
        );
        let mut bare = VersionOrigin::default();
        compiled.read_origin(&mut bare);
        assert_eq!(bare.branch_reading, None);
    }

    #[test]
    fn invalid_rules_are_refused_by_position() {
        assert_eq!(
            rules(&["(?<kind>x"])
                .normalized()
                .unwrap_err()
                .split(':')
                .next(),
            Some("branch pattern 1 is not a regular expression")
        );
        assert_eq!(
            rules(&[PATTERN, "^release/"]).normalized().unwrap_err(),
            "branch pattern 2 names neither a (?<kind>…) nor a (?<target>…) group"
        );
        let many: Vec<&str> = std::iter::repeat_n(PATTERN, MAX_BRANCH_PATTERNS + 1).collect();
        assert_eq!(
            rules(&many).normalized().unwrap_err(),
            "at most 20 branch patterns"
        );
        let long = format!("(?<kind>{})", "a".repeat(MAX_BRANCH_PATTERN_CHARS));
        assert_eq!(
            rules(&[&long]).normalized().unwrap_err(),
            "branch pattern 1 is longer than 500 characters"
        );
        assert_eq!(
            rules(&["  ", PATTERN]).normalized().unwrap(),
            rules(&[PATTERN])
        );
    }

    #[tokio::test]
    async fn rules_are_stored_read_back_and_tested() {
        let repo = MockRepo::new();
        assert_eq!(
            get_branch_rules(&repo).await.unwrap(),
            BranchRules::default()
        );
        let saved = set_branch_rules(&repo, &rules(&[PATTERN])).await.unwrap();
        assert_eq!(get_branch_rules(&repo).await.unwrap(), saved);
        let compiled = compiled_branch_rules(&repo).await.unwrap();
        assert_eq!(
            compiled.read("feature/main/x", None).and_then(|r| r.target),
            Some("main".to_string())
        );
        let err = set_branch_rules(&repo, &rules(&["("])).await.unwrap_err();
        assert!(
            matches!(err, AppError::BadRequest(m) if m.starts_with("branch rules: branch pattern 1"))
        );
        assert_eq!(
            test_branch_rules(&rules(&[PATTERN]), " bugfix/main/y ", Some(" "))
                .unwrap()
                .and_then(|r| r.kind),
            Some("bugfix".to_string())
        );
        assert!(test_branch_rules(&rules(&[PATTERN]), " ", None).is_err());
    }

    #[tokio::test]
    async fn unreadable_stored_rules_read_as_none() {
        let repo = MockRepo::new();
        repo.set_setting(BRANCH_RULES_SETTING, "not json")
            .await
            .unwrap();
        assert_eq!(
            get_branch_rules(&repo).await.unwrap(),
            BranchRules::default()
        );
        repo.set_setting(BRANCH_RULES_SETTING, r#"{"patterns":["("]}"#)
            .await
            .unwrap();
        let compiled = compiled_branch_rules(&repo).await.unwrap();
        assert_eq!(compiled.read("x", None), None);
    }
}
