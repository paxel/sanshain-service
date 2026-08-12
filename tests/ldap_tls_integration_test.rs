//! LDAP over TLS against a real OpenLDAP (testcontainers), trusted-CA side.
//!
//! A CA + server certificate (SAN `127.0.0.1`) is generated with rcgen, mounted
//! into osixia, and the CA is installed into the process trust store via
//! `EXTRA_CA_CERTS_DIR` before the first connection. Both `ldaps://` (implicit
//! TLS) and StartTLS (`ldap://` upgraded via `use_tls`) must then authenticate.
//! The *untrusted*-CA rejection lives in a separate binary
//! (`ldap_untrusted_tls_test.rs`) because the trust store is a process-wide
//! `OnceLock` that cannot hold two different roots in one process.

use ldap3::{Ldap, LdapConnAsync, Mod};
use sanshain_service::domain::models::LdapConfig;
use sanshain_service::domain::ports::AuthProvider;
use sanshain_service::infrastructure::{ldap_provider::LdapAuthProvider, tls};
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr};
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

const LDAP_PORT: u16 = 389;
const LDAPS_PORT: u16 = 636;
const LDAP_BASE_DN: &str = "dc=example,dc=org";
const LDAP_ADMIN_DN: &str = "cn=admin,dc=example,dc=org";
const LDAP_ADMIN_PW: &str = "adminpw";
const ADMINS_DN: &str = "cn=admins,ou=groups,dc=example,dc=org";
const ALICE_DN: &str = "uid=alice,ou=people,dc=example,dc=org";

fn attr<'a>(values: &[&'a str]) -> HashSet<&'a str> {
    values.iter().copied().collect()
}

/// Generate a CA and a `127.0.0.1`/`localhost` server certificate signed by it.
/// Returns (ca_pem, server_cert_pem, server_key_pem).
#[cfg(test)]
fn generate_certs() -> (String, String, String) {
    use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, SanType};
    let ca_key = KeyPair::generate().expect("ca key");
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).expect("ca params");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "Sanshain LDAP Test CA");
    let ca_cert = ca_params.self_signed(&ca_key).expect("self-signed CA");

    let srv_key = KeyPair::generate().expect("server key");
    let mut srv_params = CertificateParams::new(vec!["localhost".to_string()]).expect("srv params");
    srv_params.subject_alt_names = vec![
        SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        SanType::DnsName("localhost".try_into().expect("dns san")),
    ];
    let srv_cert = srv_params
        .signed_by(&srv_key, &ca_cert, &ca_key)
        .expect("sign server cert");

    (ca_cert.pem(), srv_cert.pem(), srv_key.serialize_pem())
}

#[cfg(test)]
async fn start_openldap_tls(
    ca_pem: &str,
    srv_pem: &str,
    key_pem: &str,
) -> ContainerAsync<GenericImage> {
    // Copy the certs *into* the image (not a bind-mount): osixia's ssl-tools
    // chowns them to `openldap`, which fails on a root-owned host bind-mount
    // (the status-80 startup death).
    const CERTS: &str = "/container/service/slapd/assets/certs";
    let mut last_err = String::new();
    for attempt in 1..=5 {
        let image = GenericImage::new("osixia/openldap", "1.5.0")
            .with_wait_for(WaitFor::message_on_stderr("slapd starting"))
            .with_exposed_port(LDAP_PORT.tcp())
            .with_exposed_port(LDAPS_PORT.tcp())
            .with_env_var("LDAP_ORGANISATION", "Example Inc.")
            .with_env_var("LDAP_DOMAIN", "example.org")
            .with_env_var("LDAP_ADMIN_PASSWORD", LDAP_ADMIN_PW)
            .with_env_var("LDAP_CONFIG_PASSWORD", "configpw")
            .with_env_var("LDAP_TLS", "true")
            .with_env_var("LDAP_TLS_CRT_FILENAME", "server.crt")
            .with_env_var("LDAP_TLS_KEY_FILENAME", "server.key")
            .with_env_var("LDAP_TLS_CA_CRT_FILENAME", "ca.crt")
            .with_env_var("LDAP_TLS_VERIFY_CLIENT", "try")
            .with_copy_to(format!("{CERTS}/server.crt"), srv_pem.as_bytes().to_vec())
            .with_copy_to(format!("{CERTS}/server.key"), key_pem.as_bytes().to_vec())
            .with_copy_to(format!("{CERTS}/ca.crt"), ca_pem.as_bytes().to_vec());
        match image.start().await {
            Ok(container) => return container,
            Err(err) => {
                last_err = err.to_string();
                eprintln!("openldap-tls start attempt {attempt}/5 failed: {last_err}");
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
        }
    }
    panic!("openldap-tls container failed to start after 5 attempts: {last_err}");
}

#[cfg(test)]
async fn admin_bind(url: &str) -> Ldap {
    let mut last = String::new();
    for _ in 0..20 {
        if let Ok((conn, mut ldap)) = LdapConnAsync::new(url).await {
            ldap3::drive!(conn);
            match ldap.simple_bind(LDAP_ADMIN_DN, LDAP_ADMIN_PW).await {
                Ok(r) => match r.success() {
                    Ok(_) => return ldap,
                    Err(e) => last = e.to_string(),
                },
                Err(e) => last = e.to_string(),
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    panic!("admin bind never succeeded: {last}");
}

#[cfg(test)]
async fn enable_memberof(url: &str) {
    let (conn, mut ldap) = LdapConnAsync::new(url).await.expect("config connect");
    ldap3::drive!(conn);
    ldap.simple_bind("cn=admin,cn=config", "configpw")
        .await
        .expect("config bind req")
        .success()
        .expect("config bind");
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
    .unwrap_or_else(|e| panic!("memberof overlay: {e}"))
    .success()
    .unwrap_or_else(|e| panic!("memberof overlay rejected: {e}"));
    let _ = ldap.unbind().await;
}

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

fn config(server_url: &str, use_tls: bool) -> LdapConfig {
    LdapConfig {
        server_url: server_url.to_string(),
        bind_dn: LDAP_ADMIN_DN.to_string(),
        bind_password: Some(LDAP_ADMIN_PW.to_string()),
        base_dn: LDAP_BASE_DN.to_string(),
        user_filter: "(uid={username})".to_string(),
        group_filter: String::new(),
        admin_group: ADMINS_DN.to_string(),
        use_tls,
    }
}

#[tokio::test]
async fn ldaps_and_starttls_authenticate_against_a_trusted_ca() {
    let (ca_pem, srv_pem, key_pem) = generate_certs();

    // The CA on disk for the process trust store (installed BEFORE any
    // connection so this init wins the OnceLock — this binary connects nowhere
    // else first).
    let ca_dir = std::env::temp_dir().join("sanshain_ldap_tls_ca");
    let _ = std::fs::remove_dir_all(&ca_dir);
    std::fs::create_dir_all(&ca_dir).expect("ca dir");
    std::fs::write(ca_dir.join("ca.crt"), &ca_pem).unwrap();
    unsafe {
        std::env::set_var(tls::EXTRA_CA_CERTS_DIR_ENV, &ca_dir);
    }
    tls::init_from_env().expect("trust store init");

    let container = start_openldap_tls(&ca_pem, &srv_pem, &key_pem).await;
    let host = container.get_host().await.unwrap();
    let plain_port = container.get_host_port_ipv4(LDAP_PORT).await.unwrap();
    let tls_port = container.get_host_port_ipv4(LDAPS_PORT).await.unwrap();
    let plain_url = format!("ldap://{host}:{plain_port}");

    let mut ready = admin_bind(&plain_url).await;
    let _ = ready.unbind().await;
    enable_memberof(&plain_url).await;
    seed(&plain_url).await;

    // ldaps:// — implicit TLS, cert SAN 127.0.0.1 matches, CA trusted.
    let ldaps = LdapAuthProvider::new(config(&format!("ldaps://127.0.0.1:{tls_port}"), false));
    let u = ldaps
        .authenticate("alice", "alicepw")
        .await
        .expect("ldaps authenticates against the trusted CA");
    assert_eq!(u.username, "alice");

    // StartTLS — plain ldap:// upgraded in-band via use_tls.
    let starttls = LdapAuthProvider::new(config(&format!("ldap://127.0.0.1:{plain_port}"), true));
    let u = starttls
        .authenticate("alice", "alicepw")
        .await
        .expect("StartTLS authenticates against the trusted CA");
    assert_eq!(u.username, "alice");
}
