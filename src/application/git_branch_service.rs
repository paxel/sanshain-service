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

/// The kinds that make a branch a maintenance branch when no admin said
/// otherwise (san-1030).
pub const DEFAULT_MAINTENANCE_KINDS: [&str; 3] = ["release", "bugfix", "hotfix"];
/// At most this many maintenance kinds.
pub const MAX_MAINTENANCE_KINDS: usize = 20;

fn default_maintenance_kinds() -> Vec<String> {
    DEFAULT_MAINTENANCE_KINDS
        .iter()
        .map(|k| k.to_string())
        .collect()
}

/// The branch rules as the admin wrote them.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BranchRules {
    /// Regular expressions with the named groups `kind` and `target`, tried
    /// in order; the first that matches a branch reads it.
    #[serde(default)]
    pub patterns: Vec<String>,
    /// The `kind`s that make a branch a maintenance branch (san-1030).
    #[serde(default = "default_maintenance_kinds")]
    pub maintenance_kinds: Vec<String>,
    /// A regular expression that makes a matching branch a maintenance
    /// branch, whatever its kind (san-1030).
    #[serde(default)]
    pub maintenance_pattern: Option<String>,
}

impl Default for BranchRules {
    fn default() -> Self {
        BranchRules {
            patterns: Vec::new(),
            maintenance_kinds: default_maintenance_kinds(),
            maintenance_pattern: None,
        }
    }
}

/// The rules ready to read branches with.
#[derive(Debug)]
pub struct CompiledBranchRules {
    patterns: Vec<Regex>,
    maintenance_kinds: Vec<String>,
    maintenance_pattern: Option<Regex>,
}

impl Default for CompiledBranchRules {
    fn default() -> Self {
        CompiledBranchRules {
            patterns: Vec::new(),
            maintenance_kinds: default_maintenance_kinds(),
            maintenance_pattern: None,
        }
    }
}

/// One pattern compiled under the size limit.
fn compile_pattern(pattern: &str, what: &str) -> Result<Regex, String> {
    if pattern.chars().count() > MAX_BRANCH_PATTERN_CHARS {
        return Err(format!(
            "{what} is longer than {MAX_BRANCH_PATTERN_CHARS} characters"
        ));
    }
    RegexBuilder::new(pattern)
        .size_limit(PATTERN_SIZE_LIMIT)
        .build()
        .map_err(|e| format!("{what} is not a regular expression: {e}"))
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
        let mut maintenance_kinds: Vec<String> = Vec::new();
        for kind in self.maintenance_kinds.iter().map(|k| k.trim()) {
            if !kind.is_empty() && !maintenance_kinds.iter().any(|k| k == kind) {
                maintenance_kinds.push(kind.to_string());
            }
        }
        if maintenance_kinds.len() > MAX_MAINTENANCE_KINDS {
            return Err(format!("at most {MAX_MAINTENANCE_KINDS} maintenance kinds"));
        }
        let maintenance_pattern = self
            .maintenance_pattern
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string);
        let rules = BranchRules {
            patterns,
            maintenance_kinds,
            maintenance_pattern,
        };
        rules.compile()?;
        Ok(rules)
    }

    /// Compile every pattern. A pattern without a `kind` or a `target` group
    /// is allowed: it then reads only the other.
    pub fn compile(&self) -> Result<CompiledBranchRules, String> {
        let mut patterns = Vec::with_capacity(self.patterns.len());
        for (index, pattern) in self.patterns.iter().enumerate() {
            let position = index.saturating_add(1);
            let regex = compile_pattern(pattern, &format!("branch pattern {position}"))?;
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
        let maintenance_pattern = self
            .maintenance_pattern
            .as_deref()
            .filter(|p| !p.trim().is_empty())
            .map(|p| compile_pattern(p, "the maintenance branch pattern"))
            .transpose()?;
        Ok(CompiledBranchRules {
            patterns,
            maintenance_kinds: self.maintenance_kinds.clone(),
            maintenance_pattern,
        })
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

    /// Whether a provide is on a maintenance branch (san-1030), a side line
    /// of semver whose refusals propose patches: its branch's `kind` is a
    /// maintenance kind, or the maintenance pattern matches its branch, or
    /// it carries a Sanshain stream tag. Any one is enough.
    pub fn is_maintenance(&self, origin: &VersionOrigin, tagged: bool) -> bool {
        if tagged {
            return true;
        }
        let Some(branch) = origin.git_branch.as_deref() else {
            return false;
        };
        let kind = self
            .read(branch, origin.git_target_branch.as_deref())
            .and_then(|r| r.kind);
        kind.is_some_and(|k| self.maintenance_kinds.contains(&k))
            || self
                .maintenance_pattern
                .as_ref()
                .is_some_and(|p| p.is_match(branch))
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

/// What rules (saved or not) read from a sample branch, and whether it is a
/// maintenance branch: the settings page's test field. The server reads it,
/// because a browser's regular expressions do not read like the service's.
pub fn test_branch_rules(
    rules: &BranchRules,
    branch: &str,
    runner_target: Option<&str>,
) -> Result<(Option<BranchReading>, bool), AppError> {
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
    let origin = VersionOrigin {
        git_branch: Some(branch.to_string()),
        git_target_branch: runner_target
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string),
        ..VersionOrigin::default()
    };
    Ok((
        compiled.read(branch, origin.git_target_branch.as_deref()),
        compiled.is_maintenance(&origin, false),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::mock_repo::MockRepo;

    fn rules(patterns: &[&str]) -> BranchRules {
        BranchRules {
            patterns: patterns.iter().map(|p| p.to_string()).collect(),
            ..BranchRules::default()
        }
    }

    #[test]
    fn a_maintenance_branch_is_a_kind_a_pattern_or_a_tag() {
        let origin = |branch: Option<&str>| VersionOrigin {
            git_branch: branch.map(str::to_string),
            ..VersionOrigin::default()
        };
        let compiled = BranchRules {
            maintenance_pattern: Some("^support/".to_string()),
            ..rules(&[PATTERN, r"^(?<kind>release)/"])
        }
        .normalized()
        .unwrap()
        .compile()
        .unwrap();
        assert!(compiled.is_maintenance(&origin(Some("release/2.6")), false));
        assert!(compiled.is_maintenance(&origin(Some("bugfix/master/npe")), false));
        assert!(compiled.is_maintenance(&origin(Some("support/acme")), false));
        assert!(!compiled.is_maintenance(&origin(Some("feature/master/login")), false));
        assert!(!compiled.is_maintenance(&origin(Some("master")), false));
        assert!(!compiled.is_maintenance(&origin(None), false));
        assert!(
            compiled.is_maintenance(&origin(None), true),
            "a tag is enough"
        );
        // Own kinds replace the defaults.
        let own = BranchRules {
            maintenance_kinds: vec![" feature ".to_string(), "feature".to_string()],
            ..rules(&[PATTERN])
        }
        .normalized()
        .unwrap();
        assert_eq!(own.maintenance_kinds, vec!["feature".to_string()]);
        let own = own.compile().unwrap();
        assert!(own.is_maintenance(&origin(Some("feature/master/login")), false));
        assert!(!own.is_maintenance(&origin(Some("hotfix/master/x")), false));
        assert_eq!(
            BranchRules {
                maintenance_pattern: Some("(".to_string()),
                ..BranchRules::default()
            }
            .normalized()
            .unwrap_err()
            .split(':')
            .next(),
            Some("the maintenance branch pattern is not a regular expression")
        );
        // Rules stored before the maintenance settings read the defaults.
        let old: BranchRules = serde_json::from_str(r#"{"patterns": []}"#).unwrap();
        assert_eq!(old.maintenance_kinds, vec!["release", "bugfix", "hotfix"]);
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
            test_branch_rules(&rules(&[PATTERN]), " bugfix/main/y ", Some(" ")).unwrap(),
            (
                Some(BranchReading {
                    kind: Some("bugfix".to_string()),
                    target: Some("main".to_string()),
                }),
                true
            )
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
