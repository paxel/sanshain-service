use crate::domain::models::{AuthenticatedUser, LdapConfig};
use crate::domain::ports::{AuthProvider, AuthProviderError, DirectoryGroups};
use ldap3::{LdapConnAsync, LdapConnSettings, Scope, SearchEntry};
use std::time::Duration;

/// How long a connection attempt may take before it counts as down.
///
/// Without a bound, an unreachable directory (firewalled port, half-dead VM)
/// blocks the caller for the OS's TCP timeout — minutes — and because group
/// resolution runs inside authorisation, that stalls every request from
/// directory-backed users for that long.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Open an LDAP connection using the trust store built at startup.
///
/// The configuration is supplied for every connection, not only when extra CA
/// certificates were mounted. Leaving ldap3 to build its own means two things
/// go wrong: it calls `ClientConfig::builder()`, which panics here because two
/// crypto providers are linked in, and its root store collapses to *empty* if
/// reading the platform certificates hiccups — trusting nothing, silently.
async fn open_connection(
    url: &str,
    starttls: bool,
) -> ldap3::result::Result<(LdapConnAsync, ldap3::Ldap)> {
    let mut settings = LdapConnSettings::new().set_conn_timeout(CONNECT_TIMEOUT);
    // StartTLS upgrades a plain `ldap://` connection to TLS in-band. `ldaps://`
    // is already TLS from the scheme, so it never needs (or wants) this.
    settings = settings.set_starttls(starttls);
    match crate::infrastructure::tls::ldap_client_config() {
        Some(config) => LdapConnAsync::with_settings(settings.set_config(config), url).await,
        // Only when the configuration could not be built at all, which is
        // already logged as an error. Plain LDAP still works on this path.
        None => LdapConnAsync::with_settings(settings, url).await,
    }
}

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

    /// StartTLS applies to a plain `ldap://` URL when the operator asked for
    /// TLS (`use_tls`). `ldaps://` is already encrypted by its scheme.
    fn use_starttls(&self) -> bool {
        self.config.use_tls && self.config.server_url.starts_with("ldap://")
    }

    async fn connect(&self) -> Result<ldap3::Ldap, AuthProviderError> {
        let (conn, mut ldap) = open_connection(&self.config.server_url, self.use_starttls())
            .await
            .map_err(|e| {
                AuthProviderError::ConnectionFailed(format!(
                    "Could not connect to LDAP server at {}: {}",
                    self.config.server_url, e
                ))
            })?;

        ldap3::drive!(conn);

        // Bind with service account
        let bind_pw = self.config.bind_password.as_deref().unwrap_or("");
        ldap.simple_bind(&self.config.bind_dn, bind_pw)
            .await
            .map_err(|e| {
                AuthProviderError::ConnectionFailed(format!(
                    "LDAP bind request failed for {}: {}",
                    self.config.bind_dn, e
                ))
            })?
            .success()
            .map_err(|e| {
                AuthProviderError::ConnectionFailed(format!(
                    "LDAP bind failed (check DN/Password): {}",
                    e
                ))
            })?;

        Ok(ldap)
    }
}

impl AuthProvider for LdapAuthProvider {
    async fn authenticate(
        &self,
        username: &str,
        password: &str,
    ) -> Result<AuthenticatedUser, AuthProviderError> {
        tracing::debug!("LDAP authenticating user: {}", username);
        let mut ldap = self.connect().await?;

        // Search for the user
        let escaped_username = escape_ldap_filter(username);
        let filter = self
            .config
            .user_filter
            .replace("{username}", &escaped_username);
        let (rs, _result) = ldap
            .search(
                &self.config.base_dn,
                Scope::Subtree,
                &filter,
                vec!["dn", "cn", "memberOf"],
            )
            .await
            .map_err(|e| AuthProviderError::Internal(format!("LDAP search: {}", e)))?
            .success()
            .map_err(|e| AuthProviderError::Internal(format!("LDAP search failed: {}", e)))?;

        if rs.is_empty() {
            let _ = ldap.unbind().await;
            return Err(AuthProviderError::InvalidCredentials);
        }

        let entry = rs.into_iter().next().ok_or_else(|| {
            AuthProviderError::Internal("LDAP search results unexpectedly empty".to_string())
        })?;
        let entry = SearchEntry::construct(entry);
        let user_dn = entry.dn;

        // Attempt bind as the user to verify password
        let (conn2, mut user_ldap) = open_connection(&self.config.server_url, self.use_starttls())
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
        let _ = ldap.unbind().await;

        // Authentication ends here. What this user may do comes from their
        // directory groups, read through `groups_for` on every authorisation
        // check — not captured now and frozen, which is what used to leave a
        // promotion or demotion in the directory with no effect.
        Ok(AuthenticatedUser {
            username: username.to_string(),
        })
    }

    async fn test_connection(&self) -> Result<(), AuthProviderError> {
        let mut ldap = self.connect().await?;
        let _ = ldap.unbind().await;
        Ok(())
    }
}

impl DirectoryGroups for LdapAuthProvider {
    /// The directory groups a user belongs to, read through the service account.
    ///
    /// Deliberately does not need the user's password: privileges have to stay
    /// current between logins, and Sanshain does not retain credentials. The
    /// bind DN the configuration already carries is what makes that possible.
    async fn groups_for(&self, username: &str) -> Result<Vec<String>, AuthProviderError> {
        let mut ldap = self.connect().await?;

        let escaped_username = escape_ldap_filter(username);
        let filter = self
            .config
            .user_filter
            .replace("{username}", &escaped_username);
        let (rs, _result) = ldap
            .search(
                &self.config.base_dn,
                Scope::Subtree,
                &filter,
                vec!["dn", "memberOf"],
            )
            .await
            .map_err(|e| AuthProviderError::Internal(format!("LDAP search: {}", e)))?
            .success()
            .map_err(|e| AuthProviderError::Internal(format!("LDAP search failed: {}", e)))?;

        let groups = rs
            .into_iter()
            .next()
            .map(SearchEntry::construct)
            .and_then(|entry| entry.attrs.get("memberOf").cloned())
            .unwrap_or_default();

        let _ = ldap.unbind().await;
        Ok(groups)
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
