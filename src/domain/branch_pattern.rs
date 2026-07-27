//! Protected-branch pattern matching.
//!
//! A protected-branch entry is a glob pattern: `*` matches any sequence of
//! characters (including none) and `?` matches exactly one. Everything else is
//! literal — notably `_` and `%`, which are *not* wildcards.
//!
//! This lives in the domain layer and is applied in Rust rather than in SQL so
//! both backends agree on what a pattern means. Previously each backend
//! delegated to its own dialect (SQLite `GLOB`, PostgreSQL `LIKE`), which made
//! `release/*` protect on SQLite only, while `feature_x` silently behaved as a
//! wildcard on PostgreSQL alone.

/// True when `name` matches the protected-branch glob `pattern`.
///
/// Patterns without `*` or `?` compare literally, so ordinary branch names
/// (`main`, `master`, `release/1.0`) match only themselves.
pub fn branch_matches_pattern(name: &str, pattern: &str) -> bool {
    if !pattern.contains('*') && !pattern.contains('?') {
        return name == pattern;
    }

    let name: Vec<char> = name.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    let (mut n, mut p) = (0usize, 0usize);
    // Position of the most recent `*` in the pattern, and how much of `name` it
    // had consumed at that point, so a failed match can backtrack and let the
    // star swallow one more character instead of giving up.
    let mut last_star: Option<usize> = None;
    let mut consumed_by_star = 0usize;

    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) {
            n += 1;
            p += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            last_star = Some(p);
            consumed_by_star = n;
            p += 1;
        } else if let Some(star) = last_star {
            p = star + 1;
            consumed_by_star += 1;
            n = consumed_by_star;
        } else {
            return false;
        }
    }

    // Trailing stars may match the empty remainder.
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// True when `name` matches any of `patterns`.
pub fn branch_matches_any(name: &str, patterns: &[String]) -> bool {
    patterns
        .iter()
        .any(|pattern| branch_matches_pattern(name, pattern))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_patterns_match_only_themselves() {
        assert!(branch_matches_pattern("main", "main"));
        assert!(!branch_matches_pattern("maintenance", "main"));
        assert!(!branch_matches_pattern("main", "master"));
        assert!(branch_matches_pattern("release/1.0", "release/1.0"));
    }

    #[test]
    fn star_matches_any_sequence_including_empty() {
        assert!(branch_matches_pattern("release/1.0", "release/*"));
        assert!(branch_matches_pattern("release/", "release/*"));
        assert!(branch_matches_pattern("release/1.0/rc1", "release/*"));
        assert!(!branch_matches_pattern("hotfix/1.0", "release/*"));
        assert!(branch_matches_pattern("anything", "*"));
    }

    #[test]
    fn question_mark_matches_exactly_one_character() {
        assert!(branch_matches_pattern("release/1", "release/?"));
        assert!(!branch_matches_pattern("release/10", "release/?"));
        assert!(!branch_matches_pattern("release/", "release/?"));
    }

    #[test]
    fn underscore_and_percent_are_literal_not_wildcards() {
        // The PostgreSQL `LIKE` hazard: `_` must not match an arbitrary
        // character, and `%` must not match a sequence.
        assert!(branch_matches_pattern("feature_x", "feature_x"));
        assert!(!branch_matches_pattern("featureAx", "feature_x"));
        assert!(!branch_matches_pattern("feature/anything", "feature%"));
        assert!(branch_matches_pattern("feature%", "feature%"));
    }

    #[test]
    fn stars_backtrack_correctly() {
        assert!(branch_matches_pattern(
            "release/1.0-final",
            "release/*final"
        ));
        assert!(!branch_matches_pattern("release/1.0-final", "release/*rc"));
        assert!(branch_matches_pattern("a/b/c", "a/*/c"));
        assert!(branch_matches_pattern("aXbXc", "a*b*c"));
        assert!(!branch_matches_pattern("aXbXd", "a*b*c"));
    }

    #[test]
    fn matches_any_checks_every_pattern() {
        let patterns = vec!["main".to_string(), "release/*".to_string()];
        assert!(branch_matches_any("main", &patterns));
        assert!(branch_matches_any("release/2.1", &patterns));
        assert!(!branch_matches_any("feature/x", &patterns));
        assert!(!branch_matches_any("main", &[]));
    }
}
