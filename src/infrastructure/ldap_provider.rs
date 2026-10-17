use crate::domain::models::{AuthenticatedUser, LdapConfig};
use crate::domain::ports::{AuthProvider, AuthProviderError, DirectoryGroups};
use ldap3::{LdapConnAsync, LdapConnSettings, LdapError, Scope, SearchEntry, StdStream};
use std::time::Duration;
use tokio::net::TcpStream;

/// How long a connection attempt may take before it counts as down.
///
/// Without a bound, an unreachable directory (firewalled port, half-dead VM)
/// blocks the caller for the OS's TCP timeout — minutes — and because group
/// resolution runs inside authorisation, that stalls every request from
/// directory-backed users for that long.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long any single exchange with the directory may take once the TCP
/// connection is up: the TLS or StartTLS handshake, a bind, a search, the
/// unbind.
///
/// The connect timeout does not help once the port answers. A scheme and port
/// mismatch (`ldaps://` on 389, StartTLS on 636) or a firewall that accepts
/// and then stalls gets past it, and without this bound the request hangs
/// until the ingress gives up — the admin's "Test connection" then shows
/// "Bad Gateway" instead of anything useful. Overridden by
/// [`OPERATION_TIMEOUT_ENV`].
const OPERATION_TIMEOUT: Duration = Duration::from_secs(10);

/// Environment variable overriding [`OPERATION_TIMEOUT`], in whole seconds.
pub const OPERATION_TIMEOUT_ENV: &str = "LDAP_OPERATION_TIMEOUT_SECS";

/// The operation timeout from the environment, or the default when the
/// variable is unset, not a number, or zero.
fn operation_timeout_from_env() -> Duration {
    std::env::var(OPERATION_TIMEOUT_ENV)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .map_or(OPERATION_TIMEOUT, Duration::from_secs)
}

/// LDAP authentication provider.
pub struct LdapAuthProvider {
    config: LdapConfig,
    operation_timeout: Duration,
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

/// The `host:port` a TCP connect needs, with the scheme's default port when
/// the URL names none — the same defaults ldap3 applies.
fn host_port(url: &url::Url) -> String {
    let default_port = if url.scheme() == "ldaps" { 636 } else { 389 };
    let host = match url.host_str() {
        Some("") | None => "localhost",
        Some(host) => host,
    };
    format!("{}:{}", host, url.port().unwrap_or(default_port))
}

impl LdapAuthProvider {
    pub fn new(config: LdapConfig) -> Self {
        Self {
            config,
            operation_timeout: operation_timeout_from_env(),
        }
    }

    /// Replace the operation timeout read from the environment.
    pub fn with_operation_timeout(mut self, timeout: Duration) -> Self {
        self.operation_timeout = timeout;
        self
    }

    /// StartTLS applies to a plain `ldap://` URL when the operator asked for
    /// TLS (`use_tls`). `ldaps://` is already encrypted by its scheme.
    fn use_starttls(&self) -> bool {
        self.config.use_tls && self.config.server_url.starts_with("ldap://")
    }

    /// The error for a directory that accepted the TCP connection and then
    /// never answered within the operation timeout.
    fn stalled(&self) -> AuthProviderError {
        AuthProviderError::ConnectionFailed(format!(
            "LDAP server at {} accepted the connection but did not respond within {} s. \
             Check that ldaps:// uses the TLS port and StartTLS the plain one.",
            self.config.server_url,
            self.operation_timeout.as_secs_f64()
        ))
    }

    /// A timed-out operation is the stalled-directory error; anything else
    /// is whatever `otherwise` makes of it.
    fn map_operation_error(
        &self,
        error: LdapError,
        otherwise: impl FnOnce(LdapError) -> AuthProviderError,
    ) -> AuthProviderError {
        match error {
            LdapError::Timeout { .. } => self.stalled(),
            other => otherwise(other),
        }
    }

    /// Open an LDAP connection using the trust store built at startup.
    ///
    /// The TCP connect runs under [`CONNECT_TIMEOUT`] here rather than inside
    /// ldap3, whose connect timeout would also cover the handshake and so
    /// could not tell "nothing is listening" from "it accepted and went
    /// quiet". Everything after the connect — the StartTLS or TLS handshake —
    /// runs under the operation timeout and reports the stalled message.
    ///
    /// The TLS configuration is supplied for every connection, not only when
    /// extra CA certificates were mounted. Leaving ldap3 to build its own
    /// means two things go wrong: it calls `ClientConfig::builder()`, which
    /// panics here because two crypto providers are linked in, and its root
    /// store collapses to *empty* if reading the platform certificates
    /// hiccups — trusting nothing, silently.
    #[tracing::instrument(skip_all)]
    async fn open(&self) -> Result<(LdapConnAsync, ldap3::Ldap), AuthProviderError> {
        let url = &self.config.server_url;
        let could_not_connect = |reason: String| {
            AuthProviderError::ConnectionFailed(format!(
                "Could not connect to LDAP server at {}: {}",
                url, reason
            ))
        };

        let parsed = url::Url::parse(url).map_err(|e| could_not_connect(e.to_string()))?;
        let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(host_port(&parsed)))
            .await
            .map_err(|_| {
                could_not_connect(format!(
                    "no answer within {} s",
                    CONNECT_TIMEOUT.as_secs_f64()
                ))
            })?
            .map_err(|e| could_not_connect(e.to_string()))?;
        let stream = stream
            .into_std()
            .map_err(|e| could_not_connect(e.to_string()))?;

        // StartTLS upgrades a plain `ldap://` connection to TLS in-band.
        // `ldaps://` is already TLS from the scheme, so it never needs (or
        // wants) this.
        let mut settings = LdapConnSettings::new()
            .set_starttls(self.use_starttls())
            .set_std_stream(StdStream::Tcp(stream));
        // `None` only when the configuration could not be built at all, which
        // is already logged as an error. Plain LDAP still works on this path.
        if let Some(config) = crate::infrastructure::tls::ldap_client_config() {
            settings = settings.set_config(config);
        }

        // No `set_conn_timeout`: ldap3 would wrap the very same future in the
        // very same `tokio::time::timeout`. For StartTLS, ldap3 drives the
        // exchange from a task it spawns; dropping this future drops the
        // `Ldap` handle, the task's request channel closes, the task ends and
        // the socket with it (`ldap_timeout_test` checks the close).
        tokio::time::timeout(
            self.operation_timeout,
            LdapConnAsync::with_settings(settings, url),
        )
        .await
        .map_err(|_| self.stalled())?
        .map_err(|e| could_not_connect(e.to_string()))
    }

    #[tracing::instrument(skip_all)]
    async fn connect(&self) -> Result<ldap3::Ldap, AuthProviderError> {
        let (conn, mut ldap) = self.open().await?;

        ldap3::drive!(conn);

        // Bind with service account
        let bind_pw = self.config.bind_password.as_deref().unwrap_or("");
        ldap.with_timeout(self.operation_timeout)
            .simple_bind(&self.config.bind_dn, bind_pw)
            .await
            .map_err(|e| {
                self.map_operation_error(e, |e| {
                    AuthProviderError::ConnectionFailed(format!(
                        "LDAP bind request failed for {}: {}",
                        self.config.bind_dn, e
                    ))
                })
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

    /// Search for the user's entry through the service-account connection.
    async fn search_user(
        &self,
        ldap: &mut ldap3::Ldap,
        username: &str,
        attrs: Vec<&str>,
    ) -> Result<Vec<ldap3::ResultEntry>, AuthProviderError> {
        let escaped_username = escape_ldap_filter(username);
        let filter = self
            .config
            .user_filter
            .replace("{username}", &escaped_username);
        let (rs, _result) = ldap
            .with_timeout(self.operation_timeout)
            .search(&self.config.base_dn, Scope::Subtree, &filter, attrs)
            .await
            .map_err(|e| {
                self.map_operation_error(e, |e| {
                    AuthProviderError::Internal(format!("LDAP search: {}", e))
                })
            })?
            .success()
            .map_err(|e| AuthProviderError::Internal(format!("LDAP search failed: {}", e)))?;
        Ok(rs)
    }

    /// Close the connection; a directory that stalls on the unbind is not
    /// worth waiting for, and the result is irrelevant either way.
    async fn unbind(&self, ldap: &mut ldap3::Ldap) {
        let _ = ldap.with_timeout(self.operation_timeout).unbind().await;
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
        let rs = self
            .search_user(&mut ldap, username, vec!["dn", "cn", "memberOf"])
            .await?;

        if rs.is_empty() {
            self.unbind(&mut ldap).await;
            return Err(AuthProviderError::InvalidCredentials);
        }

        let entry = rs.into_iter().next().ok_or_else(|| {
            AuthProviderError::Internal("LDAP search results unexpectedly empty".to_string())
        })?;
        let entry = SearchEntry::construct(entry);
        let user_dn = entry.dn;

        // Attempt bind as the user to verify password
        let (conn2, mut user_ldap) = self.open().await?;
        ldap3::drive!(conn2);

        user_ldap
            .with_timeout(self.operation_timeout)
            .simple_bind(&user_dn, password)
            .await
            .map_err(|e| self.map_operation_error(e, |_| AuthProviderError::InvalidCredentials))?
            .success()
            .map_err(|_| AuthProviderError::InvalidCredentials)?;

        self.unbind(&mut user_ldap).await;
        self.unbind(&mut ldap).await;

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
        self.unbind(&mut ldap).await;
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

        let rs = self
            .search_user(&mut ldap, username, vec!["dn", "memberOf"])
            .await?;

        let groups = rs
            .into_iter()
            .next()
            .map(SearchEntry::construct)
            .and_then(|entry| entry.attrs.get("memberOf").cloned())
            .unwrap_or_default();

        self.unbind(&mut ldap).await;
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

    fn parsed(url: &str) -> url::Url {
        url::Url::parse(url).expect("valid test URL")
    }

    #[test]
    fn host_port_applies_the_scheme_default_when_the_url_names_no_port() {
        assert_eq!(
            host_port(&parsed("ldap://directory.example")),
            "directory.example:389"
        );
        assert_eq!(
            host_port(&parsed("ldaps://directory.example")),
            "directory.example:636"
        );
    }

    #[test]
    fn host_port_keeps_an_explicit_port_and_ipv6_brackets() {
        assert_eq!(
            host_port(&parsed("ldaps://directory.example:389")),
            "directory.example:389"
        );
        assert_eq!(host_port(&parsed("ldap://[::1]:1389")), "[::1]:1389");
    }

    #[test]
    fn host_port_falls_back_to_localhost_for_an_empty_host() {
        assert_eq!(host_port(&parsed("ldap:///")), "localhost:389");
    }
}
