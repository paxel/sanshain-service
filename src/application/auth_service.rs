use crate::domain::models::*;
use crate::domain::permissions::Role;
use crate::domain::ports::{AuthProvider, NewApiToken, OidcFlow, SpecRepository};
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use rand::distr::{Alphanumeric, SampleString};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use tracing::instrument;

pub fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("Hashing error: {}", e)))?
        .to_string();
    Ok(password_hash)
}

pub fn verify_password(password: &str, hash: &str) -> Result<bool, AppError> {
    let argon2 = Argon2::default();
    let parsed_hash = argon2::PasswordHash::new(hash)
        .map_err(|e| AppError::Internal(format!("Invalid hash format: {}", e)))?;
    Ok(argon2
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

/// [`hash_password`], off the async worker thread.
pub async fn hash_password_async(password: &str) -> Result<String, AppError> {
    let password = password.to_string();
    super::run_cpu_bound(move || hash_password(&password)).await
}

/// [`verify_password`], off the async worker thread.
pub async fn verify_password_async(password: &str, hash: &str) -> Result<bool, AppError> {
    let password = password.to_string();
    let hash = hash.to_string();
    super::run_cpu_bound(move || verify_password(&password, &hash)).await
}

pub fn generate_random_password() -> String {
    Alphanumeric.sample_string(&mut rand::rng(), 16)
}

/// Claim every configured root username that has no user row yet.
///
/// Root authority is a pure username match, so an unclaimed root name is a
/// standing invitation: whoever registers it (or gets a directory shadow
/// account under it) silently becomes superuser. Claiming it with a locked
/// account (random password, no stored roles — root needs none) closes that
/// on every instance, including 1.x upgrades whose databases predate the
/// bootstrap admin.
pub async fn claim_root_usernames(
    repo: &impl SpecRepository,
    root_users: &crate::domain::permissions::RootUsers,
) -> Result<(), AppError> {
    for username in root_users.usernames() {
        if repo.find_user(username).await?.is_none() {
            let hash = hash_password(&generate_random_password())?;
            repo.create_user(username, &hash, true).await?;
            tracing::info!(
                "Claimed root username '{}' with a placeholder account: it has no usable local password; the first LDAP or OIDC login under this name signs in as it and it becomes that directory user's shadow account",
                username
            );
        }
    }
    Ok(())
}

/// The auth mode a fresh instance starts in: `INITIAL_AUTH_MODE` as
/// configured, Off when unset or empty. Dev needs the
/// `ALLOW_INSECURE_DEV_MODE` gate open, as it does when switched later.
pub fn initial_auth_mode(
    configured: Option<&str>,
    dev_gate_open: bool,
) -> Result<AuthMode, String> {
    let Some(value) = configured.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(AuthMode::Disabled);
    };
    let mode: AuthMode = value.parse().map_err(|_| {
        format!("INITIAL_AUTH_MODE '{value}' is not one of off, dev, local, ldap, oidc")
    })?;
    if mode == AuthMode::Dev && !dev_gate_open {
        return Err("INITIAL_AUTH_MODE=dev needs ALLOW_INSECURE_DEV_MODE=true".to_string());
    }
    Ok(mode)
}

/// The initial admin's password: `INITIAL_ADMIN_PASSWORD` as configured, or a
/// random one.
///
/// Set but empty counts as unset. The Helm chart and the Kustomize Secret used
/// to render the key with an empty value when nothing was configured, and ""
/// reached the service as the password of the bootstrap administrator.
pub fn initial_admin_password(configured: Option<String>) -> String {
    configured
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(generate_random_password)
}

/// Where the first-login message sends the operator: the admin page under
/// `PUBLIC_BASE_URL` when it is set, under the bind address otherwise.
pub fn initial_admin_url(public_base_url: Option<&str>, bind_address: &str) -> String {
    match public_base_url {
        Some(base) => format!("{}/admin.html", base.trim_end_matches('/')),
        None => format!("http://{bind_address}/admin.html"),
    }
}

/// Create the bootstrap administrator when no user exists. `admin_url` is
/// where the printed message sends the operator, see [`initial_admin_url`].
pub async fn ensure_initial_admin(
    repo: &impl SpecRepository,
    admin_url: &str,
) -> Result<(), AppError> {
    let count = repo.user_count().await?;
    if count == 0 {
        let username =
            std::env::var("INITIAL_ADMIN_USERNAME").unwrap_or_else(|_| "root".to_string());
        let password = initial_admin_password(std::env::var("INITIAL_ADMIN_PASSWORD").ok());
        let hash = hash_password(&password)?;
        let admin = repo.create_user(&username, &hash, true).await?;
        // The flag is gone, so the bootstrap account's authority comes from an
        // explicit grant. `SANSHAIN_ROOT_USERS` covers it as well by default,
        // but the grant is what makes it visible in the UI as an ordinary role.
        repo.grant_user_role(admin.id, Role::Admin.as_str()).await?;
        // A fresh instance starts in Off, where only the admin signs in and
        // picks the mode, unless `INITIAL_AUTH_MODE` names one for automated
        // installs. The migrations' default of `local` predates Off.
        let mode = match initial_auth_mode(
            std::env::var("INITIAL_AUTH_MODE").ok().as_deref(),
            dev_mode_gate_open(),
        ) {
            Ok(mode) => mode,
            Err(reason) => {
                eprintln!("[INITIAL SETUP] {reason}; starting in Off");
                AuthMode::Disabled
            }
        };
        set_auth_mode(repo, &mode).await?;
        eprintln!(
            "[INITIAL SETUP] Admin user created. Username: {}, Password: {}",
            username, password
        );
        eprintln!("[INITIAL SETUP] Change this password immediately at {admin_url}");
    }
    Ok(())
}

#[instrument(skip_all)]
pub async fn login(
    repo: &impl SpecRepository,
    username: &str,
    password: &str,
) -> Result<(Session, User), AppError> {
    // An empty password is never a credential: refused before any lookup, so
    // an account that was created with an empty hash cannot be entered.
    if password.is_empty() {
        return Err(AppError::Unauthorized);
    }
    let user = repo
        .find_user(username)
        .await?
        .filter(|user| !user.is_service_account())
        .ok_or(AppError::Unauthorized)?;

    if !user.approved {
        return Err(AppError::Forbidden);
    }

    if !verify_password_async(password, &user.password_hash).await? {
        return Err(AppError::Unauthorized);
    }

    let expires_at = (chrono::Utc::now() + chrono::Duration::days(7)).to_rfc3339();
    let session = repo.create_session(user.id, &expires_at).await?;
    record_sign_in(repo, &session, SignInMethod::Local).await?;
    Ok((session, user))
}

pub async fn change_password(
    repo: &impl SpecRepository,
    user: &User,
    current_token: Option<&str>,
    old_password: &str,
    new_password: &str,
) -> Result<Option<Session>, AppError> {
    if !verify_password_async(old_password, &user.password_hash).await? {
        return Err(AppError::Unauthorized);
    }
    let new_hash = hash_password_async(new_password).await?;
    repo.update_password(user.id, &new_hash).await?;

    if let Some(token) = current_token {
        repo.delete_session(token).await?;
        let expires_at = (chrono::Utc::now() + chrono::Duration::days(7)).to_rfc3339();
        let session = repo.create_session(user.id, &expires_at).await?;
        // The old password was just checked here, so this is a local sign-in.
        record_sign_in(repo, &session, SignInMethod::Local).await?;
        return Ok(Some(session));
    }

    Ok(None)
}

/// The production safety gate for dev mode. Dev mode only takes effect when the
/// `ALLOW_INSECURE_DEV_MODE` environment variable is set to `true`; without it
/// Dev cannot be picked as the auth mode, so authentication cannot be switched
/// off by accident.
pub fn dev_mode_gate_open() -> bool {
    std::env::var("ALLOW_INSECURE_DEV_MODE").unwrap_or_default() == "true"
}

/// Whether dev mode is *effective*: the auth mode is Dev **and** the
/// [`dev_mode_gate_open`] safety gate is open. Fails closed: a stored Dev
/// without the gate resolves to `false`, so protected endpoints keep enforcing
/// authentication.
pub async fn get_dev_mode(repo: &impl SpecRepository) -> Result<bool, AppError> {
    Ok(get_auth_mode(repo).await? == AuthMode::Dev && dev_mode_gate_open())
}

const DEV_USER: &str = "dev_user";

/// The account dev mode's tokenless API calls act as. A service account: it
/// has no password and never signs in, so an instance that was once in dev
/// mode keeps no admin login behind.
pub async fn ensure_dev_user(repo: &impl SpecRepository) -> Result<User, AppError> {
    lock_dev_user(repo).await?;
    if let Some(user) = repo.find_user(DEV_USER).await? {
        return Ok(user);
    }
    let user = repo
        .create_user(DEV_USER, SERVICE_ACCOUNT_PASSWORD_HASH, true)
        .await?;
    repo.grant_user_role(user.id, Role::Admin.as_str()).await?;
    Ok(user)
}

/// Lock the password of a `dev_user` created before 2.10, which carried a
/// password written in the source and so signed in as an admin on any
/// instance that had ever been in dev mode. Answers whether one was locked.
pub async fn lock_dev_user(repo: &impl SpecRepository) -> Result<bool, AppError> {
    match repo.find_user(DEV_USER).await? {
        Some(user) if !user.is_service_account() => {
            repo.update_password(user.id, SERVICE_ACCOUNT_PASSWORD_HASH)
                .await?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

#[instrument(skip_all)]
pub async fn validate_session(
    repo: &impl SpecRepository,
    token: &str,
) -> Result<Option<(User, Session)>, AppError> {
    Ok(repo.validate_session(token).await?)
}

pub async fn logout(repo: &impl SpecRepository, token: &str) -> Result<(), AppError> {
    repo.delete_session(token).await?;
    Ok(())
}

#[instrument(skip_all)]
pub async fn register_user(
    repo: &impl SpecRepository,
    root_users: &crate::domain::permissions::RootUsers,
    username: &str,
    password: &str,
) -> Result<bool, AppError> {
    if !super::sign_in_service::registration_open(repo).await? {
        return Err(AppError::ForbiddenWithReason(
            "registration is closed on this instance".to_string(),
        ));
    }
    // Root usernames hold every permission by pure name match, so an
    // unclaimed one must never be claimable through open registration.
    if root_users.contains(username) {
        return Err(AppError::Conflict("This username is reserved".to_string()));
    }
    if repo.find_user(username).await?.is_some() {
        return Err(AppError::Conflict("User already exists".to_string()));
    }
    let hash = hash_password_async(password).await?;
    let auto_approve = repo
        .get_setting("auto_approve_users")
        .await?
        .unwrap_or("false".to_string())
        == "true";
    repo.create_user(username, &hash, auto_approve).await?;
    Ok(auto_approve)
}

pub async fn list_users(repo: &impl SpecRepository) -> Result<Vec<User>, AppError> {
    Ok(repo.list_users().await?)
}

pub async fn approve_user(repo: &impl SpecRepository, user_id: i64) -> Result<bool, AppError> {
    Ok(repo.approve_user(user_id).await?)
}

pub async fn admin_delete_user(
    repo: &impl SpecRepository,
    root_users: &crate::domain::permissions::RootUsers,
    actor: Option<&crate::domain::permissions::Actor>,
    user_id: i64,
) -> Result<bool, AppError> {
    crate::application::authz::require_admin_to_delete_user(repo, root_users, actor, user_id)
        .await?;
    Ok(repo.delete_user(user_id).await?)
}

pub async fn get_auto_approve_users(repo: &impl SpecRepository) -> Result<bool, AppError> {
    let val = repo
        .get_setting("auto_approve_users")
        .await?
        .unwrap_or("false".to_string());
    Ok(val == "true")
}

pub async fn set_auto_approve_users(
    repo: &impl SpecRepository,
    enabled: bool,
) -> Result<(), AppError> {
    repo.set_setting("auto_approve_users", if enabled { "true" } else { "false" })
        .await?;
    Ok(())
}

pub async fn get_auth_mode(repo: &impl SpecRepository) -> Result<AuthMode, AppError> {
    let val = repo
        .get_setting("auth_mode")
        .await?
        .unwrap_or("disabled".to_string());
    // One parser (AuthMode::from_str) owns the string mapping and aliases; an
    // unrecognized stored value fails closed to Disabled.
    Ok(val.parse().unwrap_or(AuthMode::Disabled))
}

pub async fn set_auth_mode(repo: &impl SpecRepository, mode: &AuthMode) -> Result<(), AppError> {
    let val = mode.as_str();
    repo.set_setting("auth_mode", val).await?;
    Ok(())
}

pub async fn get_ldap_config(repo: &impl SpecRepository) -> Result<Option<LdapConfig>, AppError> {
    let json = repo.get_setting("ldap_config").await?;
    if let Some(s) = json {
        Ok(serde_json::from_str(&s)
            .map_err(|e| AppError::Internal(format!("LDAP config parse error: {}", e)))?)
    } else {
        Ok(None)
    }
}

pub async fn get_oidc_config(repo: &impl SpecRepository) -> Result<Option<OidcConfig>, AppError> {
    let json = repo.get_setting("oidc_config").await?;
    if let Some(s) = json {
        Ok(serde_json::from_str(&s)
            .map_err(|e| AppError::Internal(format!("OIDC config parse error: {}", e)))?)
    } else {
        Ok(None)
    }
}

pub async fn set_oidc_config(
    repo: &impl SpecRepository,
    config: &OidcConfig,
) -> Result<(), AppError> {
    config.validate().map_err(AppError::BadRequest)?;
    let json = serde_json::to_string(config)
        .map_err(|e| AppError::Internal(format!("OIDC config serialize error: {}", e)))?;
    repo.set_setting("oidc_config", &json).await?;
    Ok(())
}

pub async fn set_ldap_config(
    repo: &impl SpecRepository,
    config: &LdapConfig,
) -> Result<(), AppError> {
    let json = serde_json::to_string(config)
        .map_err(|e| AppError::Internal(format!("LDAP config serialize error: {}", e)))?;
    repo.set_setting("ldap_config", &json).await?;
    // The admin-group setting is no longer consulted directly — a directory
    // group grants roles through the group mapping. Carrying it over keeps an
    // operator who configures it here from finding their directory
    // administrators locked out.
    crate::application::directory_roles::migrate_admin_group(repo, &config.admin_group).await?;
    Ok(())
}

/// Bring a stored configuration's admin group into the group mapping.
///
/// Runs at startup so an instance upgrading from a release where `admin_group`
/// was consulted directly keeps its directory-backed administrators, without
/// anyone having to re-save the configuration first.
pub async fn migrate_stored_admin_group(repo: &impl SpecRepository) -> Result<(), AppError> {
    if let Some(config) = get_ldap_config(repo).await? {
        crate::application::directory_roles::migrate_admin_group(repo, &config.admin_group).await?;
    }
    Ok(())
}

pub async fn login_with_provider(
    repo: &impl SpecRepository,
    provider: &impl AuthProvider,
    username: &str,
    password: &str,
) -> Result<Session, AppError> {
    // Same rule as the local login: an empty password is refused outright, not
    // handed to the directory (an empty simple bind is anonymous on many).
    if password.is_empty() {
        return Err(AppError::Unauthorized);
    }
    // The provider's answer establishes identity; the username is all that is
    // needed from it, since privileges are resolved separately.
    let _authenticated = provider
        .authenticate(username, password)
        .await
        .map_err(|_| AppError::Unauthorized)?;

    // Ensure user exists in local DB for session management
    let local_user = match repo.find_user(username).await? {
        Some(u) => u,
        None => {
            // Create a stub user with a random password that can't be used for local login easily
            let hash = hash_password(&generate_random_password())?;
            // A directory user's privileges come from their directory groups,
            // resolved on every check. Nothing about them is captured here.
            repo.create_user(username, &hash, true).await?
        }
    };

    let expires_at = (chrono::Utc::now() + chrono::Duration::days(7)).to_rfc3339();
    let session = repo.create_session(local_user.id, &expires_at).await?;
    record_sign_in(repo, &session, SignInMethod::Ldap).await?;
    Ok(session)
}

/// What an OIDC login round-trip stashes in the flow cookie between the
/// redirect out and the callback: the CSRF state the provider must echo, the
/// nonce baked into the ID token, and the PKCE verifier for the code exchange.
pub struct OidcFlowState {
    pub state: String,
    pub nonce: String,
    pub pkce_verifier: String,
}

impl OidcFlowState {
    /// Cookie-value encoding. `|` never occurs in the base64url alphabets the
    /// three parts are drawn from.
    pub fn encode(&self) -> String {
        format!("{}|{}|{}", self.state, self.nonce, self.pkce_verifier)
    }

    /// Inverse of [`encode`](Self::encode). `None` for anything but three
    /// non-empty parts — a malformed cookie is not a login in progress.
    pub fn decode(raw: &str) -> Option<Self> {
        let mut parts = raw.splitn(3, '|');
        let state = parts.next()?.to_string();
        let nonce = parts.next()?.to_string();
        let pkce_verifier = parts.next()?.to_string();
        (!state.is_empty() && !nonce.is_empty() && !pkce_verifier.is_empty()).then_some(Self {
            state,
            nonce,
            pkce_verifier,
        })
    }
}

/// Begin OIDC login: hand back the provider authorization URL to redirect the
/// browser to, and the flow state the callback will verify against.
pub async fn oidc_begin(
    repo: &impl SpecRepository,
    provider: &impl OidcFlow,
) -> Result<(String, OidcFlowState), AppError> {
    oidc_must_be_the_source(repo).await?;
    let config = get_oidc_config(repo)
        .await?
        .ok_or_else(|| AppError::BadRequest("OIDC is not configured".to_string()))?;
    let authorize = provider.authorize_url(&config).await?;
    Ok((
        authorize.url,
        OidcFlowState {
            state: authorize.state,
            nonce: authorize.nonce,
            pkce_verifier: authorize.pkce_verifier,
        },
    ))
}

/// Complete OIDC login: check the provider-echoed state against the flow
/// cookie (CSRF), exchange the code, verify the ID token, and mint a session
/// for the resolved identity.
pub async fn oidc_complete(
    repo: &impl SpecRepository,
    root_users: &crate::domain::permissions::RootUsers,
    provider: &impl OidcFlow,
    code: String,
    callback_state: &str,
    flow: OidcFlowState,
) -> Result<Session, AppError> {
    // CSRF: the provider echoes our state; it must match the cookie.
    if callback_state != flow.state {
        return Err(AppError::Unauthorized);
    }
    oidc_must_be_the_source(repo).await?;
    let config = get_oidc_config(repo)
        .await?
        .ok_or_else(|| AppError::BadRequest("OIDC is not configured".to_string()))?;
    let claims = provider
        .exchange_and_verify(&config, code, flow.pkce_verifier, flow.nonce)
        .await?;
    let mode = get_auth_mode(repo).await?;
    if super::sign_in_service::admins_only(&mode) {
        let existing = repo.find_user(&claims.username).await?;
        let in_admin_group = !config.admin_group.is_empty()
            && claims.groups.iter().any(|g| g == &config.admin_group);
        let may = in_admin_group
            || super::sign_in_service::may_sign_in_as_admin(
                repo,
                root_users,
                &claims.username,
                existing.as_ref(),
                &[],
            )
            .await?;
        if !may {
            return Err(super::sign_in_service::admins_only_refusal(&mode));
        }
    }
    login_oidc(
        repo,
        &claims.username,
        claims.name.as_deref(),
        claims.email.as_deref(),
        &claims.groups,
        &config.admin_group,
    )
    .await
}

/// SSO signs in only while OIDC is the source: the mode, or the one Off
/// remembers.
async fn oidc_must_be_the_source(repo: &impl SpecRepository) -> Result<(), AppError> {
    if super::sign_in_service::sign_in_source(repo).await? != AuthMode::Oidc {
        return Err(AppError::ForbiddenWithReason(
            "single sign-on is not the sign-in source of this instance".to_string(),
        ));
    }
    Ok(())
}

/// Mint the session for a verified OIDC identity: the caller has already
/// verified the ID token, so the username and groups are trusted here. Find or
/// create the local shadow user (with an unusable random password — OIDC users
/// log in via the provider, not locally) and mint a session. Mirrors
/// [`login_with_provider`] minus the credential check the provider already did.
///
/// Group → admin: unlike LDAP (which re-queries the directory on every check),
/// an OIDC token is a login-time snapshot, so a user in the configured
/// `admin_group` is granted the Admin role here. Grant-only — demotion is a
/// manual revoke, not an automatic one, so a token that omits the group cannot
/// silently strip a deliberately-granted admin.
pub async fn login_oidc(
    repo: &impl SpecRepository,
    username: &str,
    name: Option<&str>,
    email: Option<&str>,
    groups: &[String],
    admin_group: &str,
) -> Result<Session, AppError> {
    let local_user = match repo.find_user(username).await? {
        // A service account never signs in, not even under a provider's name.
        Some(u) if u.is_service_account() => return Err(AppError::Unauthorized),
        Some(u) => u,
        None => {
            let hash = hash_password_async(&generate_random_password()).await?;
            repo.create_user(username, &hash, true).await?
        }
    };
    if !admin_group.is_empty() && groups.iter().any(|g| g == admin_group) {
        repo.grant_user_role(local_user.id, Role::Admin.as_str())
            .await?;
    }
    refresh_provider_identity(repo, local_user.id, name, email).await?;
    let expires_at = (chrono::Utc::now() + chrono::Duration::days(7)).to_rfc3339();
    let session = repo.create_session(local_user.id, &expires_at).await?;
    record_sign_in(repo, &session, SignInMethod::Oidc).await?;
    Ok(session)
}

/// Note a sign-in: on the session, so the pages know what it may change, and
/// on the person, so the user list shows where they come from.
pub(crate) async fn record_sign_in(
    repo: &impl SpecRepository,
    session: &Session,
    method: SignInMethod,
) -> Result<(), AppError> {
    repo.set_session_sign_in(&session.token, method).await?;
    repo.record_user_sign_in(session.user_id, method).await?;
    Ok(())
}

/// The kind of each account for the user list: `service` for a service
/// account, otherwise how the person last signed in (`local`, `ldap`,
/// `oidc`), or `unknown` before their first sign-in since 2.11.
pub async fn account_kinds(
    repo: &impl SpecRepository,
    users: &[User],
) -> Result<std::collections::HashMap<i64, &'static str>, AppError> {
    let last: std::collections::HashMap<i64, SignInMethod> = repo
        .list_user_profiles()
        .await?
        .into_iter()
        .filter_map(|p| p.last_sign_in.map(|m| (p.user_id, m)))
        .collect();
    Ok(users
        .iter()
        .map(|user| {
            let kind = if user.is_service_account() {
                "service"
            } else {
                last.get(&user.id).map_or("unknown", SignInMethod::as_str)
            };
            (user.id, kind)
        })
        .collect())
}

/// Take over what the directory or the OIDC provider says about a person at
/// sign-in: their name to show, and the Gravatar key of their e-mail address.
/// The provider is the only source for both, so what it no longer says is
/// cleared.
#[tracing::instrument(skip(repo, display_name, email))]
pub(crate) async fn refresh_provider_identity(
    repo: &impl SpecRepository,
    user_id: i64,
    display_name: Option<&str>,
    email: Option<&str>,
) -> Result<(), AppError> {
    let display_name = display_name.map(str::trim).filter(|n| !n.is_empty());
    repo.set_display_name(user_id, display_name).await?;
    let email = email.map(str::trim).filter(|e| !e.is_empty());
    repo.set_avatar_hash(user_id, email.map(gravatar_key).as_deref())
        .await?;
    Ok(())
}

/// The key Gravatar files a picture under: the SHA-256 of the trimmed,
/// lowercased address, in hex. Only this leaves the server, never the address.
pub fn gravatar_key(email: &str) -> String {
    hex::encode(Sha256::digest(email.trim().to_lowercase().as_bytes()))
}

/// Whether the pages load pictures from gravatar.com. Off unless an
/// administrator switches it on: every picture is a request leaving the
/// company.
pub async fn get_gravatar_enabled(repo: &impl SpecRepository) -> Result<bool, AppError> {
    Ok(repo.get_setting("gravatar_enabled").await?.as_deref() == Some("true"))
}

pub async fn set_gravatar_enabled(
    repo: &impl SpecRepository,
    enabled: bool,
) -> Result<(), AppError> {
    repo.set_setting("gravatar_enabled", if enabled { "true" } else { "false" })
        .await?;
    Ok(())
}

/// Every Gravatar key to show, by user id; empty while Gravatar is off.
pub async fn gravatar_keys(
    repo: &impl SpecRepository,
) -> Result<std::collections::HashMap<i64, String>, AppError> {
    if !get_gravatar_enabled(repo).await? {
        return Ok(std::collections::HashMap::new());
    }
    Ok(repo
        .list_user_profiles()
        .await?
        .into_iter()
        .filter_map(|p| p.avatar_hash.map(|key| (p.user_id, key)))
        .collect())
}

/// The longest display name accepted.
pub const MAX_DISPLAY_NAME_CHARS: usize = 100;

/// The profile of `user_id`; an empty one when nothing is stored yet.
pub async fn user_profile(
    repo: &impl SpecRepository,
    user_id: i64,
) -> Result<UserProfile, AppError> {
    Ok(repo
        .list_user_profiles()
        .await?
        .into_iter()
        .find(|p| p.user_id == user_id)
        .unwrap_or(UserProfile {
            user_id,
            ..UserProfile::default()
        }))
}

/// Every stored display name, by user id.
pub async fn display_names(
    repo: &impl SpecRepository,
) -> Result<std::collections::HashMap<i64, String>, AppError> {
    Ok(repo
        .list_user_profiles()
        .await?
        .into_iter()
        .filter_map(|p| p.display_name.map(|name| (p.user_id, name)))
        .collect())
}

/// An audit entry with the display name of whoever made it.
#[derive(Debug, Clone, Serialize)]
pub struct AuditLogView {
    #[serde(flatten)]
    pub entry: AuditLogEntry,
    /// `None` when the person has none, or the account is gone.
    pub display_name: Option<String>,
}

/// Every display name, keyed by username: what an audit entry names.
pub async fn display_names_by_username(
    repo: &impl SpecRepository,
) -> Result<std::collections::HashMap<String, String>, AppError> {
    let names = display_names(repo).await?;
    Ok(repo
        .list_users()
        .await?
        .into_iter()
        .filter_map(|u| names.get(&u.id).map(|name| (u.username, name.clone())))
        .collect())
}

/// Attach the current display name of each entry's user.
pub async fn with_display_names(
    repo: &impl SpecRepository,
    entries: Vec<AuditLogEntry>,
) -> Result<Vec<AuditLogView>, AppError> {
    let by_username = display_names_by_username(repo).await?;
    Ok(entries
        .into_iter()
        .map(|entry| AuditLogView {
            display_name: by_username.get(&entry.username).cloned(),
            entry,
        })
        .collect())
}

/// A display name as entered: trimmed, blank clears it, too long is refused.
fn clean_display_name(name: &str) -> Result<Option<String>, AppError> {
    let name = name.trim();
    if name.chars().count() > MAX_DISPLAY_NAME_CHARS {
        return Err(AppError::BadRequest(format!(
            "a display name has at most {MAX_DISPLAY_NAME_CHARS} characters"
        )));
    }
    Ok((!name.is_empty()).then(|| name.to_string()))
}

/// A person sets their own display name. Refused for someone whose name comes
/// from the directory or the OIDC provider: it would be overwritten at their
/// next sign-in.
pub async fn set_own_display_name(
    repo: &impl SpecRepository,
    user: &User,
    name: &str,
) -> Result<Option<String>, AppError> {
    if user_profile(repo, user.id)
        .await?
        .display_name_from_provider()
    {
        return Err(AppError::BadRequest(
            "your name comes from the directory or the identity provider".to_string(),
        ));
    }
    let name = clean_display_name(name)?;
    repo.set_display_name(user.id, name.as_deref()).await?;
    Ok(name)
}

/// An administrator sets the display name of a service account.
pub async fn set_service_account_display_name(
    repo: &impl SpecRepository,
    user_id: i64,
    name: &str,
) -> Result<Option<String>, AppError> {
    let account = repo
        .list_users()
        .await?
        .into_iter()
        .find(|u| u.id == user_id)
        .ok_or_else(|| AppError::NotFound("no such user".to_string()))?;
    if !account.is_service_account() {
        return Err(AppError::BadRequest(
            "only a service account's display name is set by an administrator".to_string(),
        ));
    }
    let name = clean_display_name(name)?;
    repo.set_display_name(user_id, name.as_deref()).await?;
    Ok(name)
}

/// How the session behind `token` signed in, if Sanshain recorded it.
pub async fn session_sign_in(
    repo: &impl SpecRepository,
    token: &str,
) -> Result<Option<SignInMethod>, AppError> {
    Ok(repo.session_sign_in(token).await?)
}

#[instrument(skip_all)]
pub async fn create_api_token(
    repo: &impl SpecRepository,
    user_id: i64,
    name: &str,
    expires_in_days: u64,
    scope: ApiTokenScope,
) -> Result<(String, String), AppError> {
    create_api_token_issued(repo, user_id, name, expires_in_days, scope, None).await
}

/// [`create_api_token`], recording the administrator who issued it for a
/// service account.
pub(crate) async fn create_api_token_issued(
    repo: &impl SpecRepository,
    user_id: i64,
    name: &str,
    expires_in_days: u64,
    scope: ApiTokenScope,
    issued_by: Option<&str>,
) -> Result<(String, String), AppError> {
    // 256 bits of CSPRNG entropy, matching session tokens. Storage and
    // validation are format-agnostic (SHA-256 of the raw string), so older
    // 122-bit UUID-based tokens keep validating unchanged.
    let mut token_bytes = [0u8; 32];
    {
        use rand::RngExt;
        rand::rng().fill(&mut token_bytes);
    }
    let raw_token = format!("san_{}", hex::encode(token_bytes));
    let mut hasher = Sha256::new();
    hasher.update(raw_token.as_bytes());
    let token_hash = hex::encode(hasher.finalize());

    let id = Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();
    let expires_at =
        (chrono::Utc::now() + chrono::Duration::days(expires_in_days as i64)).to_rfc3339();

    repo.create_api_token(NewApiToken {
        id: &id,
        user_id,
        name,
        token_hash: &token_hash,
        scope,
        created_at: &created_at,
        expires_at: &expires_at,
        issued_by,
    })
    .await?;
    Ok((id, raw_token))
}

pub async fn list_api_tokens(
    repo: &impl SpecRepository,
    user_id: i64,
) -> Result<Vec<ApiToken>, AppError> {
    Ok(repo.list_api_tokens(user_id).await?)
}

/// Revokes the token, returning whether one was found and owned by `user_id`
/// so the caller can answer 404 instead of reporting a success that never
/// happened.
pub async fn revoke_api_token(
    repo: &impl SpecRepository,
    token_id: &str,
    user_id: i64,
) -> Result<bool, AppError> {
    Ok(repo.delete_api_token(token_id, user_id).await?)
}

/// How long a token's `last_used_at` stamp is left alone after a use.
///
/// A client polling every few seconds would otherwise turn every request into
/// a write; a minute is precise enough for "when was this token last used?".
pub const LAST_USED_STAMP_INTERVAL: chrono::Duration = chrono::Duration::minutes(1);

/// Whether `last_used_at` is stale enough to be stamped again at `now`.
///
/// Accepts both spellings a stored stamp may have — RFC 3339 (what this code
/// writes) and the bare `YYYY-MM-DD HH:MM:SS` UTC form older rows carry. An
/// unreadable stamp is treated as due, so one bad row heals rather than sticks.
pub fn last_used_stamp_due(last_used_at: Option<&str>, now: chrono::DateTime<chrono::Utc>) -> bool {
    let Some(stamp) = last_used_at else {
        return true;
    };
    let parsed = chrono::DateTime::parse_from_rfc3339(stamp)
        .map(|t| t.with_timezone(&chrono::Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S").map(|t| t.and_utc())
        });
    match parsed {
        Ok(stamped) => now - stamped >= LAST_USED_STAMP_INTERVAL,
        Err(_) => true,
    }
}

/// Resolve a raw API token to its identity, if it is live.
///
/// Stamps `last_used_at` at most once per [`LAST_USED_STAMP_INTERVAL`] per
/// token. A failed stamp does not fail the request: the token is valid, and a
/// missing "last used" is a cosmetic loss.
#[instrument(skip_all)]
pub async fn validate_api_token(
    repo: &impl SpecRepository,
    raw_token: &str,
) -> Result<Option<ApiTokenIdentity>, AppError> {
    let mut hasher = Sha256::new();
    hasher.update(raw_token.as_bytes());
    let token_hash = hex::encode(hasher.finalize());

    let Some(identity) = repo.validate_api_token(&token_hash).await? else {
        return Ok(None);
    };
    let now = chrono::Utc::now();
    if last_used_stamp_due(identity.last_used_at.as_deref(), now)
        && let Err(e) = repo
            .touch_api_token(&identity.token_id, &now.to_rfc3339())
            .await
    {
        tracing::warn!("Could not stamp last_used_at for an API token: {}", e);
    }
    Ok(Some(identity))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::mock_repo::MockRepo;

    /// A root set that reserves no username used by these tests.
    fn no_root_users() -> crate::domain::permissions::RootUsers {
        crate::domain::permissions::RootUsers::resolve(Some("__unused-root__"), None)
    }

    #[tokio::test]
    async fn test_create_api_token_format_and_entropy() {
        let repo = MockRepo::new();
        let (id, token) = create_api_token(&repo, 1, "ci", 30, ApiTokenScope::Full)
            .await
            .unwrap();
        assert!(!id.is_empty());
        assert!(token.starts_with("san_"));
        let suffix = &token["san_".len()..];
        assert_eq!(suffix.len(), 64);
        assert_eq!(hex::decode(suffix).unwrap().len(), 32);

        let (_, second_token) = create_api_token(&repo, 1, "ci-2", 30, ApiTokenScope::Full)
            .await
            .unwrap();
        assert_ne!(token, second_token);
    }

    #[tokio::test]
    async fn create_api_token_stores_the_requested_scope() {
        let repo = MockRepo::new();
        let user = repo.create_user("owner", "h", true).await.unwrap();
        create_api_token(&repo, user.id, "ai", 30, ApiTokenScope::ReadOnly)
            .await
            .unwrap();
        create_api_token(&repo, user.id, "ci", 30, ApiTokenScope::Full)
            .await
            .unwrap();
        let listed = list_api_tokens(&repo, user.id).await.unwrap();
        let scope_of = |name: &str| listed.iter().find(|t| t.name == name).unwrap().scope;
        assert_eq!(scope_of("ai"), ApiTokenScope::ReadOnly);
        assert_eq!(scope_of("ci"), ApiTokenScope::Full);
    }

    #[tokio::test]
    async fn validate_api_token_resolves_identity_and_scope() {
        let repo = MockRepo::new();
        let user = repo.create_user("owner", "h", true).await.unwrap();
        let (id, raw) = create_api_token(&repo, user.id, "ai", 30, ApiTokenScope::ReadOnly)
            .await
            .unwrap();
        let identity = validate_api_token(&repo, &raw).await.unwrap().unwrap();
        assert_eq!(identity.token_id, id);
        assert_eq!(identity.scope, ApiTokenScope::ReadOnly);
        assert_eq!(identity.user.username, "owner");
        assert!(
            validate_api_token(&repo, "san_nope")
                .await
                .unwrap()
                .is_none()
        );
    }

    // Spec: last_used_at is written on use, at most once a minute per token.
    #[tokio::test]
    async fn validate_api_token_stamps_last_used_once_per_minute() {
        let repo = MockRepo::new();
        let user = repo.create_user("owner", "h", true).await.unwrap();
        let (_, raw) = create_api_token(&repo, user.id, "ci", 30, ApiTokenScope::Full)
            .await
            .unwrap();

        let first = validate_api_token(&repo, &raw).await.unwrap().unwrap();
        assert_eq!(first.last_used_at, None, "the stamp lands after validation");
        let stamped = list_api_tokens(&repo, user.id).await.unwrap()[0]
            .last_used_at
            .clone()
            .expect("first use stamps last_used_at");

        let second = validate_api_token(&repo, &raw).await.unwrap().unwrap();
        assert_eq!(second.last_used_at.as_deref(), Some(stamped.as_str()));
        let after_second = list_api_tokens(&repo, user.id).await.unwrap()[0]
            .last_used_at
            .clone();
        assert_eq!(
            after_second.as_deref(),
            Some(stamped.as_str()),
            "a use within the minute must not write again"
        );
    }

    #[test]
    fn last_used_stamp_is_due_when_missing_old_or_unreadable() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-16T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert!(last_used_stamp_due(None, now));
        assert!(last_used_stamp_due(Some("garbage"), now));
        assert!(last_used_stamp_due(Some("2026-10-16T11:59:00Z"), now));
        assert!(last_used_stamp_due(Some("2026-10-16 11:58:30"), now));
        assert!(!last_used_stamp_due(Some("2026-10-16T11:59:01Z"), now));
        assert!(!last_used_stamp_due(Some("2026-10-16 11:59:30"), now));
    }

    struct StubOidc;
    impl crate::domain::ports::OidcFlow for StubOidc {
        async fn authorize_url(
            &self,
            _config: &OidcConfig,
        ) -> Result<crate::domain::ports::OidcAuthorizeUrl, AppError> {
            Ok(crate::domain::ports::OidcAuthorizeUrl {
                url: "https://idp.example/authorize".to_string(),
                state: "st".to_string(),
                nonce: "no".to_string(),
                pkce_verifier: "pk".to_string(),
            })
        }

        async fn exchange_and_verify(
            &self,
            _config: &OidcConfig,
            code: String,
            pkce_verifier: String,
            expected_nonce: String,
        ) -> Result<crate::domain::ports::OidcClaims, AppError> {
            if code == "good" && pkce_verifier == "pk" && expected_nonce == "no" {
                Ok(crate::domain::ports::OidcClaims {
                    username: "alice".to_string(),
                    name: Some("Alice Liddell".to_string()),
                    email: None,
                    groups: Vec::new(),
                })
            } else {
                Err(AppError::Unauthorized)
            }
        }
    }

    async fn repo_with_oidc_config() -> MockRepo {
        let repo = MockRepo::new();
        repo.set_setting(
            "oidc_config",
            r#"{"issuer_url":"https://idp.example","client_id":"c","redirect_url":"https://app.example/auth/oidc/callback"}"#,
        )
        .await
        .unwrap();
        repo.set_setting("auth_mode", "oidc").await.unwrap();
        repo
    }

    #[test]
    fn oidc_flow_state_roundtrips_and_rejects_malformed() {
        let flow = OidcFlowState {
            state: "st".to_string(),
            nonce: "no".to_string(),
            pkce_verifier: "pk".to_string(),
        };
        let decoded = OidcFlowState::decode(&flow.encode()).unwrap();
        assert_eq!(decoded.state, "st");
        assert_eq!(decoded.nonce, "no");
        assert_eq!(decoded.pkce_verifier, "pk");

        assert!(OidcFlowState::decode("").is_none());
        assert!(OidcFlowState::decode("st|no").is_none());
        assert!(OidcFlowState::decode("|no|pk").is_none());
        assert!(OidcFlowState::decode("st||pk").is_none());
        assert!(OidcFlowState::decode("st|no|").is_none());
    }

    #[tokio::test]
    async fn oidc_begin_answers_url_and_flow_state() {
        let repo = repo_with_oidc_config().await;
        let (url, flow) = oidc_begin(&repo, &StubOidc).await.unwrap();
        assert_eq!(url, "https://idp.example/authorize");
        assert_eq!(flow.state, "st");
        assert_eq!(flow.nonce, "no");
        assert_eq!(flow.pkce_verifier, "pk");
    }

    #[tokio::test]
    async fn oidc_is_refused_while_it_is_not_the_sign_in_source() {
        let repo = repo_with_oidc_config().await;
        repo.set_setting("auth_mode", "local").await.unwrap();
        assert!(matches!(
            oidc_begin(&repo, &StubOidc).await,
            Err(AppError::ForbiddenWithReason(_))
        ));
        let flow = OidcFlowState {
            state: "st".to_string(),
            nonce: "no".to_string(),
            pkce_verifier: "pk".to_string(),
        };
        let result = oidc_complete(
            &repo,
            &no_root_users(),
            &StubOidc,
            "good".to_string(),
            "st",
            flow,
        )
        .await;
        assert!(matches!(result, Err(AppError::ForbiddenWithReason(_))));
        assert!(repo.find_user("alice").await.unwrap().is_none());
    }

    /// Off remembers OIDC: an administrator signs in through it, anyone else
    /// is refused without an account being created for them.
    #[tokio::test]
    async fn during_off_oidc_admits_only_administrators() {
        let repo = repo_with_oidc_config().await;
        crate::application::sign_in_service::switch_auth_mode(&repo, AuthMode::Disabled)
            .await
            .unwrap();
        let flow = || OidcFlowState {
            state: "st".to_string(),
            nonce: "no".to_string(),
            pkce_verifier: "pk".to_string(),
        };
        let result = oidc_complete(
            &repo,
            &no_root_users(),
            &StubOidc,
            "good".to_string(),
            "st",
            flow(),
        )
        .await;
        assert!(
            matches!(result, Err(AppError::ForbiddenWithReason(ref m)) if m.contains("maintenance"))
        );
        assert!(
            repo.find_user("alice").await.unwrap().is_none(),
            "no shadow account"
        );

        let alice = repo.create_user("alice", "x", true).await.unwrap();
        repo.grant_user_role(alice.id, Role::Admin.as_str())
            .await
            .unwrap();
        let session = oidc_complete(
            &repo,
            &no_root_users(),
            &StubOidc,
            "good".to_string(),
            "st",
            flow(),
        )
        .await
        .unwrap();
        assert_eq!(session.user_id, alice.id);
    }

    #[tokio::test]
    async fn oidc_begin_without_config_is_a_bad_request() {
        let repo = MockRepo::new();
        repo.set_setting("auth_mode", "oidc").await.unwrap();
        assert!(matches!(
            oidc_begin(&repo, &StubOidc).await,
            Err(AppError::BadRequest(_))
        ));
    }

    #[tokio::test]
    async fn the_account_kind_is_the_last_sign_in_and_service_accounts_are_known_at_once() {
        let repo = MockRepo::new();
        let alice = repo.create_user("alice", "hash", true).await.unwrap();
        let bob = repo.create_user("bob", "hash", true).await.unwrap();
        let jenkins = repo
            .create_user("jenkins", SERVICE_ACCOUNT_PASSWORD_HASH, true)
            .await
            .unwrap();

        let session = repo.create_session(alice.id, "2099-01-01").await.unwrap();
        record_sign_in(&repo, &session, SignInMethod::Ldap)
            .await
            .unwrap();
        let users = vec![alice.clone(), bob.clone(), jenkins.clone()];
        let kinds = account_kinds(&repo, &users).await.unwrap();
        assert_eq!(kinds[&alice.id], "ldap");
        assert_eq!(kinds[&bob.id], "unknown");
        assert_eq!(kinds[&jenkins.id], "service");
        assert_eq!(
            repo.session_sign_in(&session.token).await.unwrap(),
            Some(SignInMethod::Ldap)
        );

        // The tag follows the latest sign-in.
        let session = repo.create_session(alice.id, "2099-01-01").await.unwrap();
        record_sign_in(&repo, &session, SignInMethod::Local)
            .await
            .unwrap();
        assert_eq!(
            account_kinds(&repo, &users).await.unwrap()[&alice.id],
            "local"
        );
    }

    #[test]
    fn the_gravatar_key_is_the_sha256_of_the_normalised_address() {
        // Gravatar's own documented example.
        assert_eq!(
            gravatar_key(" MyEmailAddress@example.com "),
            "84059b07d4be67b806386c0aad8070a23f18836bbaae342275dc0a83414c32ee"
        );
    }

    #[tokio::test]
    async fn gravatar_keys_are_only_handed_out_while_gravatar_is_on() {
        let repo = MockRepo::new();
        let alice = repo.create_user("alice", "hash", true).await.unwrap();
        refresh_provider_identity(&repo, alice.id, None, Some("alice@example.com"))
            .await
            .unwrap();
        assert!(
            !get_gravatar_enabled(&repo).await.unwrap(),
            "off by default"
        );
        assert!(gravatar_keys(&repo).await.unwrap().is_empty());
        set_gravatar_enabled(&repo, true).await.unwrap();
        assert_eq!(
            gravatar_keys(&repo).await.unwrap().get(&alice.id),
            Some(&gravatar_key("alice@example.com"))
        );
    }

    #[tokio::test]
    async fn what_the_provider_no_longer_says_is_cleared_at_the_next_sign_in() {
        let repo = MockRepo::new();
        let alice = repo.create_user("alice", "hash", true).await.unwrap();
        refresh_provider_identity(
            &repo,
            alice.id,
            Some(" Alice Liddell "),
            Some("alice@example.com"),
        )
        .await
        .unwrap();
        let profile = user_profile(&repo, alice.id).await.unwrap();
        assert_eq!(profile.display_name.as_deref(), Some("Alice Liddell"));
        assert_eq!(profile.avatar_hash, Some(gravatar_key("alice@example.com")));

        refresh_provider_identity(&repo, alice.id, Some("  "), None)
            .await
            .unwrap();
        let profile = user_profile(&repo, alice.id).await.unwrap();
        assert_eq!(profile.display_name, None);
        assert_eq!(profile.avatar_hash, None);
    }

    #[tokio::test]
    async fn local_users_name_themselves_and_admins_name_service_accounts() {
        let repo = MockRepo::new();
        let alice = repo.create_user("alice", "hash", true).await.unwrap();
        let jenkins = repo
            .create_user("jenkins", SERVICE_ACCOUNT_PASSWORD_HASH, true)
            .await
            .unwrap();
        assert_eq!(
            set_own_display_name(&repo, &alice, "  Alice Liddell ")
                .await
                .unwrap()
                .as_deref(),
            Some("Alice Liddell")
        );
        assert_eq!(
            set_own_display_name(&repo, &alice, " ").await.unwrap(),
            None,
            "blank clears it"
        );
        assert!(matches!(
            set_own_display_name(&repo, &alice, &"x".repeat(MAX_DISPLAY_NAME_CHARS + 1)).await,
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            set_service_account_display_name(&repo, alice.id, "Alice").await,
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            set_service_account_display_name(&repo, 9999, "Ghost").await,
            Err(AppError::NotFound(_))
        ));
        set_service_account_display_name(&repo, jenkins.id, "Penguin CI build")
            .await
            .unwrap();
        assert_eq!(
            display_names(&repo)
                .await
                .unwrap()
                .get(&jenkins.id)
                .map(String::as_str),
            Some("Penguin CI build")
        );

        let entry = |username: &str| AuditLogEntry {
            id: 0,
            timestamp: String::new(),
            username: username.to_string(),
            action: "X".to_string(),
            details: String::new(),
            service: None,
            version: None,
            action_type: None,
            diff: None,
            stream: None,
        };
        let views = with_display_names(&repo, vec![entry("jenkins"), entry("gone")])
            .await
            .unwrap();
        assert_eq!(views[0].display_name.as_deref(), Some("Penguin CI build"));
        assert_eq!(views[1].display_name, None);
    }

    #[tokio::test]
    async fn oidc_complete_mints_a_session_for_the_verified_identity() {
        let repo = repo_with_oidc_config().await;
        let flow = OidcFlowState {
            state: "st".to_string(),
            nonce: "no".to_string(),
            pkce_verifier: "pk".to_string(),
        };
        let session = oidc_complete(
            &repo,
            &no_root_users(),
            &StubOidc,
            "good".to_string(),
            "st",
            flow,
        )
        .await
        .unwrap();
        assert!(!session.token.is_empty());
        let alice = repo.find_user("alice").await.unwrap().unwrap();
        assert_eq!(
            user_profile(&repo, alice.id)
                .await
                .unwrap()
                .display_name
                .as_deref(),
            Some("Alice Liddell"),
            "the provider's name claim"
        );
        assert_eq!(
            repo.session_sign_in(&session.token).await.unwrap(),
            Some(SignInMethod::Oidc)
        );
    }

    #[tokio::test]
    async fn oidc_complete_refuses_a_state_mismatch() {
        let repo = repo_with_oidc_config().await;
        let flow = OidcFlowState {
            state: "st".to_string(),
            nonce: "no".to_string(),
            pkce_verifier: "pk".to_string(),
        };
        let result = oidc_complete(
            &repo,
            &no_root_users(),
            &StubOidc,
            "good".to_string(),
            "forged",
            flow,
        )
        .await;
        assert!(matches!(result, Err(AppError::Unauthorized)));
        assert!(repo.find_user("alice").await.unwrap().is_none());
    }

    #[test]
    fn test_hash_and_verify_password() {
        let hash = hash_password("secret123").unwrap();
        assert!(verify_password("secret123", &hash).unwrap());
        assert!(!verify_password("wrong", &hash).unwrap());
    }

    // The blocking-pool wrappers must behave exactly like the sync functions:
    // same acceptance, same rejection, hashes interchangeable between the two.
    #[tokio::test]
    async fn test_hash_and_verify_password_async() {
        let hash = hash_password_async("secret123").await.unwrap();
        assert!(verify_password_async("secret123", &hash).await.unwrap());
        assert!(!verify_password_async("wrong", &hash).await.unwrap());
        assert!(verify_password("secret123", &hash).unwrap());
        let sync_hash = hash_password("secret123").unwrap();
        assert!(
            verify_password_async("secret123", &sync_hash)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn test_ensure_initial_admin_creates_user() {
        let repo = MockRepo::new();
        ensure_initial_admin(&repo, "http://127.0.0.1:3000/admin.html")
            .await
            .unwrap();
        let users = repo.list_users().await.unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].username, "root");
        let roles = repo
            .list_user_roles(users[0].id)
            .await
            .expect("roles readable");
        assert_eq!(
            roles,
            vec!["admin".to_string()],
            "the bootstrap account's authority is an explicit grant now"
        );
    }

    #[tokio::test]
    async fn test_ensure_initial_admin_skips_when_users_exist() {
        let repo = MockRepo::new();
        repo.create_user("existing", "hash", true).await.unwrap();
        ensure_initial_admin(&repo, "http://127.0.0.1:3000/admin.html")
            .await
            .unwrap();
        let users = repo.list_users().await.unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].username, "existing");
    }

    #[tokio::test]
    async fn test_login_success() {
        let repo = MockRepo::new();
        let hash = hash_password("pass").unwrap();
        repo.create_user("alice", &hash, true).await.unwrap();
        let (session, user) = login(&repo, "alice", "pass").await.unwrap();
        assert!(!session.token.is_empty());
        assert_eq!(user.username, "alice");
    }

    #[tokio::test]
    async fn test_login_wrong_password() {
        let repo = MockRepo::new();
        let hash = hash_password("pass").unwrap();
        repo.create_user("alice", &hash, true).await.unwrap();
        let err = login(&repo, "alice", "wrong").await.unwrap_err();
        assert!(matches!(err, AppError::Unauthorized));
    }

    #[tokio::test]
    async fn test_login_unapproved_user() {
        let repo = MockRepo::new();
        let hash = hash_password("pass").unwrap();
        repo.create_user("bob", &hash, false).await.unwrap();
        let err = login(&repo, "bob", "pass").await.unwrap_err();
        assert!(matches!(err, AppError::Forbidden));
    }

    #[tokio::test]
    async fn test_register_user_success() {
        let repo = MockRepo::new();
        repo.set_setting("auth_mode", "local").await.unwrap();
        repo.set_setting("registration_enabled", "true")
            .await
            .unwrap();
        register_user(&repo, &no_root_users(), "newuser", "pass123")
            .await
            .unwrap();
        let user = repo.find_user("newuser").await.unwrap().unwrap();
        assert_eq!(user.username, "newuser");
    }

    /// A root username holds every permission by pure name match, so an
    /// unclaimed one must never be claimable through open registration.
    #[tokio::test]
    async fn register_rejects_a_reserved_root_username() {
        let repo = MockRepo::new();
        repo.set_setting("auth_mode", "local").await.unwrap();
        repo.set_setting("registration_enabled", "true")
            .await
            .unwrap();
        let root_users = crate::domain::permissions::RootUsers::resolve(Some("ops-root"), None);
        let err = register_user(&repo, &root_users, "ops-root", "pass123")
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)));
        assert!(repo.find_user("ops-root").await.unwrap().is_none());
    }

    /// Startup claims unclaimed root names with locked accounts, and leaves
    /// accounts that already exist alone.
    #[tokio::test]
    async fn claim_root_usernames_creates_locked_accounts_once() {
        let repo = MockRepo::new();
        let existing_hash = hash_password("kept").unwrap();
        repo.create_user("alice", &existing_hash, true)
            .await
            .unwrap();
        let root_users =
            crate::domain::permissions::RootUsers::resolve(Some("alice, ops-root"), None);

        claim_root_usernames(&repo, &root_users).await.unwrap();

        let claimed = repo.find_user("ops-root").await.unwrap().unwrap();
        assert!(claimed.approved);
        let alice = repo.find_user("alice").await.unwrap().unwrap();
        assert_eq!(
            alice.password_hash, existing_hash,
            "an already-claimed name must not be touched"
        );

        // Idempotent across restarts: the claimed account is not recreated.
        let claimed_hash = claimed.password_hash.clone();
        claim_root_usernames(&repo, &root_users).await.unwrap();
        let claimed_again = repo.find_user("ops-root").await.unwrap().unwrap();
        assert_eq!(claimed_again.password_hash, claimed_hash);
    }

    #[tokio::test]
    async fn test_register_user_duplicate() {
        let repo = MockRepo::new();
        repo.set_setting("auth_mode", "local").await.unwrap();
        repo.set_setting("registration_enabled", "true")
            .await
            .unwrap();
        register_user(&repo, &no_root_users(), "dup", "pass")
            .await
            .unwrap();
        let err = register_user(&repo, &no_root_users(), "dup", "pass")
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn test_change_password() {
        let repo = MockRepo::new();
        let hash = hash_password("old").unwrap();
        let user = repo.create_user("u", &hash, true).await.unwrap();
        change_password(&repo, &user, None, "old", "new")
            .await
            .unwrap();
        login(&repo, "u", "new").await.unwrap();
    }
    /// A directory that accepts every bind must still not let an empty
    /// password through the provider path.
    struct AcceptingProvider;

    impl AuthProvider for AcceptingProvider {
        async fn authenticate(
            &self,
            username: &str,
            _password: &str,
        ) -> Result<AuthenticatedUser, crate::domain::ports::AuthProviderError> {
            Ok(AuthenticatedUser {
                username: username.to_string(),
                ..Default::default()
            })
        }

        async fn test_connection(&self) -> Result<(), crate::domain::ports::AuthProviderError> {
            Ok(())
        }
    }

    /// What the claim log line promises: the first directory login under a
    /// claimed root name signs in as the placeholder row, which becomes that
    /// user's shadow account; the random local password stays unusable.
    #[tokio::test]
    async fn directory_login_signs_in_as_the_claimed_root_placeholder() {
        let repo = MockRepo::new();
        let root_users = crate::domain::permissions::RootUsers::resolve(Some("ops-root"), None);
        claim_root_usernames(&repo, &root_users).await.unwrap();
        let placeholder = repo.find_user("ops-root").await.unwrap().unwrap();

        let session = login_with_provider(&repo, &AcceptingProvider, "ops-root", "dir-pass")
            .await
            .unwrap();

        assert_eq!(session.user_id, placeholder.id);
        assert_eq!(repo.user_count().await.unwrap(), 1, "no second account");
        let err = login(&repo, "ops-root", "dir-pass").await.unwrap_err();
        assert!(
            matches!(err, AppError::Unauthorized),
            "the directory password is not a local one"
        );
    }
    #[test]
    fn initial_admin_url_prefers_the_public_base_url() {
        assert_eq!(
            initial_admin_url(Some("https://sanshain.corp"), "0.0.0.0:3000"),
            "https://sanshain.corp/admin.html"
        );
        assert_eq!(
            initial_admin_url(Some("https://sanshain.corp/"), "0.0.0.0:3000"),
            "https://sanshain.corp/admin.html"
        );
        assert_eq!(
            initial_admin_url(None, "127.0.0.1:3000"),
            "http://127.0.0.1:3000/admin.html"
        );
    }

    #[tokio::test]
    async fn dev_user_never_signs_in_and_an_old_one_is_locked() {
        let repo = MockRepo::new();
        let dev = ensure_dev_user(&repo).await.unwrap();
        assert!(dev.is_service_account());
        assert!(matches!(
            login(&repo, "dev_user", "dev_user_internal").await,
            Err(AppError::Unauthorized)
        ));

        // One created before 2.10, with the password from the source.
        let repo = MockRepo::new();
        let old = repo
            .create_user(
                "dev_user",
                &hash_password("dev_user_internal").unwrap(),
                true,
            )
            .await
            .unwrap();
        assert!(login(&repo, "dev_user", "dev_user_internal").await.is_ok());
        assert!(lock_dev_user(&repo).await.unwrap());
        assert!(!lock_dev_user(&repo).await.unwrap(), "only once");
        assert!(matches!(
            login(&repo, "dev_user", "dev_user_internal").await,
            Err(AppError::Unauthorized)
        ));
        assert_eq!(
            ensure_dev_user(&repo).await.unwrap().id,
            old.id,
            "same account"
        );
    }

    #[test]
    fn initial_auth_mode_defaults_to_off_and_gates_dev() {
        assert_eq!(initial_auth_mode(None, false), Ok(AuthMode::Disabled));
        assert_eq!(initial_auth_mode(Some("  "), false), Ok(AuthMode::Disabled));
        assert_eq!(initial_auth_mode(Some("local"), false), Ok(AuthMode::Local));
        assert_eq!(initial_auth_mode(Some("ldap"), false), Ok(AuthMode::Ldap));
        assert_eq!(initial_auth_mode(Some("dev"), true), Ok(AuthMode::Dev));
        assert!(
            initial_auth_mode(Some("dev"), false)
                .unwrap_err()
                .contains("ALLOW_INSECURE_DEV_MODE")
        );
        assert!(
            initial_auth_mode(Some("kerberos"), true)
                .unwrap_err()
                .contains("kerberos")
        );
    }

    #[test]
    fn initial_admin_password_treats_empty_as_unset() {
        assert_eq!(
            initial_admin_password(Some("configured".to_string())),
            "configured"
        );
        for unset in [None, Some(String::new()), Some("   ".to_string())] {
            let password = initial_admin_password(unset);
            assert_eq!(password.len(), 16, "a random password, not the empty one");
            assert!(password.chars().all(|c| c.is_ascii_alphanumeric()));
        }
    }

    /// Defect 1 of 2.6.1: an account created with the hash of "" (the shipped
    /// Helm and Kustomize Secrets rendered an empty INITIAL_ADMIN_PASSWORD)
    /// must not be enterable with an empty password.
    #[tokio::test]
    async fn login_refuses_an_empty_password() {
        let repo = MockRepo::new();
        let hash = hash_password("").unwrap();
        repo.create_user("root", &hash, true).await.unwrap();
        let err = login(&repo, "root", "").await.unwrap_err();
        assert!(matches!(err, AppError::Unauthorized));
    }

    #[tokio::test]
    async fn directory_login_refuses_an_empty_password() {
        let repo = MockRepo::new();
        let err = login_with_provider(&repo, &AcceptingProvider, "alice", "")
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Unauthorized));
        assert!(
            repo.find_user("alice").await.unwrap().is_none(),
            "no shadow account for a refused login"
        );
    }
}
