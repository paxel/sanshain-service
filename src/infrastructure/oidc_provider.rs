//! OpenID Connect provider for human browser login (ai/improvements grill batch
//! 2). The `openid` crate drives the security-critical parts — discovery, the
//! PKCE authorization-code flow, and ID-token verification (JWKS signature,
//! nonce, issuer, audience, expiry). The username/groups claims are
//! configurable by name, so they are read from the *already-verified* token
//! payload as JSON.
//!
//! Discovery and the key set go through the process trust store (platform CAs
//! plus `EXTRA_CA_CERTS_DIR`), and the key set must be served over https.
//!
//! OIDC is human-only: Consumer/machine clients keep API tokens.

use crate::domain::models::{AppError, OidcConfig};
use crate::domain::ports::{OidcAuthorizeUrl, OidcClaims, OidcFlow};
use base64::Engine;
use openid::{DiscoveredClient, Options, StandardClaims, Token};
use rand::distr::{Alphanumeric, SampleString};

/// The `openid`-backed implementation of the [`OidcFlow`] port.
pub struct OidcProvider;

impl OidcFlow for OidcProvider {
    async fn authorize_url(&self, config: &OidcConfig) -> Result<OidcAuthorizeUrl, AppError> {
        authorize_url(config).await
    }

    async fn exchange_and_verify(
        &self,
        config: &OidcConfig,
        code: String,
        pkce_verifier: String,
        expected_nonce: String,
    ) -> Result<OidcClaims, AppError> {
        exchange_and_verify(config, code, pkce_verifier, expected_nonce).await
    }
}

/// An HTTP client that does not follow redirects — required by the OIDC spec's
/// SSRF guidance for token/discovery requests — and trusts the process trust
/// store.
fn http_client() -> Result<reqwest::Client, AppError> {
    let tls = crate::infrastructure::tls::client_config()
        .ok_or_else(|| AppError::Internal("OIDC HTTP client: no TLS configuration".to_string()))?;
    reqwest::ClientBuilder::new()
        .tls_backend_preconfigured((*tls).clone())
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AppError::Internal(format!("OIDC HTTP client: {e}")))
}

/// Discover the provider and fetch its key set, which must be served over
/// https.
async fn discover(config: &OidcConfig) -> Result<DiscoveredClient, AppError> {
    let issuer = reqwest::Url::parse(&config.issuer_url)
        .map_err(|e| AppError::BadRequest(format!("OIDC issuer URL: {e}")))?;
    reqwest::Url::parse(&config.redirect_url)
        .map_err(|e| AppError::BadRequest(format!("OIDC redirect URL: {e}")))?;
    DiscoveredClient::discover_with_client(
        http_client()?,
        config.client_id.clone(),
        config.client_secret.clone(),
        config.redirect_url.clone(),
        issuer,
    )
    .await
    .map_err(|e| AppError::Internal(format!("OIDC discovery failed: {e}")))
}

/// 32 alphanumeric characters (about 190 bits) for `state` and `nonce`.
fn random_token() -> String {
    Alphanumeric.sample_string(&mut rand::rng(), 32)
}

/// Build the provider authorization URL plus the state/nonce/PKCE the callback
/// verifies against.
async fn authorize_url(config: &OidcConfig) -> Result<OidcAuthorizeUrl, AppError> {
    let client = discover(config).await?;
    let state = random_token();
    let nonce = random_token();
    let scopes: Vec<&str> = config
        .scopes
        .iter()
        .map(String::as_str)
        .filter(|scope| *scope != "openid")
        .collect();
    let options = Options {
        // `openid` prefixes the `openid` scope itself.
        scope: (!scopes.is_empty()).then(|| scopes.join(" ")),
        state: Some(state.clone()),
        nonce: Some(nonce.clone()),
        ..Default::default()
    };
    let (url, pkce) = client.auth_url_with_new_pkce(&options);
    Ok(OidcAuthorizeUrl {
        url: url.to_string(),
        state,
        nonce,
        pkce_verifier: pkce.code_verifier().to_string(),
    })
}

/// Exchange the authorization code for tokens, verify the ID token, and resolve
/// the identity. Verification (signature via JWKS, nonce, issuer, audience,
/// expiry) is done by `openid`; a failure is an authentication failure.
async fn exchange_and_verify(
    config: &OidcConfig,
    code: String,
    pkce_verifier: String,
    expected_nonce: String,
) -> Result<OidcClaims, AppError> {
    let client = discover(config).await?;
    let bearer = client
        .request_token_pkce(&code, Some(&pkce_verifier))
        .await
        .map_err(|e| {
            tracing::warn!("OIDC code exchange failed: {e}");
            AppError::Unauthorized
        })?;
    // `openid` verifies an ID token only when one came back, so its absence
    // is refused here.
    let raw_id_token = bearer.id_token.clone().ok_or(AppError::Unauthorized)?;
    let token: Token<StandardClaims> = bearer.into();
    let mut id_token = token.id_token.ok_or(AppError::Unauthorized)?;
    // This is the verification: signature (against the discovered JWKS), then
    // nonce, issuer, audience and expiry. An error here is a rejected login.
    client
        .decode_token(&mut id_token)
        .and_then(|()| client.validate_token(&id_token, expected_nonce.as_str(), None))
        .map_err(|e| {
            tracing::warn!("OIDC ID token rejected: {e}");
            AppError::Unauthorized
        })?;
    let claims = id_token.payload().map_err(|e| {
        tracing::warn!("OIDC ID token payload: {e}");
        AppError::Unauthorized
    })?;
    let sub = openid::Claims::sub(claims).to_string();

    // Flexible claims from the (now-verified) payload. Reading the payload JSON
    // is safe here precisely because the token was just verified above.
    let payload = decode_payload(&raw_id_token)?;
    let username = payload
        .get(&config.username_claim)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or(sub);
    let name = payload
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let email = payload
        .get("email")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let groups = payload
        .get(&config.groups_claim)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    Ok(OidcClaims {
        username,
        name,
        email,
        groups,
    })
}

/// Decode a compact JWT's payload segment (already-verified) into JSON.
fn decode_payload(jwt: &str) -> Result<serde_json::Value, AppError> {
    let payload_b64 = jwt
        .split('.')
        .nth(1)
        .ok_or_else(|| AppError::Internal("malformed ID token".to_string()))?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|e| AppError::Internal(format!("ID token payload decode: {e}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| AppError::Internal(format!("ID token payload JSON: {e}")))
}
