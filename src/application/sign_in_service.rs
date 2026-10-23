//! Who may sign in, and how, under each auth mode.
//!
//! The mode is one of Off, Dev, Local, LDAP and OIDC. Off is maintenance: the
//! API answers 503 and only holders of `manage_auth_config` sign in, through
//! the source that was active before Off (remembered in
//! `auth_mode_before_off`) or a local password. Dev drops the token
//! requirement on the API and, like Off, admits only those holders to the web
//! UI. Local alone offers registration, behind its own switch. Under LDAP a
//! local password is checked first, so it never reaches the directory, where
//! a failed bind counts towards the account lockout.

use super::auth_service::{
    generate_random_password, get_auth_mode, get_auto_approve_users, get_oidc_config,
    hash_password_async, verify_password_async,
};
use crate::domain::models::{AppError, AuthMode, AuthenticatedUser, Session, SignInMethod, User};
use crate::domain::permissions::{Actor, Permission, RootUsers};
use crate::domain::ports::{AuthProvider, SpecRepository};
use tracing::instrument;

const BEFORE_OFF_KEY: &str = "auth_mode_before_off";
const REGISTRATION_KEY: &str = "registration_enabled";

/// Whether only holders of `manage_auth_config` may sign in: in Off
/// (maintenance) and in Dev.
pub fn admins_only(mode: &AuthMode) -> bool {
    matches!(mode, AuthMode::Disabled | AuthMode::Dev)
}

/// The refusal for everyone else while [`admins_only`] holds.
pub fn admins_only_refusal(mode: &AuthMode) -> AppError {
    AppError::ForbiddenWithReason(if *mode == AuthMode::Disabled {
        "Sanshain is in maintenance; only administrators can sign in".to_string()
    } else {
        "Sanshain is in dev mode; only administrators can sign in".to_string()
    })
}

/// Where passwords and identities are checked: the mode itself, or for Off
/// the source that was active before it. Dev, and an Off nothing preceded,
/// check local passwords only.
pub async fn sign_in_source(repo: &impl SpecRepository) -> Result<AuthMode, AppError> {
    let mode = get_auth_mode(repo).await?;
    let source = if mode == AuthMode::Disabled {
        repo.get_setting(BEFORE_OFF_KEY)
            .await?
            .and_then(|v| v.parse().ok())
            .unwrap_or(AuthMode::Local)
    } else {
        mode
    };
    Ok(match source {
        AuthMode::Ldap | AuthMode::Oidc => source,
        _ => AuthMode::Local,
    })
}

/// Switch the auth mode. Entering Off remembers the mode it left, so the
/// administrators of a directory keep signing in through it during
/// maintenance.
pub async fn switch_auth_mode(repo: &impl SpecRepository, mode: AuthMode) -> Result<(), AppError> {
    let current = get_auth_mode(repo).await?;
    if mode == AuthMode::Dev && !super::auth_service::dev_mode_gate_open() {
        return Err(AppError::BadRequest(
            "dev mode needs the server started with ALLOW_INSECURE_DEV_MODE=true".to_string(),
        ));
    }
    if mode == AuthMode::Disabled && current != AuthMode::Disabled {
        repo.set_setting(BEFORE_OFF_KEY, current.as_str()).await?;
    }
    super::auth_service::set_auth_mode(repo, &mode).await
}

/// Switch a stored Dev mode to Local when the server runs without the
/// `ALLOW_INSECURE_DEV_MODE` gate. Before 2.10 Dev could be picked without the
/// gate and then did nothing, so such an instance has worked as Local; the
/// setting now says so. Answers whether it switched.
pub async fn settle_ungated_dev_mode(repo: &impl SpecRepository) -> Result<bool, AppError> {
    if get_auth_mode(repo).await? != AuthMode::Dev || super::auth_service::dev_mode_gate_open() {
        return Ok(false);
    }
    super::auth_service::set_auth_mode(repo, &AuthMode::Local).await?;
    Ok(true)
}

pub async fn get_registration_enabled(repo: &impl SpecRepository) -> Result<bool, AppError> {
    Ok(repo.get_setting(REGISTRATION_KEY).await?.as_deref() == Some("true"))
}

pub async fn set_registration_enabled(
    repo: &impl SpecRepository,
    enabled: bool,
) -> Result<(), AppError> {
    repo.set_setting(REGISTRATION_KEY, if enabled { "true" } else { "false" })
        .await?;
    Ok(())
}

/// Whether anyone may register now: Local mode with the switch on.
pub async fn registration_open(repo: &impl SpecRepository) -> Result<bool, AppError> {
    Ok(get_auth_mode(repo).await? == AuthMode::Local && get_registration_enabled(repo).await?)
}

/// What the login page offers, readable without signing in.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LoginOptions {
    pub auth_mode: AuthMode,
    /// The register card is shown.
    pub registration: bool,
    /// A registered account can sign in at once.
    pub auto_approve: bool,
    /// The "Sign in with SSO" button is shown.
    pub sso: bool,
    /// Only administrators can sign in (Off, Dev).
    pub admins_only: bool,
}

pub async fn login_options(repo: &impl SpecRepository) -> Result<LoginOptions, AppError> {
    let auth_mode = get_auth_mode(repo).await?;
    Ok(LoginOptions {
        admins_only: admins_only(&auth_mode),
        auth_mode,
        registration: registration_open(repo).await?,
        auto_approve: get_auto_approve_users(repo).await?,
        sso: sign_in_source(repo).await? == AuthMode::Oidc
            && get_oidc_config(repo).await?.is_some(),
    })
}

/// A password check that succeeded.
#[derive(Debug)]
pub enum Verified {
    /// The local password matched this account.
    Local(User),
    /// The directory accepted the password; the account it signs in as, if
    /// one exists yet, and who the directory says the person is.
    Directory(Option<User>, AuthenticatedUser),
}

/// Check `password`: the local one first, then the directory when there is
/// one. A local password that matches never reaches the directory, where a
/// failed bind would count towards the account's lockout. A local account that
/// is refused (wrong password, not yet approved) still gets the directory's
/// answer; when both refuse, the local refusal stands, so "not approved"
/// survives.
#[instrument(skip_all)]
pub async fn verify_password_local_first(
    repo: &impl SpecRepository,
    directory: Option<&impl AuthProvider>,
    username: &str,
    password: &str,
) -> Result<Verified, AppError> {
    if password.is_empty() {
        return Err(AppError::Unauthorized);
    }
    let existing = repo.find_user(username).await?;
    if existing.as_ref().is_some_and(User::is_service_account) {
        // Not by password, not through the directory: its tokens are all it has.
        return Err(AppError::Unauthorized);
    }
    let local_refusal = match &existing {
        Some(user) if !verify_password_async(password, &user.password_hash).await? => {
            AppError::Unauthorized
        }
        Some(user) if !user.approved => AppError::Forbidden,
        Some(user) => return Ok(Verified::Local(user.clone())),
        None => AppError::Unauthorized,
    };
    match directory {
        Some(provider) => match provider.authenticate(username, password).await {
            Ok(identity) => Ok(Verified::Directory(existing, identity)),
            Err(_) => Err(local_refusal),
        },
        None => Err(local_refusal),
    }
}

/// Whether `username` may sign in while only administrators may: their
/// stored roles (when an account exists), the roles their directory groups
/// confer, and root by name.
pub async fn may_sign_in_as_admin(
    repo: &impl SpecRepository,
    root_users: &RootUsers,
    username: &str,
    existing: Option<&User>,
    directory_groups: &[String],
) -> Result<bool, AppError> {
    let actor = match existing {
        Some(user) => super::authz::resolve_actor(repo, user, root_users, directory_groups).await?,
        None => Actor {
            user_id: 0,
            username: username.to_string(),
            is_root: root_users.contains(username),
            roles: super::directory_roles::roles_from_directory(repo, directory_groups)
                .await?
                .iter()
                .filter_map(|r| crate::domain::permissions::Role::parse(r))
                .collect(),
            directory_groups: directory_groups.to_vec(),
        },
    };
    Ok(actor.has_permission(Permission::ManageAuthConfig))
}

/// Start the session for a verified sign-in, creating the shadow account of a
/// first directory login. Called only after any admins-only check, so a
/// refused sign-in leaves neither an account nor a session behind.
pub async fn start_session(
    repo: &impl SpecRepository,
    root_users: &RootUsers,
    verified: Verified,
) -> Result<Session, AppError> {
    let (user, method, identity) = match verified {
        Verified::Local(user) => (user, SignInMethod::Local, None),
        Verified::Directory(existing, identity) => (
            directory_account(repo, root_users, existing, &identity).await?,
            SignInMethod::Ldap,
            Some(identity),
        ),
    };
    if let Some(identity) = identity {
        super::auth_service::refresh_provider_identity(
            repo,
            user.id,
            identity.display_name.as_deref(),
            identity.email.as_deref(),
        )
        .await?;
    }
    let expires_at = (chrono::Utc::now() + chrono::Duration::days(7)).to_rfc3339();
    let session = repo.create_session(user.id, &expires_at).await?;
    super::auth_service::record_sign_in(repo, &session, method).await?;
    Ok(session)
}

/// The one account of a directory entry. The directory says who the person
/// is: their username (the username attribute) and their immutable ID. Every
/// account that belongs to the entry (found by that ID, by the username, by
/// the name that was typed, or by the entry's e-mail address, the usual
/// alternative sign-in name) is folded into one, which takes the username
/// and the ID. A rename in the directory renames the account. Root names are
/// held by configuration: such an account is neither renamed nor merged.
async fn directory_account(
    repo: &impl SpecRepository,
    root_users: &RootUsers,
    typed: Option<User>,
    identity: &AuthenticatedUser,
) -> Result<User, AppError> {
    let canonical = identity.username.as_str();
    let mut candidates: Vec<User> = Vec::new();
    let mut consider = |user: Option<User>| {
        if let Some(user) = user
            && !user.is_service_account()
            && !candidates.iter().any(|c| c.id == user.id)
        {
            candidates.push(user);
        }
    };
    if let Some(id) = identity.directory_id.as_deref() {
        consider(repo.find_user_by_directory_id(id).await?);
    }
    consider(repo.find_user(canonical).await?);
    consider(typed);
    if let Some(email) = identity.email.as_deref() {
        consider(repo.find_user(email.trim()).await?);
    }

    let Some(target) = candidates
        .iter()
        .find(|u| u.username == canonical)
        .or_else(|| candidates.first())
        .cloned()
    else {
        // A first sign-in: privileges come from the directory groups, resolved
        // on every check; the local password is random and unusable.
        let hash = hash_password_async(&generate_random_password()).await?;
        let user = repo.create_user(canonical, &hash, true).await?;
        if let Some(id) = identity.directory_id.as_deref() {
            repo.set_directory_id(user.id, id).await?;
        }
        return Ok(user);
    };

    for other in candidates.iter().filter(|u| u.id != target.id) {
        if root_users.contains(&other.username) || root_users.contains(&target.username) {
            continue;
        }
        repo.merge_user_into(other.id, target.id).await?;
        audit(
            repo,
            &target.username,
            "MERGE_USER",
            &format!(
                "Merged '{}' into '{}': both are the directory entry of '{canonical}'",
                other.username, target.username
            ),
        )
        .await?;
    }

    let mut target = target;
    let name_taken = repo
        .find_user(canonical)
        .await?
        .is_some_and(|u| u.id != target.id);
    if target.username != canonical
        && !name_taken
        && !root_users.contains(&target.username)
        && !root_users.contains(canonical)
    {
        repo.rename_user(target.id, canonical).await?;
        audit(
            repo,
            canonical,
            "RENAME_USER",
            &format!(
                "Renamed '{}' to '{canonical}' as the directory names them",
                target.username
            ),
        )
        .await?;
        target.username = canonical.to_string();
    }

    if let Some(id) = identity.directory_id.as_deref()
        && repo
            .find_user_by_directory_id(id)
            .await?
            .is_none_or(|holder| holder.id == target.id)
    {
        repo.set_directory_id(target.id, id).await?;
    }
    Ok(target)
}

async fn audit(
    repo: &impl SpecRepository,
    username: &str,
    action: &str,
    details: &str,
) -> Result<(), AppError> {
    repo.insert_audit_log(
        username,
        crate::domain::ports::NewAuditLog {
            action,
            details,
            action_type: Some("ADMIN"),
            ..Default::default()
        },
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::auth_service::hash_password;
    use crate::application::mock_repo::MockRepo;
    use crate::domain::models::AuthenticatedUser;
    use crate::domain::ports::AuthProviderError;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A directory that accepts one password and counts every bind, so a test
    /// can see whether a password reached it at all.
    struct Directory {
        password: &'static str,
        binds: AtomicUsize,
    }

    impl Directory {
        fn accepting(password: &'static str) -> Self {
            Directory {
                password,
                binds: AtomicUsize::new(0),
            }
        }
        fn binds(&self) -> usize {
            self.binds.load(Ordering::SeqCst)
        }
    }

    fn roots() -> RootUsers {
        RootUsers::resolve(Some("root"), None)
    }

    fn entry(username: &str, id: &str, email: &str) -> AuthenticatedUser {
        AuthenticatedUser {
            username: username.to_string(),
            display_name: None,
            email: Some(email.to_string()),
            directory_id: Some(id.to_string()),
        }
    }

    #[tokio::test]
    async fn an_e_mail_sign_in_lands_on_the_account_of_the_short_name_and_folds_the_duplicate() {
        let repo = repo_with("axel", "x", true).await;
        let axel = repo.find_user("axel").await.unwrap().unwrap();
        // Before 2.11 the e-mail sign-in made a second account.
        let dup = repo
            .create_user("a.smart@alva.com", "hash", true)
            .await
            .unwrap();
        repo.grant_user_role(dup.id, "releaser").await.unwrap();

        let session = start_session(
            &repo,
            &roots(),
            Verified::Directory(
                Some(dup.clone()),
                entry("axel", "uuid:1", "a.smart@alva.com"),
            ),
        )
        .await
        .unwrap();
        assert_eq!(session.user_id, axel.id, "one account: the short name's");
        assert!(repo.find_user("a.smart@alva.com").await.unwrap().is_none());
        assert_eq!(
            repo.list_user_roles(axel.id).await.unwrap(),
            vec!["releaser".to_string()],
            "the duplicate's roles came along"
        );
        let logs = repo.audit_logs.lock().unwrap().clone();
        assert!(logs.iter().any(|l| l.action == "MERGE_USER"));

        // The short name later: same account, found by the directory ID.
        let session = start_session(
            &repo,
            &roots(),
            Verified::Directory(
                Some(axel.clone()),
                entry("axel", "uuid:1", "a.smart@alva.com"),
            ),
        )
        .await
        .unwrap();
        assert_eq!(session.user_id, axel.id);
    }

    #[tokio::test]
    async fn a_rename_in_the_directory_renames_the_account() {
        let repo = MockRepo::new();
        let first = start_session(
            &repo,
            &roots(),
            Verified::Directory(None, entry("axel", "uuid:7", "axel@alva.com")),
        )
        .await
        .unwrap();
        let session = start_session(
            &repo,
            &roots(),
            Verified::Directory(None, entry("axel.dumb", "uuid:7", "axel.dumb@alva.com")),
        )
        .await
        .unwrap();
        assert_eq!(
            session.user_id, first.user_id,
            "recognised by the directory ID"
        );
        assert!(repo.find_user("axel").await.unwrap().is_none());
        let renamed = repo.find_user("axel.dumb").await.unwrap().unwrap();
        assert_eq!(renamed.id, first.user_id);
        let logs = repo.audit_logs.lock().unwrap().clone();
        assert!(logs.iter().any(|l| l.action == "RENAME_USER"));
    }

    #[tokio::test]
    async fn a_root_name_is_neither_renamed_nor_merged() {
        let repo = repo_with("root", "x", true).await;
        let root = repo.find_user("root").await.unwrap().unwrap();
        let session = start_session(
            &repo,
            &roots(),
            Verified::Directory(Some(root.clone()), entry("rsmith", "uuid:9", "r@alva.com")),
        )
        .await
        .unwrap();
        assert_eq!(session.user_id, root.id);
        assert!(
            repo.find_user("root").await.unwrap().is_some(),
            "still root"
        );
        assert!(repo.find_user("rsmith").await.unwrap().is_none());
    }

    impl AuthProvider for Directory {
        async fn authenticate(
            &self,
            username: &str,
            password: &str,
        ) -> Result<AuthenticatedUser, AuthProviderError> {
            self.binds.fetch_add(1, Ordering::SeqCst);
            if password == self.password {
                Ok(AuthenticatedUser {
                    username: username.to_string(),
                    display_name: Some(format!("{username} from the directory")),
                    email: Some(format!(" {}@Example.com ", username.to_uppercase())),
                    directory_id: None,
                })
            } else {
                Err(AuthProviderError::InvalidCredentials)
            }
        }

        async fn test_connection(&self) -> Result<(), AuthProviderError> {
            Ok(())
        }
    }

    fn no_directory() -> Option<&'static Directory> {
        None
    }

    async fn repo_with(username: &str, password: &str, approved: bool) -> MockRepo {
        let repo = MockRepo::new();
        repo.create_user(username, &hash_password(password).unwrap(), approved)
            .await
            .unwrap();
        repo
    }

    #[tokio::test]
    async fn a_matching_local_password_never_reaches_the_directory() {
        let repo = repo_with("axel", "local-pass", true).await;
        let directory = Directory::accepting("ldap-pass");
        let verified = verify_password_local_first(&repo, Some(&directory), "axel", "local-pass")
            .await
            .unwrap();
        assert!(matches!(verified, Verified::Local(ref u) if u.username == "axel"));
        assert_eq!(
            directory.binds(),
            0,
            "no bind, so no failed bind towards a lockout"
        );
    }

    #[tokio::test]
    async fn the_directory_password_signs_in_as_the_same_account() {
        let repo = repo_with("axel", "local-pass", true).await;
        let directory = Directory::accepting("ldap-pass");
        let verified = verify_password_local_first(&repo, Some(&directory), "axel", "ldap-pass")
            .await
            .unwrap();
        assert!(matches!(verified, Verified::Directory(Some(ref u), _) if u.username == "axel"));
        assert_eq!(directory.binds(), 1);
        let session = start_session(&repo, &roots(), verified).await.unwrap();
        assert_eq!(
            repo.session_sign_in(&session.token).await.unwrap(),
            Some(SignInMethod::Ldap),
            "the same account, signed in through the directory"
        );

        let verified = verify_password_local_first(&repo, Some(&directory), "axel", "local-pass")
            .await
            .unwrap();
        let session = start_session(&repo, &roots(), verified).await.unwrap();
        assert_eq!(
            repo.session_sign_in(&session.token).await.unwrap(),
            Some(SignInMethod::Local)
        );
    }

    #[tokio::test]
    async fn a_directory_user_without_an_account_is_verified_but_not_created() {
        let repo = MockRepo::new();
        let directory = Directory::accepting("ldap-pass");
        let verified = verify_password_local_first(&repo, Some(&directory), "dana", "ldap-pass")
            .await
            .unwrap();
        assert!(matches!(verified, Verified::Directory(None, _)));
        assert!(repo.find_user("dana").await.unwrap().is_none());
        let session = start_session(&repo, &roots(), verified).await.unwrap();
        assert!(
            repo.find_user("dana").await.unwrap().is_some(),
            "created on the session"
        );
        assert_eq!(
            repo.session_sign_in(&session.token).await.unwrap(),
            Some(SignInMethod::Ldap)
        );
        let dana = repo.find_user("dana").await.unwrap().unwrap();
        let profile = super::super::auth_service::user_profile(&repo, dana.id)
            .await
            .unwrap();
        assert_eq!(
            profile.display_name.as_deref(),
            Some("dana from the directory")
        );
        assert!(profile.display_name_from_provider());
        assert_eq!(
            profile.avatar_hash,
            Some(super::super::auth_service::gravatar_key("dana@example.com")),
            "the directory's mail, trimmed and lowercased"
        );
        let err = super::super::auth_service::set_own_display_name(&repo, &dana, "Dana")
            .await
            .unwrap_err();
        assert!(
            matches!(err, AppError::BadRequest(_)),
            "the directory owns it"
        );
    }

    #[tokio::test]
    async fn an_unapproved_local_account_still_gets_the_directory_answer() {
        let repo = repo_with("newbie", "local-pass", false).await;
        let directory = Directory::accepting("ldap-pass");
        let verified = verify_password_local_first(&repo, Some(&directory), "newbie", "ldap-pass")
            .await
            .unwrap();
        assert!(matches!(verified, Verified::Directory(Some(_), _)));

        let err = verify_password_local_first(&repo, Some(&directory), "newbie", "local-pass")
            .await
            .unwrap_err();
        assert!(
            matches!(err, AppError::Forbidden),
            "not approved, said as such"
        );
        let err = verify_password_local_first(&repo, no_directory(), "newbie", "wrong")
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Unauthorized));
    }

    #[tokio::test]
    async fn an_empty_password_is_refused_before_anything_is_asked() {
        let repo = repo_with("axel", "local-pass", true).await;
        let directory = Directory::accepting("");
        let err = verify_password_local_first(&repo, Some(&directory), "axel", "")
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Unauthorized));
        assert_eq!(directory.binds(), 0);
    }

    #[tokio::test]
    async fn off_remembers_the_source_it_interrupted() {
        let repo = MockRepo::new();
        repo.set_setting("auth_mode", "disabled").await.unwrap();
        assert_eq!(
            sign_in_source(&repo).await.unwrap(),
            AuthMode::Local,
            "fresh Off"
        );

        repo.set_setting("auth_mode", "ldap").await.unwrap();
        switch_auth_mode(&repo, AuthMode::Disabled).await.unwrap();
        assert_eq!(get_auth_mode(&repo).await.unwrap(), AuthMode::Disabled);
        assert_eq!(sign_in_source(&repo).await.unwrap(), AuthMode::Ldap);

        // Switching to Off again keeps what Off interrupted.
        switch_auth_mode(&repo, AuthMode::Disabled).await.unwrap();
        assert_eq!(sign_in_source(&repo).await.unwrap(), AuthMode::Ldap);

        switch_auth_mode(&repo, AuthMode::Oidc).await.unwrap();
        assert_eq!(sign_in_source(&repo).await.unwrap(), AuthMode::Oidc);
    }

    #[tokio::test]
    async fn dev_mode_is_refused_while_its_gate_is_closed() {
        assert!(!super::super::auth_service::dev_mode_gate_open());
        let repo = MockRepo::new();
        repo.set_setting("auth_mode", "local").await.unwrap();
        let err = switch_auth_mode(&repo, AuthMode::Dev).await.unwrap_err();
        assert!(
            matches!(err, AppError::BadRequest(ref m) if m.contains("ALLOW_INSECURE_DEV_MODE"))
        );
        assert_eq!(get_auth_mode(&repo).await.unwrap(), AuthMode::Local);
    }

    #[tokio::test]
    async fn a_stored_dev_mode_without_its_gate_becomes_local() {
        assert!(!super::super::auth_service::dev_mode_gate_open());
        let repo = MockRepo::new();
        repo.set_setting("auth_mode", "dev").await.unwrap();
        assert!(settle_ungated_dev_mode(&repo).await.unwrap());
        assert_eq!(get_auth_mode(&repo).await.unwrap(), AuthMode::Local);
        assert!(!settle_ungated_dev_mode(&repo).await.unwrap(), "only once");

        repo.set_setting("auth_mode", "ldap").await.unwrap();
        assert!(!settle_ungated_dev_mode(&repo).await.unwrap());
        assert_eq!(get_auth_mode(&repo).await.unwrap(), AuthMode::Ldap);
    }

    #[tokio::test]
    async fn registration_is_open_only_in_local_mode_with_its_switch_on() {
        let repo = MockRepo::new();
        for (mode, enabled, open) in [
            ("local", false, false),
            ("local", true, true),
            ("ldap", true, false),
            ("disabled", true, false),
            ("oidc", true, false),
        ] {
            repo.set_setting("auth_mode", mode).await.unwrap();
            set_registration_enabled(&repo, enabled).await.unwrap();
            assert_eq!(
                registration_open(&repo).await.unwrap(),
                open,
                "{mode} {enabled}"
            );
        }
    }

    #[tokio::test]
    async fn login_options_tell_the_page_what_to_offer() {
        let repo = MockRepo::new();
        repo.set_setting("auth_mode", "local").await.unwrap();
        set_registration_enabled(&repo, true).await.unwrap();
        repo.set_setting("auto_approve_users", "true")
            .await
            .unwrap();
        let options = login_options(&repo).await.unwrap();
        assert!(options.registration && options.auto_approve);
        assert!(!options.sso && !options.admins_only);

        repo.set_setting(
            "oidc_config",
            r#"{"issuer_url":"https://idp.example","client_id":"c","redirect_url":"https://app.example/auth/oidc/callback"}"#,
        )
        .await
        .unwrap();
        assert!(
            !login_options(&repo).await.unwrap().sso,
            "configured, but not the source"
        );
        switch_auth_mode(&repo, AuthMode::Oidc).await.unwrap();
        assert!(login_options(&repo).await.unwrap().sso);
        switch_auth_mode(&repo, AuthMode::Disabled).await.unwrap();
        let options = login_options(&repo).await.unwrap();
        assert!(options.sso, "admins sign in through the remembered SSO");
        assert!(options.admins_only && !options.registration);
    }

    #[tokio::test]
    async fn only_holders_of_manage_auth_config_may_sign_in_as_admins() {
        let repo = repo_with("admin-alice", "pw", true).await;
        let alice = repo.find_user("admin-alice").await.unwrap().unwrap();
        repo.grant_user_role(alice.id, "admin").await.unwrap();
        let bob = repo.create_user("bob", "x", true).await.unwrap();
        let roots = RootUsers::resolve(Some("ops-root"), None);

        assert!(
            may_sign_in_as_admin(&repo, &roots, "admin-alice", Some(&alice), &[])
                .await
                .unwrap()
        );
        assert!(
            !may_sign_in_as_admin(&repo, &roots, "bob", Some(&bob), &[])
                .await
                .unwrap()
        );
        assert!(
            may_sign_in_as_admin(&repo, &roots, "ops-root", None, &[])
                .await
                .unwrap()
        );
        assert!(
            !may_sign_in_as_admin(&repo, &roots, "stranger", None, &[])
                .await
                .unwrap()
        );
    }
}
