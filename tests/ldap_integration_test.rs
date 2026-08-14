//! LDAP authentication against a real OpenLDAP server (testcontainers).
//!
//! The provider was previously covered only by config-validation unit tests —
//! the actual bind/search/group flow never ran. This exercises it end to end
//! over plain `ldap://`: the happy path with group resolution, the credential
//! and lookup failure modes, LDAP filter-injection escaping, a bad
//! service-account bind, and the connect timeout. The `ldaps://`/StartTLS
//! matrix lives in `ldap_tls_integration_test.rs` (a separate binary, because
//! the client trust store is a process-wide `OnceLock`).

use ldap3::{Ldap, LdapConnAsync, Mod};
use sanshain_service::domain::models::LdapConfig;
use sanshain_service::domain::ports::{AuthProvider, AuthProviderError, DirectoryGroups};
use sanshain_service::infrastructure::ldap_provider::LdapAuthProvider;
use std::collections::HashSet;
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

const LDAP_PORT: u16 = 389;
const LDAP_BASE_DN: &str = "dc=example,dc=org";
const LDAP_ADMIN_DN: &str = "cn=admin,dc=example,dc=org";
const LDAP_ADMIN_PW: &str = "adminpw";
const ADMINS_DN: &str = "cn=admins,ou=groups,dc=example,dc=org";
const ALICE_DN: &str = "uid=alice,ou=people,dc=example,dc=org";

/// Start a throwaway OpenLDAP container (osixia/openldap), retrying transient
/// pull/start failures (same bollard concurrent-pull flake the Postgres helper
/// guards against). osixia enables the `memberof` overlay by default, so a
/// user's `memberOf` is populated from the `groupOfNames` it belongs to.
#[cfg(test)]
async fn start_openldap() -> ContainerAsync<GenericImage> {
    let mut last_err = String::new();
    for attempt in 1..=5 {
        let image = GenericImage::new("osixia/openldap", "1.5.0")
            .with_wait_for(WaitFor::message_on_stderr("slapd starting"))
            .with_exposed_port(LDAP_PORT.tcp())
            .with_env_var("LDAP_ORGANISATION", "Example Inc.")
            .with_env_var("LDAP_DOMAIN", "example.org")
            .with_env_var("LDAP_ADMIN_PASSWORD", LDAP_ADMIN_PW)
            .with_env_var("LDAP_CONFIG_PASSWORD", "configpw");
        match image.start().await {
            Ok(container) => return container,
            Err(err) => {
                last_err = err.to_string();
                eprintln!("openldap container start attempt {attempt}/5 failed: {last_err}");
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
        }
    }
    panic!("openldap container failed to start after 5 attempts: {last_err}");
}

fn attr<'a>(values: &[&'a str]) -> HashSet<&'a str> {
    values.iter().copied().collect()
}

/// Bind as the directory admin, retrying while osixia finishes its first-boot
/// bootstrap (the "slapd starting" log precedes readiness for writes).
#[cfg(test)]
async fn admin_bind(url: &str) -> Ldap {
    let mut last = String::new();
    for _ in 0..20 {
        match LdapConnAsync::new(url).await {
            Ok((conn, mut ldap)) => {
                ldap3::drive!(conn);
                match ldap.simple_bind(LDAP_ADMIN_DN, LDAP_ADMIN_PW).await {
                    Ok(r) => match r.success() {
                        Ok(_) => return ldap,
                        Err(e) => last = e.to_string(),
                    },
                    Err(e) => last = e.to_string(),
                }
            }
            Err(e) => last = e.to_string(),
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    panic!("admin bind never succeeded: {last}");
}

/// Enable the memberof overlay via `cn=config` so a `groupOfNames` membership
/// populates the member's `memberOf` (which is what the provider reads). osixia
/// does not enable it by default. Must run *before* the groups are seeded — the
/// overlay populates `memberOf` on member additions made while it is active.
#[cfg(test)]
async fn enable_memberof(url: &str) {
    let (conn, mut ldap) = LdapConnAsync::new(url).await.expect("connect for config");
    ldap3::drive!(conn);
    ldap.simple_bind("cn=admin,cn=config", "configpw")
        .await
        .expect("config bind request")
        .success()
        .expect("config bind");
    // Ensure the memberof module is loaded (no-op/soft-fail if already present).
    let _ = ldap
        .modify(
            "cn=module{0},cn=config",
            vec![Mod::Add("olcModuleLoad", attr(&["memberof"]))],
        )
        .await;
    ldap.add(
        "olcOverlay=memberof,olcDatabase={1}mdb,cn=config",
        vec![
            ("objectClass", attr(&["olcOverlayConfig", "olcMemberOf"])),
            ("olcOverlay", attr(&["memberof"])),
        ],
    )
    .await
    .unwrap_or_else(|e| panic!("add memberof overlay: {e}"))
    .success()
    .unwrap_or_else(|e| panic!("memberof overlay rejected: {e}"));
    let _ = ldap.unbind().await;
}

/// Seed people/groups: user `alice` and group `admins` with `alice` as a
/// member (the memberof overlay then populates `alice`'s `memberOf`).
#[cfg(test)]
async fn seed(url: &str) {
    let mut ldap = admin_bind(url).await;
    for (dn, attrs) in [
        (
            "ou=people,dc=example,dc=org",
            vec![
                ("objectClass", attr(&["organizationalUnit"])),
                ("ou", attr(&["people"])),
            ],
        ),
        (
            "ou=groups,dc=example,dc=org",
            vec![
                ("objectClass", attr(&["organizationalUnit"])),
                ("ou", attr(&["groups"])),
            ],
        ),
        (
            ALICE_DN,
            vec![
                ("objectClass", attr(&["inetOrgPerson"])),
                ("cn", attr(&["Alice"])),
                ("sn", attr(&["Anderson"])),
                ("uid", attr(&["alice"])),
                ("userPassword", attr(&["alicepw"])),
            ],
        ),
        (
            ADMINS_DN,
            vec![
                ("objectClass", attr(&["groupOfNames"])),
                ("cn", attr(&["admins"])),
                ("member", attr(&[ALICE_DN])),
            ],
        ),
    ] {
        ldap.add(dn, attrs)
            .await
            .unwrap_or_else(|e| panic!("add {dn}: {e}"))
            .success()
            .unwrap_or_else(|e| panic!("add {dn} rejected: {e}"));
    }
    let _ = ldap.unbind().await;
}

fn config(url: &str) -> LdapConfig {
    LdapConfig {
        server_url: url.to_string(),
        bind_dn: LDAP_ADMIN_DN.to_string(),
        bind_password: Some(LDAP_ADMIN_PW.to_string()),
        base_dn: LDAP_BASE_DN.to_string(),
        user_filter: "(uid={username})".to_string(),
        group_filter: String::new(),
        admin_group: ADMINS_DN.to_string(),
        use_tls: false,
    }
}

#[tokio::test]
async fn ldap_plain_bind_search_groups_and_failure_modes() {
    let container = start_openldap().await;
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(LDAP_PORT).await.unwrap();
    let url = format!("ldap://{host}:{port}");
    // Gate on slapd readiness, enable the memberof overlay, then seed so the
    // group membership populates alice's memberOf.
    let mut ready = admin_bind(&url).await;
    let _ = ready.unbind().await;
    enable_memberof(&url).await;
    seed(&url).await;

    let provider = LdapAuthProvider::new(config(&url));

    // Happy path: valid credentials authenticate.
    let user = provider
        .authenticate("alice", "alicepw")
        .await
        .expect("valid credentials authenticate");
    assert_eq!(user.username, "alice");

    // Group resolution via memberOf → the admins group.
    let groups = provider.groups_for("alice").await.expect("groups resolve");
    assert!(
        groups.iter().any(|g| g.eq_ignore_ascii_case(ADMINS_DN)),
        "memberOf must include the admins group, got: {groups:?}"
    );

    // Wrong password: user found, user-bind fails → InvalidCredentials.
    assert!(matches!(
        provider.authenticate("alice", "wrong").await,
        Err(AuthProviderError::InvalidCredentials)
    ));

    // Unknown user: search empty → InvalidCredentials.
    assert!(matches!(
        provider.authenticate("nobody", "x").await,
        Err(AuthProviderError::InvalidCredentials)
    ));

    // Filter injection: a crafted username must be escaped, not bypass auth.
    assert!(matches!(
        provider.authenticate("*)(uid=*", "x").await,
        Err(AuthProviderError::InvalidCredentials)
    ));

    // Bad service-account bind → ConnectionFailed (before any user lookup).
    let mut bad = config(&url);
    bad.bind_password = Some("not-the-admin-password".to_string());
    assert!(matches!(
        LdapAuthProvider::new(bad)
            .authenticate("alice", "alicepw")
            .await,
        Err(AuthProviderError::ConnectionFailed(_))
    ));
}

/// An unreachable directory fails fast (bounded by CONNECT_TIMEOUT), not after
/// the OS TCP timeout. No container needed.
#[tokio::test]
async fn ldap_unreachable_server_fails_fast() {
    // 127.0.0.1:1 refuses immediately; the point is the error class, and that
    // the call returns rather than hanging.
    let provider = LdapAuthProvider::new(config("ldap://127.0.0.1:1"));
    let started = std::time::Instant::now();
    let result = provider.authenticate("alice", "alicepw").await;
    assert!(
        matches!(result, Err(AuthProviderError::ConnectionFailed(_))),
        "unreachable server is a connection failure, got: {result:?}"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "must fail fast, took {:?}",
        started.elapsed()
    );
}
