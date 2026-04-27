use crate::domain::models::*;
use crate::domain::ports::{AuthProvider, SpecRepository};
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use rand::distr::{Alphanumeric, SampleString};
use sha2::{Digest, Sha256};
use uuid::Uuid;

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

pub fn generate_random_password() -> String {
    Alphanumeric.sample_string(&mut rand::rng(), 16)
}

pub async fn ensure_initial_admin(repo: &impl SpecRepository) -> Result<(), AppError> {
    let count = repo.user_count().await?;
    if count == 0 {
        let username =
            std::env::var("INITIAL_ADMIN_USERNAME").unwrap_or_else(|_| "root".to_string());
        let password =
            std::env::var("INITIAL_ADMIN_PASSWORD").unwrap_or_else(|_| generate_random_password());
        let hash = hash_password(&password)?;
        repo.create_user(&username, &hash, true, true).await?;
        eprintln!(
            "[INITIAL SETUP] Admin user created. Username: {}, Password: {}",
            username, password
        );
        eprintln!(
            "[INITIAL SETUP] Change this password immediately at http://localhost:3000/admin.html"
        );
    }
    Ok(())
}

pub async fn login(
    repo: &impl SpecRepository,
    username: &str,
    password: &str,
) -> Result<Session, AppError> {
    let user = repo
        .find_user(username)
        .await?
        .ok_or(AppError::Unauthorized)?;

    if !user.approved {
        return Err(AppError::Forbidden);
    }

    if !verify_password(password, &user.password_hash)? {
        return Err(AppError::Unauthorized);
    }

    let expires_at = (chrono::Utc::now() + chrono::Duration::days(7)).to_rfc3339();
    let session = repo.create_session(user.id, &expires_at).await?;
    Ok(session)
}

pub async fn change_password(
    repo: &impl SpecRepository,
    user: &User,
    current_token: Option<&str>,
    old_password: &str,
    new_password: &str,
) -> Result<Option<Session>, AppError> {
    if !verify_password(old_password, &user.password_hash)? {
        return Err(AppError::Unauthorized);
    }
    let new_hash = hash_password(new_password)?;
    repo.update_password(user.id, &new_hash).await?;

    if let Some(token) = current_token {
        repo.delete_session(token).await?;
        let expires_at = (chrono::Utc::now() + chrono::Duration::days(7)).to_rfc3339();
        let session = repo.create_session(user.id, &expires_at).await?;
        return Ok(Some(session));
    }

    Ok(None)
}

pub async fn get_dev_mode(repo: &impl SpecRepository) -> Result<bool, AppError> {
    let val = repo
        .get_setting("dev_mode")
        .await?
        .unwrap_or("false".to_string());
    Ok(val == "true")
}

pub async fn set_dev_mode(repo: &impl SpecRepository, enabled: bool) -> Result<(), AppError> {
    repo.set_setting("dev_mode", if enabled { "true" } else { "false" })
        .await?;
    Ok(())
}

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

pub async fn register_user(
    repo: &impl SpecRepository,
    username: &str,
    password: &str,
) -> Result<(), AppError> {
    if !get_local_users_enabled(repo).await? {
        return Err(AppError::Forbidden);
    }
    if repo.find_user(username).await?.is_some() {
        return Err(AppError::Conflict("User already exists".to_string()));
    }
    let hash = hash_password(password)?;
    let auto_approve = repo
        .get_setting("auto_approve_users")
        .await?
        .unwrap_or("false".to_string())
        == "true";
    repo.create_user(username, &hash, false, auto_approve)
        .await?;
    Ok(())
}

pub async fn list_users(repo: &impl SpecRepository) -> Result<Vec<User>, AppError> {
    Ok(repo.list_users().await?)
}

pub async fn approve_user(repo: &impl SpecRepository, user_id: i64) -> Result<bool, AppError> {
    Ok(repo.approve_user(user_id).await?)
}

pub async fn admin_delete_user(repo: &impl SpecRepository, user_id: i64) -> Result<bool, AppError> {
    Ok(repo.delete_user(user_id).await?)
}

pub async fn get_local_users_enabled(repo: &impl SpecRepository) -> Result<bool, AppError> {
    let val = repo
        .get_setting("local_users_enabled")
        .await?
        .unwrap_or("true".to_string());
    Ok(val == "true")
}

pub async fn set_local_users_enabled(
    repo: &impl SpecRepository,
    enabled: bool,
) -> Result<(), AppError> {
    repo.set_setting(
        "local_users_enabled",
        if enabled { "true" } else { "false" },
    )
    .await?;
    Ok(())
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
        .unwrap_or("dev".to_string());
    match val.as_str() {
        "ldap" => Ok(AuthMode::Ldap),
        "local" => Ok(AuthMode::Local),
        "dev" => Ok(AuthMode::Dev),
        _ => Ok(AuthMode::Dev),
    }
}

pub async fn set_auth_mode(repo: &impl SpecRepository, mode: &AuthMode) -> Result<(), AppError> {
    let val = match mode {
        AuthMode::Local => "local",
        AuthMode::Ldap => "ldap",
        AuthMode::Dev => "dev",
    };
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

pub async fn set_ldap_config(
    repo: &impl SpecRepository,
    config: &LdapConfig,
) -> Result<(), AppError> {
    let json = serde_json::to_string(config)
        .map_err(|e| AppError::Internal(format!("LDAP config serialize error: {}", e)))?;
    repo.set_setting("ldap_config", &json).await?;
    Ok(())
}

pub async fn test_ldap_connection(provider: &impl AuthProvider) -> Result<(), AppError> {
    provider
        .test_connection()
        .await
        .map_err(|e| AppError::BadRequest(format!("LDAP connection failed: {}", e)))
}

pub async fn login_with_provider(
    repo: &impl SpecRepository,
    provider: &impl AuthProvider,
    username: &str,
    password: &str,
) -> Result<Session, AppError> {
    let auth_user = provider
        .authenticate(username, password)
        .await
        .map_err(|_| AppError::Unauthorized)?;

    // Ensure user exists in local DB for session management
    let local_user = match repo.find_user(username).await? {
        Some(u) => u,
        None => {
            // Create a stub user with a random password that can't be used for local login easily
            let hash = hash_password(&generate_random_password())?;
            repo.create_user(username, &hash, auth_user.is_admin, true)
                .await?
        }
    };

    let expires_at = (chrono::Utc::now() + chrono::Duration::days(7)).to_rfc3339();
    let session = repo.create_session(local_user.id, &expires_at).await?;
    Ok(session)
}

pub async fn create_api_token(
    repo: &impl SpecRepository,
    user_id: i64,
    name: &str,
    expires_in_days: u64,
) -> Result<(String, String), AppError> {
    let raw_token = format!("san_{}", Uuid::new_v4().to_string().replace("-", ""));
    let mut hasher = Sha256::new();
    hasher.update(raw_token.as_bytes());
    let token_hash = hex::encode(hasher.finalize());

    let id = Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();
    let expires_at =
        (chrono::Utc::now() + chrono::Duration::days(expires_in_days as i64)).to_rfc3339();

    repo.create_api_token(&id, user_id, name, &token_hash, &created_at, &expires_at)
        .await?;
    Ok((id, raw_token))
}

pub async fn list_api_tokens(
    repo: &impl SpecRepository,
    user_id: i64,
) -> Result<Vec<ApiToken>, AppError> {
    Ok(repo.list_api_tokens(user_id).await?)
}

pub async fn revoke_api_token(
    repo: &impl SpecRepository,
    token_id: &str,
    user_id: i64,
) -> Result<bool, AppError> {
    Ok(repo.delete_api_token(token_id, user_id).await?)
}

pub async fn validate_api_token(
    repo: &impl SpecRepository,
    raw_token: &str,
) -> Result<Option<User>, AppError> {
    let mut hasher = Sha256::new();
    hasher.update(raw_token.as_bytes());
    let token_hash = hex::encode(hasher.finalize());

    Ok(repo.validate_api_token(&token_hash).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::mock_repo::MockRepo;

    #[test]
    fn test_hash_and_verify_password() {
        let hash = hash_password("secret123").unwrap();
        assert!(verify_password("secret123", &hash).unwrap());
        assert!(!verify_password("wrong", &hash).unwrap());
    }

    #[tokio::test]
    async fn test_ensure_initial_admin_creates_user() {
        let repo = MockRepo::new();
        ensure_initial_admin(&repo).await.unwrap();
        let users = repo.list_users().await.unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].username, "root");
        assert!(users[0].is_admin);
    }

    #[tokio::test]
    async fn test_ensure_initial_admin_skips_when_users_exist() {
        let repo = MockRepo::new();
        repo.create_user("existing", "hash", false, true)
            .await
            .unwrap();
        ensure_initial_admin(&repo).await.unwrap();
        let users = repo.list_users().await.unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].username, "existing");
    }

    #[tokio::test]
    async fn test_login_success() {
        let repo = MockRepo::new();
        let hash = hash_password("pass").unwrap();
        repo.create_user("alice", &hash, false, true).await.unwrap();
        let session = login(&repo, "alice", "pass").await.unwrap();
        assert!(!session.token.is_empty());
    }

    #[tokio::test]
    async fn test_login_wrong_password() {
        let repo = MockRepo::new();
        let hash = hash_password("pass").unwrap();
        repo.create_user("alice", &hash, false, true).await.unwrap();
        let err = login(&repo, "alice", "wrong").await.unwrap_err();
        assert!(matches!(err, AppError::Unauthorized));
    }

    #[tokio::test]
    async fn test_login_unapproved_user() {
        let repo = MockRepo::new();
        let hash = hash_password("pass").unwrap();
        repo.create_user("bob", &hash, false, false).await.unwrap();
        let err = login(&repo, "bob", "pass").await.unwrap_err();
        assert!(matches!(err, AppError::Forbidden));
    }

    #[tokio::test]
    async fn test_register_user_success() {
        let repo = MockRepo::new();
        repo.set_setting("local_users_enabled", "true")
            .await
            .unwrap();
        register_user(&repo, "newuser", "pass123").await.unwrap();
        let user = repo.find_user("newuser").await.unwrap().unwrap();
        assert_eq!(user.username, "newuser");
    }

    #[tokio::test]
    async fn test_register_user_duplicate() {
        let repo = MockRepo::new();
        repo.set_setting("local_users_enabled", "true")
            .await
            .unwrap();
        register_user(&repo, "dup", "pass").await.unwrap();
        let err = register_user(&repo, "dup", "pass").await.unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn test_change_password() {
        let repo = MockRepo::new();
        let hash = hash_password("old").unwrap();
        let user = repo.create_user("u", &hash, false, true).await.unwrap();
        change_password(&repo, &user, None, "old", "new")
            .await
            .unwrap();
        login(&repo, "u", "new").await.unwrap();
    }
}
