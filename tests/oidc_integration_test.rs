//! OIDC human login against a real Keycloak (testcontainers).
//!
//! Proves the provider's security-critical paths against a real IdP: discovery
//! and authorization-URL construction, and the full authorization-code exchange
//! with ID-token verification. The auth-code step is driven headlessly — the
//! Keycloak login form is submitted over HTTP and the `code` extracted from the
//! redirect — so no browser is needed.
//!
//! Keycloak serves https with a certificate from a CA generated here, which
//! reaches the process trust store through `EXTRA_CA_CERTS_DIR`. One CA serves
//! every test of this binary because the trust store is a process-wide
//! `OnceLock`.

#![expect(
    unsafe_code,
    reason = "edition 2024: std::env::set_var/remove_var are unsafe"
)]

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, SanType};
use sanshain_service::domain::models::{AppError, OidcConfig};
use sanshain_service::domain::ports::OidcFlow;
use sanshain_service::infrastructure::oidc_provider::OidcProvider;
use sanshain_service::infrastructure::tls;
use serde_json::json;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::OnceLock;
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

const REALM: &str = "sanshain";
const CLIENT_ID: &str = "sanshain-app";
const CLIENT_SECRET: &str = "test-client-secret";
const REDIRECT: &str = "http://127.0.0.1:9/auth/oidc/callback";
const USER: &str = "alice";
const PASS: &str = "alicepw";

/// The test CA (PEM) and Keycloak's certificate and key, signed by it for
/// `localhost` and `127.0.0.1`.
#[cfg(test)]
struct Certs {
    ca: String,
    server: String,
    key: String,
}

/// The CA, generated once and installed in the process trust store before any
/// OIDC call of this binary.
#[cfg(test)]
fn certs() -> &'static Certs {
    static CERTS: OnceLock<Certs> = OnceLock::new();
    CERTS.get_or_init(|| {
        let ca_key = KeyPair::generate().expect("ca key");
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).expect("ca params");
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "Sanshain OIDC Test CA");
        let ca_cert = ca_params.self_signed(&ca_key).expect("self-signed CA");

        let srv_key = KeyPair::generate().expect("server key");
        let mut srv_params =
            CertificateParams::new(vec!["localhost".to_string()]).expect("server params");
        srv_params.subject_alt_names = vec![
            SanType::DnsName("localhost".try_into().expect("dns san")),
            SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        ];
        let srv_cert = srv_params
            .signed_by(&srv_key, &Issuer::from_params(&ca_params, &ca_key))
            .expect("sign server cert");

        let ca_dir = std::env::temp_dir().join("sanshain_oidc_tls_ca");
        let _ = std::fs::remove_dir_all(&ca_dir);
        std::fs::create_dir_all(&ca_dir).expect("ca dir");
        std::fs::write(ca_dir.join("ca.crt"), ca_cert.pem()).expect("write ca");
        unsafe {
            std::env::set_var(tls::EXTRA_CA_CERTS_DIR_ENV, &ca_dir);
        }
        tls::init_from_env().expect("trust store init");

        Certs {
            ca: ca_cert.pem(),
            server: srv_cert.pem(),
            key: srv_key.serialize_pem(),
        }
    })
}

/// A client for the test's own calls to Keycloak (admin API, login form) that
/// trusts the test CA.
#[cfg(test)]
fn test_client(builder: reqwest::ClientBuilder) -> reqwest::Client {
    let ca = reqwest::Certificate::from_pem(certs().ca.as_bytes()).expect("ca pem");
    builder
        .add_root_certificate(ca)
        .build()
        .expect("test http client")
}

#[cfg(test)]
async fn start_keycloak() -> ContainerAsync<GenericImage> {
    let certs = certs();
    let mut last = String::new();
    for attempt in 1..=5 {
        let image = GenericImage::new("quay.io/keycloak/keycloak", "24.0")
            .with_wait_for(WaitFor::message_on_stdout("Listening on:"))
            .with_exposed_port(8080.tcp())
            .with_exposed_port(8443.tcp())
            .with_env_var("KEYCLOAK_ADMIN", "admin")
            .with_env_var("KEYCLOAK_ADMIN_PASSWORD", "admin")
            .with_env_var("KC_HTTPS_CERTIFICATE_FILE", "/opt/keycloak/conf/tls.crt")
            .with_env_var(
                "KC_HTTPS_CERTIFICATE_KEY_FILE",
                "/opt/keycloak/conf/tls.key",
            )
            .with_copy_to(
                "/opt/keycloak/conf/tls.crt",
                certs.server.clone().into_bytes(),
            )
            .with_copy_to("/opt/keycloak/conf/tls.key", certs.key.clone().into_bytes())
            .with_cmd(["start-dev"]);
        match image.start().await {
            Ok(c) => return c,
            Err(e) => {
                last = e.to_string();
                eprintln!("keycloak start attempt {attempt}/5 failed: {last}");
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
        }
    }
    panic!("keycloak failed to start after 5 attempts: {last}");
}

#[cfg(test)]
async fn admin_token(base: &str, http: &reqwest::Client) -> String {
    let resp = http
        .post(format!(
            "{base}/realms/master/protocol/openid-connect/token"
        ))
        .form(&[
            ("client_id", "admin-cli"),
            ("username", "admin"),
            ("password", "admin"),
            ("grant_type", "password"),
        ])
        .send()
        .await
        .expect("admin token request")
        .error_for_status()
        .expect("admin token status");
    let v: serde_json::Value = resp.json().await.expect("admin token json");
    v["access_token"]
        .as_str()
        .expect("access_token")
        .to_string()
}

/// Create the realm, a confidential client with a known secret, and a user with
/// a password.
#[cfg(test)]
async fn setup_realm(base: &str, http: &reqwest::Client) {
    let token = admin_token(base, http).await;
    let bearer = |rb: reqwest::RequestBuilder| rb.bearer_auth(&token);

    bearer(http.post(format!("{base}/admin/realms")))
        .json(&json!({ "realm": REALM, "enabled": true }))
        .send()
        .await
        .expect("create realm")
        .error_for_status()
        .expect("realm status");

    bearer(http.post(format!("{base}/admin/realms/{REALM}/clients")))
        .json(&json!({
            "clientId": CLIENT_ID,
            "secret": CLIENT_SECRET,
            "clientAuthenticatorType": "client-secret",
            "redirectUris": [REDIRECT],
            "standardFlowEnabled": true,
            "publicClient": false,
            "protocol": "openid-connect",
        }))
        .send()
        .await
        .expect("create client")
        .error_for_status()
        .expect("client status");

    bearer(http.post(format!("{base}/admin/realms/{REALM}/users")))
        .json(&json!({
            "username": USER,
            "enabled": true,
            "email": "alice@example.org",
            "emailVerified": true,
            "firstName": "Alice",
            "lastName": "Anderson",
            "credentials": [{ "type": "password", "value": PASS, "temporary": false }],
        }))
        .send()
        .await
        .expect("create user")
        .error_for_status()
        .expect("user status");
}

#[cfg(test)]
fn config(base: &str) -> OidcConfig {
    OidcConfig {
        issuer_url: format!("{base}/realms/{REALM}"),
        client_id: CLIENT_ID.to_string(),
        client_secret: Some(CLIENT_SECRET.to_string()),
        redirect_url: REDIRECT.to_string(),
        scopes: vec!["openid".into(), "email".into(), "profile".into()],
        username_claim: "preferred_username".to_string(),
        groups_claim: "groups".to_string(),
        admin_group: String::new(),
    }
}

#[tokio::test]
async fn oidc_discovery_and_authorize_url_against_keycloak() {
    let container = start_keycloak().await;
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(8443).await.unwrap();
    let base = format!("https://{host}:{port}");
    let http = test_client(reqwest::Client::builder());
    setup_realm(&base, &http).await;

    // Discovery + client build + PKCE authorization URL.
    let authorize = OidcProvider
        .authorize_url(&config(&base))
        .await
        .expect("authorize_url discovers and builds");
    assert!(
        authorize.url.contains("/protocol/openid-connect/auth"),
        "authorize URL targets Keycloak's auth endpoint: {}",
        authorize.url
    );
    assert!(
        authorize.url.contains("code_challenge="),
        "PKCE challenge present"
    );
    assert!(!authorize.state.is_empty() && !authorize.nonce.is_empty());

    // The same realm over plain http: the key set that validates logins must
    // come over https, so discovery refuses it.
    let http_port = container.get_host_port_ipv4(8080).await.unwrap();
    let refused = OidcProvider
        .authorize_url(&config(&format!("http://{host}:{http_port}")))
        .await;
    match refused {
        Err(AppError::Internal(message)) => assert!(
            message.contains("OIDC discovery failed") && message.contains("must use TLS"),
            "{message}"
        ),
        Err(other) => panic!("expected a discovery failure, got {other:?}"),
        Ok(_) => panic!("an http issuer must be refused"),
    }
}

/// Extract the Keycloak login-form POST action from the login page HTML.
#[cfg(test)]
fn extract_login_action(html: &str) -> String {
    for (idx, _) in html.match_indices("action=\"") {
        let start = idx + "action=\"".len();
        let end = html[start..]
            .find('"')
            .map(|e| e + start)
            .expect("action end");
        let action = &html[start..end];
        if action.contains("authenticate") {
            return action.replace("&amp;", "&");
        }
    }
    panic!("no login form action in Keycloak page");
}

/// Drive the Keycloak login form headlessly and return the authorization `code`
/// from the redirect. Uses a cookie-storing, non-redirect-following client so
/// the session cookie carries GET→POST and the final 302 to the callback is
/// captured rather than chased.
#[cfg(test)]
async fn obtain_code(authorize_url: &str) -> String {
    let http = test_client(
        reqwest::Client::builder()
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::none()),
    );
    let page = http
        .get(authorize_url)
        .send()
        .await
        .expect("login page")
        .text()
        .await
        .expect("login page body");
    let action = extract_login_action(&page);
    let resp = http
        .post(&action)
        .form(&[("username", USER), ("password", PASS), ("credentialId", "")])
        .send()
        .await
        .expect("login submit");
    let location = resp
        .headers()
        .get("location")
        .unwrap_or_else(|| panic!("login did not redirect (status {})", resp.status()))
        .to_str()
        .expect("location str");
    // location = <redirect>?...&code=...&...
    let query = location.split('?').nth(1).expect("redirect has a query");
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix("code="))
        .expect("code in redirect")
        .to_string()
}

#[tokio::test]
async fn oidc_full_code_flow_verifies_the_id_token() {
    let container = start_keycloak().await;
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(8443).await.unwrap();
    let base = format!("https://{host}:{port}");
    let http = test_client(reqwest::Client::builder());
    setup_realm(&base, &http).await;

    let cfg = config(&base);
    let authorize = OidcProvider
        .authorize_url(&cfg)
        .await
        .expect("authorize_url");

    // Headless login → authorization code.
    let code = obtain_code(&authorize.url).await;

    // Exchange + verify the ID token (JWKS signature, nonce, issuer, audience,
    // expiry) → resolved identity.
    let claims = OidcProvider
        .exchange_and_verify(&cfg, code, authorize.pkce_verifier, authorize.nonce)
        .await
        .expect("code exchange + ID token verification");
    assert_eq!(claims.username, USER);
    assert_eq!(claims.email.as_deref(), Some("alice@example.org"));
}
