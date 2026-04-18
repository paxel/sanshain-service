use crate::domain::models::{AuthenticatedUser, LdapConfig};
use crate::domain::ports::{AuthProvider, AuthProviderError};
use ldap3::{LdapConnAsync, Scope, SearchEntry};

/// LDAP authentication provider.
pub struct LdapAuthProvider {
    config: LdapConfig,
}

fn escape_ldap_filter(val: &str) -> String {
    let mut escaped = String::with_capacity(val.len());
    for c in val.chars() {
        match c {
            '\\' => escaped.push_str("\\5c"),
            '*' => escaped.push_str("\\2a"),
            '(' => escaped.push_str("\\28"),
            ')' => escaped.push_str("\\29"),
            '\0' => escaped.push_str("\\00"),
            _ => escaped.push(c),
        }
    }
    escaped
}

impl LdapAuthProvider {
    pub fn new(config: LdapConfig) -> Self {
        Self { config }
    }

    async fn connect(&self) -> Result<ldap3::Ldap, AuthProviderError> {
        let (conn, mut ldap) = LdapConnAsync::new(&self.config.server_url)
            .await
            .map_err(|e| AuthProviderError::ConnectionFailed(format!("LDAP connect: {}", e)))?;

        ldap3::drive!(conn);

        // Bind with service account
        let bind_pw = self.config.bind_password.as_deref().unwrap_or("");
        ldap.simple_bind(&self.config.bind_dn, bind_pw)
            .await
            .map_err(|e| AuthProviderError::ConnectionFailed(format!("LDAP bind: {}", e)))?
            .success()
            .map_err(|e| AuthProviderError::ConnectionFailed(format!("LDAP bind failed: {}", e)))?;

        Ok(ldap)
    }
}

impl AuthProvider for LdapAuthProvider {
    async fn authenticate(
        &self,
        username: &str,
        password: &str,
    ) -> Result<AuthenticatedUser, AuthProviderError> {
        let mut ldap = self.connect().await?;

        // Search for the user
        let escaped_username = escape_ldap_filter(username);
        let filter = self.config.user_filter.replace("{username}", &escaped_username);
        let (rs, _result) = ldap
            .search(&self.config.base_dn, Scope::Subtree, &filter, vec!["dn", "cn", "memberOf"])
            .await
            .map_err(|e| AuthProviderError::Internal(format!("LDAP search: {}", e)))?
            .success()
            .map_err(|e| AuthProviderError::Internal(format!("LDAP search failed: {}", e)))?;

        if rs.is_empty() {
            let _ = ldap.unbind().await;
            return Err(AuthProviderError::InvalidCredentials);
        }

        let entry = SearchEntry::construct(rs.into_iter().next().unwrap());
        let user_dn = entry.dn;

        // Attempt bind as the user to verify password
        let (conn2, mut user_ldap) = LdapConnAsync::new(&self.config.server_url)
            .await
            .map_err(|e| AuthProviderError::ConnectionFailed(format!("LDAP connect: {}", e)))?;
        ldap3::drive!(conn2);

        user_ldap
            .simple_bind(&user_dn, password)
            .await
            .map_err(|_| AuthProviderError::InvalidCredentials)?
            .success()
            .map_err(|_| AuthProviderError::InvalidCredentials)?;

        let _ = user_ldap.unbind().await;

        // Determine admin status from group membership
        let is_admin = if !self.config.admin_group.is_empty() {
            entry
                .attrs
                .get("memberOf")
                .map(|groups| groups.iter().any(|g| g == &self.config.admin_group))
                .unwrap_or(false)
        } else {
            false
        };

        let _ = ldap.unbind().await;

        Ok(AuthenticatedUser {
            username: username.to_string(),
            is_admin,
        })
    }

    async fn test_connection(&self) -> Result<(), AuthProviderError> {
        let mut ldap = self.connect().await?;
        let _ = ldap.unbind().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_ldap_filter() {
        assert_eq!(escape_ldap_filter("user"), "user");
        assert_eq!(escape_ldap_filter("user*"), "user\\2a");
        assert_eq!(escape_ldap_filter("user("), "user\\28");
        assert_eq!(escape_ldap_filter("user)"), "user\\29");
        assert_eq!(escape_ldap_filter("user\\"), "user\\5c");
        assert_eq!(escape_ldap_filter("user\0"), "user\\00");
        assert_eq!(
            escape_ldap_filter("admin)(|(user=*"),
            "admin\\29\\28|\\28user=\\2a"
        );
    }
}
