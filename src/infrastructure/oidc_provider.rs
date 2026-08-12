//! OpenID Connect provider for human browser login (ai/improvements grill batch
//! 2). openidconnect drives the security-critical parts — discovery, the PKCE
//! authorization-code flow, and ID-token verification (JWKS signature, nonce,
//! issuer, audience, expiry). The username/groups claims are configurable by
//! name, which openidconnect's typed claims can't express, so they are read
//! from the *already-verified* token payload as JSON.
//!
//! OIDC is human-only: Consumer/machine clients keep API tokens.

use crate::domain::models::{AppError, OidcConfig};
use crate::domain::ports::{OidcAuthorizeUrl, OidcClaims, OidcFlow};
use base64::Engine;
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse, reqwest,
};

/// The openidconnect-backed implementation of the [`OidcFlow`] port.
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
/// SSRF guidance for token/discovery requests.
fn http_client() -> Result<reqwest::Client, AppError> {
    reqwest::ClientBuilder::new()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AppError::Internal(format!("OIDC HTTP client: {e}")))
}

async fn discover(
    config: &OidcConfig,
    http: &reqwest::Client,
) -> Result<CoreProviderMetadata, AppError> {
    let issuer = IssuerUrl::new(config.issuer_url.clone())
        .map_err(|e| AppError::BadRequest(format!("OIDC issuer URL: {e}")))?;
    CoreProviderMetadata::discover_async(issuer, http)
        .await
        .map_err(|e| AppError::Internal(format!("OIDC discovery failed: {e}")))
}

/// Build the provider authorization URL plus the state/nonce/PKCE the callback
/// verifies against.
async fn authorize_url(config: &OidcConfig) -> Result<OidcAuthorizeUrl, AppError> {
    let http = http_client()?;
    let metadata = discover(config, &http).await?;
    let redirect = RedirectUrl::new(config.redirect_url.clone())
        .map_err(|e| AppError::BadRequest(format!("OIDC redirect URL: {e}")))?;
    // Inlined (not returned from a helper): after configuration the client's
    // endpoint typestate differs from the default `CoreClient` and cannot be
    // named without spelling out ~17 type parameters.
    let client = CoreClient::from_provider_metadata(
        metadata,
        ClientId::new(config.client_id.clone()),
        config.client_secret.clone().map(ClientSecret::new),
    )
    .set_redirect_uri(redirect);

    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let mut request = client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .set_pkce_challenge(challenge);
    for scope in &config.scopes {
        if scope != "openid" {
            request = request.add_scope(Scope::new(scope.clone()));
        }
    }
    let (url, state, nonce) = request.url();
    Ok(OidcAuthorizeUrl {
        url: url.to_string(),
        state: state.secret().clone(),
        nonce: nonce.secret().clone(),
        pkce_verifier: verifier.secret().clone(),
    })
}

/// Exchange the authorization code for tokens, verify the ID token, and resolve
/// the identity. Verification (signature via JWKS, nonce, issuer, audience,
/// expiry) is done by openidconnect; a failure is an authentication failure.
async fn exchange_and_verify(
    config: &OidcConfig,
    code: String,
    pkce_verifier: String,
    expected_nonce: String,
) -> Result<OidcClaims, AppError> {
    let http = http_client()?;
    let metadata = discover(config, &http).await?;
    let redirect = RedirectUrl::new(config.redirect_url.clone())
        .map_err(|e| AppError::BadRequest(format!("OIDC redirect URL: {e}")))?;
    let client = CoreClient::from_provider_metadata(
        metadata,
        ClientId::new(config.client_id.clone()),
        config.client_secret.clone().map(ClientSecret::new),
    )
    .set_redirect_uri(redirect);

    let token_response = client
        .exchange_code(AuthorizationCode::new(code))
        .map_err(|e| AppError::Internal(format!("OIDC token request build: {e}")))?
        .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier))
        .request_async(&http)
        .await
        .map_err(|e| {
            tracing::warn!("OIDC code exchange failed: {e}");
            AppError::Unauthorized
        })?;

    let id_token = token_response.id_token().ok_or(AppError::Unauthorized)?;

    // This is the verification: signature (against the discovered JWKS), nonce,
    // issuer, audience and expiry. An error here is a rejected login.
    let verifier = client.id_token_verifier();
    let claims = id_token
        .claims(&verifier, &Nonce::new(expected_nonce))
        .map_err(|e| {
            tracing::warn!("OIDC ID token rejected: {e}");
            AppError::Unauthorized
        })?;
    let sub = claims.subject().as_str().to_string();

    // Flexible claims from the (now-verified) payload. Reading the payload JSON
    // is safe here precisely because the token was just verified above.
    let payload = decode_payload(&id_token.to_string())?;
    let username = payload
        .get(&config.username_claim)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or(sub);
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
