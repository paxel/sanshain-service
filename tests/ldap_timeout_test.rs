//! The LDAP operation timeout, against a socket that behaves like a broken
//! directory — no container.
//!
//! The connect timeout (covered in `ldap_integration_test.rs` against a closed
//! port) only bounds the TCP connect. A scheme and port mismatch (`ldaps://`
//! on 389, StartTLS on 636) or a firewall that accepts and then stalls gets
//! past that: the TCP connect succeeds and nothing ever answers. These tests
//! stand up a local listener that accepts and never writes, and one that
//! answers the service-account bind and then stalls on the search, and check
//! that the provider gives up within the operation timeout with a message
//! that names the URL and points at the port and scheme.

use sanshain_service::domain::models::LdapConfig;
use sanshain_service::domain::ports::{AuthProvider, AuthProviderError};
use sanshain_service::infrastructure::ldap_provider::LdapAuthProvider;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Short enough that a test finishes well under a second, long enough that
/// the TCP connect and the first write are comfortably done before it fires.
const SHORT_TIMEOUT: Duration = Duration::from_millis(200);

/// Generous upper bound on how long a timed-out call may take overall.
const MUST_RETURN_WITHIN: Duration = Duration::from_secs(2);

/// Accepts every connection and never reads or writes on any of them.
#[cfg(test)]
async fn listener_that_never_answers() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let mut held = Vec::new();
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                return;
            };
            // Keep the socket open so the client sees a live connection, not a
            // reset.
            held.push(socket);
        }
    });
    addr.to_string()
}

/// Accepts one connection, never writes, and resolves the returned receiver
/// once the client has closed that connection (read returns end of stream).
#[cfg(test)]
async fn listener_that_reports_the_close() -> (String, tokio::sync::oneshot::Receiver<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut buf = [0u8; 512];
        while matches!(socket.read(&mut buf).await, Ok(n) if n > 0) {}
        let _ = closed_tx.send(());
    });
    (addr.to_string(), closed_rx)
}

/// Accepts one connection, answers the first message (the service-account
/// bind) with a success BindResponse carrying the request's message id, and
/// then goes quiet for good.
#[cfg(test)]
async fn listener_that_answers_bind_then_stalls() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut buf = [0u8; 512];
        let Ok(n) = socket.read(&mut buf).await else {
            return;
        };
        // LDAPMessage ::= SEQUENCE { messageID INTEGER, protocolOp ... }.
        // For a short bind request that is `30 <len> 02 01 <id> 60 ...`.
        assert!(n >= 5 && buf[0] == 0x30 && buf[2] == 0x02 && buf[3] == 0x01);
        let id = buf[4];
        // BindResponse: resultCode success, empty matchedDN and diagnostic.
        let response = [
            0x30, 0x0c, 0x02, 0x01, id, 0x61, 0x07, 0x0a, 0x01, 0x00, 0x04, 0x00, 0x04, 0x00,
        ];
        let _ = socket.write_all(&response).await;
        // Hold the socket open and never answer anything else.
        let mut sink = [0u8; 512];
        while let Ok(n) = socket.read(&mut sink).await {
            if n == 0 {
                return;
            }
        }
    });
    addr.to_string()
}

fn config(url: &str, use_tls: bool) -> LdapConfig {
    LdapConfig {
        server_url: url.to_string(),
        bind_dn: "cn=admin,dc=example,dc=org".to_string(),
        bind_password: Some("adminpw".to_string()),
        base_dn: "dc=example,dc=org".to_string(),
        user_filter: "(uid={username})".to_string(),
        group_filter: String::new(),
        admin_group: String::new(),
        use_tls,
        username_attribute: String::new(),
    }
}

fn provider(url: &str, use_tls: bool) -> LdapAuthProvider {
    LdapAuthProvider::new(config(url, use_tls)).with_operation_timeout(SHORT_TIMEOUT)
}

/// The error a stalled directory must produce: the provider's connection
/// error, naming the URL, the timeout, and the port/scheme hint.
#[cfg(test)]
fn assert_stalled(result: Result<(), AuthProviderError>, url: &str) {
    let Err(AuthProviderError::ConnectionFailed(message)) = result else {
        panic!("a stalled directory is a connection failure, got: {result:?}");
    };
    let expected = format!(
        "LDAP server at {url} accepted the connection but did not respond within 0.2 s. \
         Check that ldaps:// uses the TLS port and StartTLS the plain one."
    );
    assert_eq!(message, expected);
}

#[tokio::test]
async fn test_connection_over_plain_ldap_gives_up_when_the_bind_is_never_answered() {
    let url = format!("ldap://{}", listener_that_never_answers().await);
    let started = Instant::now();

    let result = provider(&url, false).test_connection().await;

    assert!(
        started.elapsed() < MUST_RETURN_WITHIN,
        "must fail within the operation timeout, took {:?}",
        started.elapsed()
    );
    assert_stalled(result, &url);
}

#[tokio::test]
async fn test_connection_with_starttls_gives_up_when_the_handshake_is_never_answered() {
    let url = format!("ldap://{}", listener_that_never_answers().await);
    let started = Instant::now();

    let result = provider(&url, true).test_connection().await;

    assert!(
        started.elapsed() < MUST_RETURN_WITHIN,
        "must fail within the operation timeout, took {:?}",
        started.elapsed()
    );
    assert_stalled(result, &url);
}

/// ldap3 drives the StartTLS exchange from a task it spawns, which the
/// timeout cannot cancel. The task must still end — and close the socket —
/// once the provider has given up, or every stalled attempt would pin a
/// connection until the directory closes it.
#[tokio::test]
async fn a_timed_out_starttls_handshake_releases_the_socket() {
    let (addr, closed) = listener_that_reports_the_close().await;
    let url = format!("ldap://{addr}");

    let result = provider(&url, true).test_connection().await;
    assert_stalled(result, &url);

    tokio::time::timeout(MUST_RETURN_WITHIN, closed)
        .await
        .expect("the server must see the connection closed once the provider gave up")
        .expect("the listener task reports the close");
}

#[tokio::test]
async fn test_connection_over_ldaps_gives_up_when_the_handshake_is_never_answered() {
    let url = format!("ldaps://{}", listener_that_never_answers().await);
    let started = Instant::now();

    let result = provider(&url, true).test_connection().await;

    assert!(
        started.elapsed() < MUST_RETURN_WITHIN,
        "must fail within the operation timeout, took {:?}",
        started.elapsed()
    );
    assert_stalled(result, &url);
}

#[tokio::test]
async fn authenticate_gives_up_when_the_search_is_never_answered() {
    let url = format!("ldap://{}", listener_that_answers_bind_then_stalls().await);
    let started = Instant::now();

    let result = provider(&url, false).authenticate("alice", "alicepw").await;

    assert!(
        started.elapsed() < MUST_RETURN_WITHIN,
        "must fail within the operation timeout, took {:?}",
        started.elapsed()
    );
    assert_stalled(result.map(|_| ()), &url);
}

/// A directory that is simply not listening is unreachable, not stalled:
/// the TCP connect never succeeded.
#[tokio::test]
async fn a_refused_connection_is_reported_as_unreachable() {
    let url = "ldap://127.0.0.1:1";

    let result = provider(url, false).test_connection().await;

    let Err(AuthProviderError::Unreachable(message)) = result else {
        panic!("a refused connection means the directory was never reached, got: {result:?}");
    };
    assert!(
        message.starts_with("Could not connect to LDAP server at ldap://127.0.0.1:1"),
        "unexpected message: {message}"
    );
    assert!(
        !message.contains("accepted the connection"),
        "a refused connection was never accepted: {message}"
    );
}
