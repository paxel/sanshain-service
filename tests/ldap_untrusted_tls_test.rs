//! LDAP over TLS, untrusted-CA side: a server certificate signed by a CA the
//! process does not trust must be rejected at the handshake. Its own binary,
//! separate from the trusted case, because the client trust store is a
//! process-wide `OnceLock` that cannot hold two different roots at once.
//!
//! The server cert still carries SAN `127.0.0.1`, so the connection is refused
//! purely because the CA does not chain — not a hostname mismatch. That is what
//! proves the trust store is actually enforced (the "silently trusts nothing /
//! silently trusts anything" failure this guards against).

use sanshain_service::domain::models::LdapConfig;
use sanshain_service::domain::ports::{AuthProvider, AuthProviderError};
use sanshain_service::infrastructure::ldap_provider::LdapAuthProvider;
use sanshain_service::infrastructure::tls;
use std::net::{IpAddr, Ipv4Addr};
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

const LDAP_PORT: u16 = 389;
const LDAPS_PORT: u16 = 636;
const LDAP_ADMIN_PW: &str = "adminpw";

#[cfg(test)]
fn generate_certs() -> (String, String, String) {
    use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, SanType};
    let ca_key = KeyPair::generate().expect("ca key");
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).expect("ca params");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "Untrusted Test CA");
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

#[tokio::test]
async fn ldaps_against_an_untrusted_ca_is_rejected() {
    let (ca_pem, srv_pem, key_pem) = generate_certs();

    // Install the platform-only trust store (the container's CA is NOT added).
    unsafe {
        std::env::remove_var(tls::EXTRA_CA_CERTS_DIR_ENV);
    }
    tls::init_from_env().expect("trust store init");

    let container = start_openldap_tls(&ca_pem, &srv_pem, &key_pem).await;
    let tls_port = container.get_host_port_ipv4(LDAPS_PORT).await.unwrap();

    let provider = LdapAuthProvider::new(LdapConfig {
        server_url: format!("ldaps://127.0.0.1:{tls_port}"),
        bind_dn: "cn=admin,dc=example,dc=org".to_string(),
        bind_password: Some(LDAP_ADMIN_PW.to_string()),
        base_dn: "dc=example,dc=org".to_string(),
        user_filter: "(uid={username})".to_string(),
        group_filter: String::new(),
        admin_group: String::new(),
        use_tls: false,
    });

    // The TLS handshake must fail (CA does not chain) → ConnectionFailed, before
    // any bind or search.
    let result = provider.authenticate("alice", "alicepw").await;
    assert!(
        matches!(result, Err(AuthProviderError::ConnectionFailed(_))),
        "an untrusted server CA must be rejected at the handshake, got: {result:?}"
    );
}
