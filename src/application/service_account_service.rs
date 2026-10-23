//! Service accounts: accounts an administrator creates for automation, such
//! as a `jenkins` that provides from CI. They have no password and never sign
//! in; holders of `manage_users` issue their API tokens, see every one of
//! them on the token page with who issued it, and may revoke any of them.

use super::auth_service::{create_api_token_issued, list_api_tokens};
use crate::domain::models::{
    ApiToken, ApiTokenScope, AppError, SERVICE_ACCOUNT_PASSWORD_HASH, User,
};
use crate::domain::permissions::{Actor, Permission, RootUsers};
use crate::domain::ports::SpecRepository;
use tracing::instrument;

/// Create the service account `username`.
#[instrument(skip_all)]
pub async fn create_service_account(
    repo: &impl SpecRepository,
    root_users: &RootUsers,
    username: &str,
) -> Result<User, AppError> {
    let username = username.trim();
    if username.is_empty() || username.chars().any(char::is_whitespace) {
        return Err(AppError::BadRequest(
            "a service account needs a name without spaces".to_string(),
        ));
    }
    if root_users.contains(username) {
        return Err(AppError::Conflict("This username is reserved".to_string()));
    }
    if repo.find_user(username).await?.is_some() {
        return Err(AppError::Conflict("User already exists".to_string()));
    }
    Ok(repo
        .create_user(username, SERVICE_ACCOUNT_PASSWORD_HASH, true)
        .await?)
}

async fn service_accounts(repo: &impl SpecRepository) -> Result<Vec<User>, AppError> {
    Ok(repo
        .list_users()
        .await?
        .into_iter()
        .filter(User::is_service_account)
        .collect())
}

/// A token on the token page: the caller's own, or one of a service account.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ListedToken {
    #[serde(flatten)]
    pub token: ApiToken,
    /// The service account the token belongs to; none for the caller's own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub for_user: Option<String>,
}

/// The caller's tokens, and for holders of `manage_users` every service
/// account's tokens too.
pub async fn list_tokens_for(
    repo: &impl SpecRepository,
    actor: &Actor,
) -> Result<Vec<ListedToken>, AppError> {
    let mut listed: Vec<ListedToken> = list_api_tokens(repo, actor.user_id)
        .await?
        .into_iter()
        .map(|token| ListedToken {
            token,
            for_user: None,
        })
        .collect();
    if actor.has_permission(Permission::ManageUsers) {
        for account in service_accounts(repo).await? {
            listed.extend(
                list_api_tokens(repo, account.id)
                    .await?
                    .into_iter()
                    .map(|token| ListedToken {
                        token,
                        for_user: Some(account.username.clone()),
                    }),
            );
        }
    }
    Ok(listed)
}

/// What a new token is for.
pub struct TokenRequest<'a> {
    pub name: &'a str,
    pub expires_in_days: u64,
    pub scope: ApiTokenScope,
    /// A service account to issue it for; the caller's own when none.
    pub for_user: Option<&'a str>,
}

/// Create a token for the caller, or issue one for a service account.
#[instrument(skip_all)]
pub async fn issue_token(
    repo: &impl SpecRepository,
    actor: &Actor,
    request: TokenRequest<'_>,
) -> Result<(String, String), AppError> {
    let Some(for_user) = request.for_user.filter(|u| !u.is_empty()) else {
        return create_api_token_issued(
            repo,
            actor.user_id,
            request.name,
            request.expires_in_days,
            request.scope,
            None,
        )
        .await;
    };
    if !actor.has_permission(Permission::ManageUsers) {
        return Err(AppError::ForbiddenWithReason(
            "issuing a token for a service account needs manage_users".to_string(),
        ));
    }
    let account = repo
        .find_user(for_user)
        .await?
        .filter(User::is_service_account)
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "'{for_user}' is not a service account; tokens are issued only for those"
            ))
        })?;
    create_api_token_issued(
        repo,
        account.id,
        request.name,
        request.expires_in_days,
        request.scope,
        Some(&actor.username),
    )
    .await
}

/// Revoke a token of the caller's, or, for holders of `manage_users`, of any
/// service account. Answers whether one went.
pub async fn revoke_token_as(
    repo: &impl SpecRepository,
    actor: &Actor,
    token_id: &str,
) -> Result<bool, AppError> {
    if repo.delete_api_token(token_id, actor.user_id).await? {
        return Ok(true);
    }
    if actor.has_permission(Permission::ManageUsers) {
        for account in service_accounts(repo).await? {
            if repo.delete_api_token(token_id, account.id).await? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::mock_repo::MockRepo;
    use crate::application::sign_in_service::verify_password_local_first;
    use crate::domain::permissions::Role;

    fn actor(user: &User, roles: Vec<Role>) -> Actor {
        Actor {
            user_id: user.id,
            username: user.username.clone(),
            is_root: false,
            roles,
            directory_groups: Vec::new(),
        }
    }

    struct AcceptAll;

    impl crate::domain::ports::AuthProvider for AcceptAll {
        async fn authenticate(
            &self,
            username: &str,
            _password: &str,
        ) -> Result<crate::domain::models::AuthenticatedUser, crate::domain::ports::AuthProviderError>
        {
            Ok(crate::domain::models::AuthenticatedUser {
                username: username.to_string(),
                ..Default::default()
            })
        }

        async fn test_connection(&self) -> Result<(), crate::domain::ports::AuthProviderError> {
            Ok(())
        }
    }

    fn no_roots() -> RootUsers {
        RootUsers::resolve(Some("ops-root"), None)
    }

    fn request<'a>(name: &'a str, for_user: Option<&'a str>) -> TokenRequest<'a> {
        TokenRequest {
            name,
            expires_in_days: 30,
            scope: ApiTokenScope::Full,
            for_user,
        }
    }

    #[tokio::test]
    async fn a_service_account_is_created_ready_and_never_signs_in() {
        let repo = MockRepo::new();
        let jenkins = create_service_account(&repo, &no_roots(), " jenkins ")
            .await
            .unwrap();
        assert_eq!(jenkins.username, "jenkins");
        assert!(jenkins.approved && jenkins.is_service_account());

        for password in ["", "!service-account", "anything"] {
            // A directory that accepts everything still does not let it in.
            let err = verify_password_local_first(&repo, Some(&AcceptAll), "jenkins", password)
                .await
                .unwrap_err();
            assert!(matches!(err, AppError::Unauthorized), "{password:?}");
        }
        assert!(matches!(
            super::super::auth_service::login(&repo, "jenkins", "!service-account").await,
            Err(AppError::Unauthorized)
        ));
    }

    #[tokio::test]
    async fn names_are_checked_like_registration() {
        let repo = MockRepo::new();
        repo.create_user("taken", "x", true).await.unwrap();
        for (name, conflict) in [
            ("taken", true),
            ("ops-root", true),
            ("", false),
            ("two words", false),
        ] {
            let err = create_service_account(&repo, &no_roots(), name)
                .await
                .unwrap_err();
            assert_eq!(
                matches!(err, AppError::Conflict(_)),
                conflict,
                "{name:?}: {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn user_managers_issue_see_and_revoke_service_account_tokens() {
        let repo = MockRepo::new();
        let jenkins = create_service_account(&repo, &no_roots(), "jenkins")
            .await
            .unwrap();
        let alice = repo.create_user("alice", "x", true).await.unwrap();
        let bob = repo.create_user("bob", "x", true).await.unwrap();
        let carol = repo.create_user("carol", "x", true).await.unwrap();
        let alice = actor(&alice, vec![Role::UserManager]);
        let bob = actor(&bob, vec![Role::UserManager]);
        let carol = actor(&carol, Vec::new());

        let (id, raw) = issue_token(&repo, &alice, request("ci", Some("jenkins")))
            .await
            .unwrap();
        assert!(raw.starts_with("san_"));
        let tokens = list_api_tokens(&repo, jenkins.id).await.unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].issued_by.as_deref(), Some("alice"));

        // Every user manager sees it, marked; a plain user does not.
        let seen = list_tokens_for(&repo, &bob).await.unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].for_user.as_deref(), Some("jenkins"));
        assert!(list_tokens_for(&repo, &carol).await.unwrap().is_empty());

        // A plain user can neither issue nor revoke one.
        let err = issue_token(&repo, &carol, request("x", Some("jenkins")))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::ForbiddenWithReason(_)));
        assert!(!revoke_token_as(&repo, &carol, &id).await.unwrap());

        // Bob revokes the token Alice issued.
        assert!(revoke_token_as(&repo, &bob, &id).await.unwrap());
        assert!(list_api_tokens(&repo, jenkins.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn tokens_are_issued_only_for_service_accounts() {
        let repo = MockRepo::new();
        let alice = repo.create_user("alice", "x", true).await.unwrap();
        repo.create_user("dave", "x", true).await.unwrap();
        let alice = actor(&alice, vec![Role::Admin]);
        for target in ["dave", "nobody"] {
            let err = issue_token(&repo, &alice, request("t", Some(target)))
                .await
                .unwrap_err();
            assert!(matches!(err, AppError::BadRequest(_)), "{target}");
        }
        // No target: the caller's own token, as before.
        issue_token(&repo, &alice, request("mine", None))
            .await
            .unwrap();
        let mine = list_tokens_for(&repo, &alice).await.unwrap();
        assert_eq!(mine.len(), 1);
        assert!(mine[0].for_user.is_none() && mine[0].token.issued_by.is_none());
    }
}
