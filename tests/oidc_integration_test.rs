//! OIDC human login against a real Keycloak (testcontainers).
//!
//! Proves the provider's security-critical paths against a real IdP: discovery
//! and authorization-URL construction, and the full authorization-code exchange
//! with ID-token verification. The auth-code step is driven headlessly — the
//! Keycloak login form is submitted over HTTP and the `code` extracted from the
//! redirect — so no browser is needed.

use sanshain_service::domain::models::OidcConfig;
use sanshain_service::infrastructure::oidc_provider;
use serde_json::json;
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

const REALM: &str = "sanshain";
const CLIENT_ID: &str = "sanshain-app";
const CLIENT_SECRET: &str = "test-client-secret";
const REDIRECT: &str = "http://127.0.0.1:9/auth/oidc/callback";
const USER: &str = "alice";
const PASS: &str = "alicepw";

#[cfg(test)]
async fn start_keycloak() -> ContainerAsync<GenericImage> {
    let mut last = String::new();
    for attempt in 1..=5 {
        let image = GenericImage::new("quay.io/keycloak/keycloak", "24.0")
            .with_wait_for(WaitFor::message_on_stdout(
                "Listening on: http://0.0.0.0:8080",
            ))
            .with_exposed_port(8080.tcp())
            .with_env_var("KEYCLOAK_ADMIN", "admin")
            .with_env_var("KEYCLOAK_ADMIN_PASSWORD", "admin")
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
    let port = container.get_host_port_ipv4(8080).await.unwrap();
    let base = format!("http://{host}:{port}");
    let http = reqwest::Client::new();
    setup_realm(&base, &http).await;

    // Discovery + client build + PKCE authorization URL.
    let authorize = oidc_provider::authorize_url(&config(&base))
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
    let http = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("login http client");
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
    let port = container.get_host_port_ipv4(8080).await.unwrap();
    let base = format!("http://{host}:{port}");
    let http = reqwest::Client::new();
    setup_realm(&base, &http).await;

    let cfg = config(&base);
    let authorize = oidc_provider::authorize_url(&cfg)
        .await
        .expect("authorize_url");

    // Headless login → authorization code.
    let code = obtain_code(&authorize.url).await;

    // Exchange + verify the ID token (JWKS signature, nonce, issuer, audience,
    // expiry) → resolved identity.
    let claims =
        oidc_provider::exchange_and_verify(&cfg, code, authorize.pkce_verifier, authorize.nonce)
            .await
            .expect("code exchange + ID token verification");
    assert_eq!(claims.username, USER);
    assert_eq!(claims.email.as_deref(), Some("alice@example.org"));
}
