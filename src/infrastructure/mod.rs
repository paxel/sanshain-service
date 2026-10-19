pub mod cached_repository;
pub mod database;
pub mod ldap_provider;
pub mod local_auth_provider;
pub mod net_diag;
pub mod oidc_provider;
pub(crate) mod pin_rows;
pub mod postgres_repository;
pub mod sqlite_repository;
pub mod sqlite_snapshot;
pub mod telemetry;
pub mod tls;
pub(crate) mod webhook_rows;
pub mod webhook_sender;

/// Turn a `find_endpoint` query into a case-insensitive SQL `LIKE` pattern:
/// the query matches as a substring, `*` matches anything, and the `LIKE`
/// metacharacters in the query itself are escaped (`\` is the escape
/// character, declared with `ESCAPE` by every query using this).
pub(crate) fn like_pattern(query: &str) -> String {
    let mut pattern = String::from("%");
    for c in query.trim().to_lowercase().chars() {
        match c {
            '*' => pattern.push('%'),
            '%' | '_' | '\\' => {
                pattern.push('\\');
                pattern.push(c);
            }
            other => pattern.push(other),
        }
    }
    pattern.push('%');
    pattern
}

#[cfg(test)]
mod tests {
    use super::like_pattern;

    #[test]
    fn like_pattern_lowercases_wildcards_and_escapes() {
        assert_eq!(like_pattern("Users"), "%users%");
        assert_eq!(like_pattern("/users/*/orders"), "%/users/%/orders%");
        assert_eq!(like_pattern("a_b%c\\d"), "%a\\_b\\%c\\\\d%");
        assert_eq!(like_pattern("  spaced "), "%spaced%");
    }
}
