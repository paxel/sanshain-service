use crate::domain::models::{AuthenticatedUser, LdapConfig};
use crate::domain::ports::{AuthProvider, AuthProviderError, DirectoryGroups};
use ldap3::{
    LdapConnAsync, LdapConnSettings, LdapError, Scope, SearchEntry, SearchOptions, StdStream,
};
use std::io;
use std::net::SocketAddr;
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

/// The host and port a TCP connect needs, with the scheme's default port
/// when the URL names none — the same defaults ldap3 applies.
fn host_and_port(url: &url::Url) -> (String, u16) {
    let default_port = if url.scheme() == "ldaps" { 636 } else { 389 };
    let host = match url.host_str() {
        Some("") | None => "localhost",
        Some(host) => host,
    };
    (host.to_string(), url.port().unwrap_or(default_port))
}

/// `host:port` for the resolver, from [`host_and_port`].
fn host_port(url: &url::Url) -> String {
    let (host, port) = host_and_port(url);
    format!("{host}:{port}")
}

/// The host and port a server URL points at, for the network diagnosis of an
/// unreachable directory; `None` when the URL does not parse.
pub fn directory_host_and_port(server_url: &str) -> Option<(String, u16)> {
    url::Url::parse(server_url).ok().map(|u| host_and_port(&u))
}

/// The plain-language reason a TLS or StartTLS handshake failed, with where to
/// look next, recognised from the rustls text ldap3 passes through; anything
/// unrecognised is kept as it came.
fn explain_handshake_error(text: &str) -> String {
    let explained = if text.contains("UnknownIssuer") {
        "the server certificate is not trusted: its CA is not in the trust store; \
         mount the CA with EXTRA_CA_CERTS_DIR"
    } else if text.contains("NotValidForName") {
        "the server certificate is not valid for the hostname in the URL: \
         use the name the certificate carries, or reissue it for this one"
    } else if text.contains("Expired") {
        "the server certificate has expired: renew it on the directory"
    } else if text.contains("NotValidYet") {
        "the server certificate is not valid yet: check the clocks of the directory and this host"
    } else if text.contains("corrupt message") || text.contains("InvalidMessage") {
        "the server did not answer with TLS: ldaps:// points at a plain LDAP port; \
         use the TLS port, or ldap:// with StartTLS"
    } else if text.contains("fatal alert") {
        "the server refused the TLS handshake: \
         check which TLS versions and ciphers the directory accepts"
    } else if text.contains("reset")
        || text.contains("unexpected EOF")
        || text.contains("UnexpectedEof")
    {
        "the server closed the connection during the handshake: \
         StartTLS on a TLS port does this; use ldaps:// for that port"
    } else {
        return text.to_string();
    };
    format!("{explained} ({text})")
}

/// The plain-language reason a TCP connect to `addr` failed, with where to
/// look next. The OS text alone (`No route to host (os error 113)`) tells an
/// admin nothing about which side of the network to check.
fn explain_connect_error(addr: SocketAddr, error: &io::Error) -> String {
    let host = addr.ip();
    match error.kind() {
        io::ErrorKind::ConnectionRefused => format!(
            "nothing listens on {addr}: the host is up but the port is closed; \
             check the port and that the directory is running"
        ),
        io::ErrorKind::HostUnreachable => format!(
            "this container has no route to {host}: \
             check the network, VPN or Docker network of the pod"
        ),
        io::ErrorKind::NetworkUnreachable => format!(
            "the network of {host} is unreachable from this container: \
             check its routes and, for an IPv6 address, whether the pod has IPv6 at all"
        ),
        io::ErrorKind::TimedOut => format!(
            "no answer from {addr} within {} s: the host is down or a firewall drops the \
             connection; check host, port and firewall",
            CONNECT_TIMEOUT.as_secs_f64()
        ),
        io::ErrorKind::PermissionDenied => format!(
            "the container may not connect to {addr}: a policy or firewall on this host refuses it"
        ),
        _ => format!("connecting to {addr} failed: {error}"),
    }
}

/// Resolve `host_port` and bring up a TCP connection to the first address
/// that accepts one, all within [`CONNECT_TIMEOUT`].
///
/// Every failure here is [`AuthProviderError::Unreachable`]: the directory
/// never got a say. The addresses are tried in resolver order, as ldap3 would,
/// and the message names the last address that failed.
async fn connect_tcp(url: &str, host_port: &str) -> Result<TcpStream, AuthProviderError> {
    let unreachable = |reason: String| {
        AuthProviderError::Unreachable(format!(
            "Could not connect to LDAP server at {}: {}",
            url, reason
        ))
    };
    let deadline = tokio::time::Instant::now() + CONNECT_TIMEOUT;

    let addrs: Vec<SocketAddr> =
        match tokio::time::timeout_at(deadline, tokio::net::lookup_host(host_port)).await {
            Ok(Ok(addrs)) => addrs.collect(),
            Ok(Err(e)) => {
                return Err(unreachable(format!(
                    "the hostname could not be resolved ({e}); \
                     check the name and the DNS resolvers of this container"
                )));
            }
            Err(_) => {
                return Err(unreachable(format!(
                    "resolving the hostname took longer than {} s; \
                     check the DNS resolvers of this container",
                    CONNECT_TIMEOUT.as_secs_f64()
                )));
            }
        };
    let Some(first) = addrs.first() else {
        return Err(unreachable(
            "the hostname resolved to no address; check the name".to_string(),
        ));
    };

    let mut last = (*first, io::Error::from(io::ErrorKind::TimedOut));
    for addr in &addrs {
        match tokio::time::timeout_at(deadline, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(e)) => last = (*addr, e),
            Err(_) => {
                last = (*addr, io::Error::from(io::ErrorKind::TimedOut));
                break;
            }
        }
    }
    Err(unreachable(explain_connect_error(last.0, &last.1)))
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
        let stream = connect_tcp(url, &host_port(&parsed)).await?;
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
        .map_err(|e| could_not_connect(explain_handshake_error(&e.to_string())))
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
            .search_user(
                &mut ldap,
                username,
                vec!["dn", "cn", "memberOf", "displayName", "mail"],
            )
            .await?;

        if rs.is_empty() {
            self.unbind(&mut ldap).await;
            return Err(AuthProviderError::InvalidCredentials);
        }

        let entry = rs.into_iter().next().ok_or_else(|| {
            AuthProviderError::Internal("LDAP search results unexpectedly empty".to_string())
        })?;
        let entry = SearchEntry::construct(entry);
        let display_name = entry
            .attrs
            .get("displayName")
            .and_then(|values| values.first())
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty());
        let email = entry
            .attrs
            .get("mail")
            .and_then(|values| values.first())
            .map(|mail| mail.trim().to_string())
            .filter(|mail| !mail.is_empty());
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
            display_name,
            email,
        })
    }

    async fn test_connection(&self) -> Result<(), AuthProviderError> {
        let mut ldap = self.connect().await?;
        self.unbind(&mut ldap).await;
        Ok(())
    }
}

/// The object classes a group has: `group` in Active Directory,
/// `groupOfNames`, `groupOfUniqueNames` or `posixGroup` in OpenLDAP.
const GROUP_CLASSES: &str = "(|(objectClass=group)(objectClass=groupOfNames)(objectClass=groupOfUniqueNames)(objectClass=posixGroup))";

/// LDAP result code for a search that hit its size limit: the entries up to
/// the limit are still valid.
const SIZE_LIMIT_EXCEEDED: u32 = 4;

impl crate::domain::ports::DirectoryGroupSearch for LdapAuthProvider {
    /// Searched through the service account, under the base DN, by a
    /// substring of the common name.
    async fn find_groups(
        &self,
        text: &str,
        limit: i32,
    ) -> Result<Vec<crate::domain::ports::DirectoryGroup>, AuthProviderError> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(Vec::new());
        }
        let filter = format!("(&{GROUP_CLASSES}(cn=*{}*))", escape_ldap_filter(text));
        let mut ldap = self.connect().await?;
        let searched = ldap
            .with_timeout(self.operation_timeout)
            .with_search_options(SearchOptions::new().sizelimit(limit))
            .search(&self.config.base_dn, Scope::Subtree, &filter, vec!["cn"])
            .await;
        self.unbind(&mut ldap).await;
        let ldap3::SearchResult(entries, result) = searched.map_err(|e| {
            self.map_operation_error(e, |e| {
                AuthProviderError::Internal(format!("LDAP group search: {}", e))
            })
        })?;
        if result.rc != 0 && result.rc != SIZE_LIMIT_EXCEEDED {
            return Err(AuthProviderError::Internal(format!(
                "LDAP group search failed: {}",
                result
            )));
        }
        Ok(entries
            .into_iter()
            .map(SearchEntry::construct)
            .map(|entry| {
                let name = entry
                    .attrs
                    .get("cn")
                    .and_then(|values| values.first())
                    .cloned()
                    .unwrap_or_else(|| entry.dn.clone());
                crate::domain::ports::DirectoryGroup { dn: entry.dn, name }
            })
            .take(usize::try_from(limit).unwrap_or(0))
            .collect())
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

    fn addr() -> SocketAddr {
        "10.1.2.3:636".parse().expect("literal address")
    }

    #[test]
    fn connect_errors_are_explained_in_plain_words() {
        let cases = [
            (
                io::ErrorKind::ConnectionRefused,
                "nothing listens on 10.1.2.3:636: the host is up but the port is closed; \
                 check the port and that the directory is running",
            ),
            (
                io::ErrorKind::HostUnreachable,
                "this container has no route to 10.1.2.3: \
                 check the network, VPN or Docker network of the pod",
            ),
            (
                io::ErrorKind::NetworkUnreachable,
                "the network of 10.1.2.3 is unreachable from this container: \
                 check its routes and, for an IPv6 address, whether the pod has IPv6 at all",
            ),
            (
                io::ErrorKind::TimedOut,
                "no answer from 10.1.2.3:636 within 5 s: the host is down or a firewall drops \
                 the connection; check host, port and firewall",
            ),
            (
                io::ErrorKind::PermissionDenied,
                "the container may not connect to 10.1.2.3:636: \
                 a policy or firewall on this host refuses it",
            ),
        ];
        for (kind, expected) in cases {
            assert_eq!(
                explain_connect_error(addr(), &io::Error::from(kind)),
                expected,
                "{kind:?}"
            );
        }
    }

    /// The kernel's own numbers are what a real connect hands back; they must
    /// land on the same explanations as the abstract kinds.
    #[test]
    fn os_error_numbers_map_to_their_explanations() {
        let ehostunreach = io::Error::from_raw_os_error(113);
        assert!(
            explain_connect_error(addr(), &ehostunreach).starts_with("this container has no route"),
            "{ehostunreach}"
        );
        let enetunreach = io::Error::from_raw_os_error(101);
        assert!(
            explain_connect_error(addr(), &enetunreach).starts_with("the network of 10.1.2.3"),
            "{enetunreach}"
        );
        let econnrefused = io::Error::from_raw_os_error(111);
        assert!(
            explain_connect_error(addr(), &econnrefused).starts_with("nothing listens on"),
            "{econnrefused}"
        );
    }

    #[test]
    fn an_unknown_connect_error_keeps_the_os_text() {
        let error = io::Error::other("something odd");
        assert_eq!(
            explain_connect_error(addr(), &error),
            "connecting to 10.1.2.3:636 failed: something odd"
        );
    }

    #[tokio::test]
    async fn a_name_that_does_not_resolve_is_unreachable_with_a_dns_hint() {
        let result = connect_tcp("ldap://no-such-host.invalid", "no-such-host.invalid:389").await;
        let Err(AuthProviderError::Unreachable(message)) = result else {
            panic!("expected Unreachable, got {result:?}");
        };
        assert!(
            message.starts_with(
                "Could not connect to LDAP server at ldap://no-such-host.invalid: \
                 the hostname could not be resolved ("
            ),
            "{message}"
        );
        assert!(
            message.ends_with("check the name and the DNS resolvers of this container"),
            "{message}"
        );
    }

    #[tokio::test]
    async fn a_closed_port_is_unreachable_and_names_the_address() {
        let result = connect_tcp("ldap://127.0.0.1:1", "127.0.0.1:1").await;
        let Err(AuthProviderError::Unreachable(message)) = result else {
            panic!("expected Unreachable, got {result:?}");
        };
        assert_eq!(
            message,
            "Could not connect to LDAP server at ldap://127.0.0.1:1: nothing listens on \
             127.0.0.1:1: the host is up but the port is closed; check the port and that the \
             directory is running"
        );
    }

    #[test]
    fn handshake_errors_are_explained_and_keep_the_original_text() {
        let cases = [
            (
                "I/O error: invalid peer certificate: UnknownIssuer",
                "the server certificate is not trusted: its CA is not in the trust store; \
                 mount the CA with EXTRA_CA_CERTS_DIR",
            ),
            (
                "I/O error: invalid peer certificate: NotValidForName",
                "the server certificate is not valid for the hostname in the URL: \
                 use the name the certificate carries, or reissue it for this one",
            ),
            (
                "I/O error: invalid peer certificate: Expired",
                "the server certificate has expired: renew it on the directory",
            ),
            (
                "I/O error: invalid peer certificate: NotValidYet",
                "the server certificate is not valid yet: check the clocks of the directory and \
                 this host",
            ),
            (
                "I/O error: received corrupt message of type InvalidContentType",
                "the server did not answer with TLS: ldaps:// points at a plain LDAP port; \
                 use the TLS port, or ldap:// with StartTLS",
            ),
            (
                "I/O error: received fatal alert: HandshakeFailure",
                "the server refused the TLS handshake: \
                 check which TLS versions and ciphers the directory accepts",
            ),
            (
                "I/O error: Connection reset by peer (os error 104)",
                "the server closed the connection during the handshake: \
                 StartTLS on a TLS port does this; use ldaps:// for that port",
            ),
        ];
        for (raw, explanation) in cases {
            assert_eq!(
                explain_handshake_error(raw),
                format!("{explanation} ({raw})")
            );
        }
    }

    #[test]
    fn an_unknown_handshake_error_is_passed_through() {
        assert_eq!(
            explain_handshake_error("LDAP operation result: rc=52"),
            "LDAP operation result: rc=52"
        );
    }
}
