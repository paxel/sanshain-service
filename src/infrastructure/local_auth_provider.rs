use crate::domain::models::AuthenticatedUser;
use crate::domain::ports::{AuthProvider, AuthProviderError, SpecRepository};

/// Local authentication provider using Argon2 password hashing.
pub struct LocalAuthProvider<R: SpecRepository> {
    repo: R,
}

impl<R: SpecRepository> LocalAuthProvider<R> {
    pub fn new(repo: R) -> Self {
        Self { repo }
    }
}

impl<R: SpecRepository + 'static> AuthProvider for LocalAuthProvider<R> {
    async fn authenticate(
        &self,
        username: &str,
        password: &str,
    ) -> Result<AuthenticatedUser, AuthProviderError> {
        let user = self
            .repo
            .find_user(username)
            .await
            .map_err(|e| AuthProviderError::Internal(format!("{:?}", e)))?
            .ok_or(AuthProviderError::InvalidCredentials)?;

        let parsed_hash = argon2::PasswordHash::new(&user.password_hash)
            .map_err(|e| AuthProviderError::Internal(format!("hash parse error: {}", e)))?;

        use argon2::PasswordVerifier;
        argon2::Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .map_err(|_| AuthProviderError::InvalidCredentials)?;

        if !user.approved {
            return Err(AuthProviderError::InvalidCredentials);
        }

        Ok(AuthenticatedUser {
            username: user.username,
            is_admin: user.is_admin,
        })
    }

    async fn test_connection(&self) -> Result<(), AuthProviderError> {
        // Local auth is always available if the DB works.
        let _ = self
            .repo
            .user_count()
            .await
            .map_err(|e| AuthProviderError::ConnectionFailed(format!("{:?}", e)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::sqlite_repository::SqliteSpecRepository;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn setup_db() -> SqliteSpecRepository {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();

        let repo = SqliteSpecRepository::new(pool);
        repo.run_migrations().await.unwrap();
        repo
    }

    async fn seed_users(repo: &SqliteSpecRepository) {
        use argon2::{
            Argon2,
            password_hash::{PasswordHasher, SaltString, rand_core::OsRng},
        };
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let hash = argon2
            .hash_password("password123".as_bytes(), &salt)
            .unwrap()
            .to_string();

        repo.create_user("approved_user", &hash, false, true)
            .await
            .unwrap();
        repo.create_user("unapproved_user", &hash, false, false)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_authenticate_success() {
        let repo = setup_db().await;
        seed_users(&repo).await;
        let provider = LocalAuthProvider::new(repo);

        let result = provider.authenticate("approved_user", "password123").await;
        assert!(
            result.is_ok(),
            "Result failed with: {:?}",
            result.err().unwrap()
        );
        let user = result.unwrap();
        assert_eq!(user.username, "approved_user");
    }

    #[tokio::test]
    async fn test_authenticate_invalid_password() {
        let repo = setup_db().await;
        seed_users(&repo).await;
        let provider = LocalAuthProvider::new(repo);

        let result = provider
            .authenticate("approved_user", "wrongpassword")
            .await;
        assert!(matches!(result, Err(AuthProviderError::InvalidCredentials)));
    }

    #[tokio::test]
    async fn test_authenticate_user_not_found() {
        let repo = setup_db().await;
        seed_users(&repo).await;
        let provider = LocalAuthProvider::new(repo);

        let result = provider.authenticate("nonexistent", "password123").await;
        assert!(matches!(result, Err(AuthProviderError::InvalidCredentials)));
    }

    #[tokio::test]
    async fn test_authenticate_unapproved_user() {
        let repo = setup_db().await;
        seed_users(&repo).await;
        let provider = LocalAuthProvider::new(repo);

        let result = provider
            .authenticate("unapproved_user", "password123")
            .await;
        assert!(matches!(result, Err(AuthProviderError::InvalidCredentials)));
    }

    #[tokio::test]
    async fn test_connection_success() {
        let repo = setup_db().await;
        let provider = LocalAuthProvider::new(repo);

        let result = provider.test_connection().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_authenticate_hash_parse_error() {
        let repo = setup_db().await;
        repo.create_user("badhash", "not-a-valid-argon2-hash", false, true)
            .await
            .unwrap();
        let provider = LocalAuthProvider::new(repo);

        let result = provider.authenticate("badhash", "password123").await;
        assert!(matches!(result, Err(AuthProviderError::Internal(_))));
    }

    #[tokio::test]
    async fn test_connection_db_failure() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let repo = SqliteSpecRepository::new(pool);
        let provider = LocalAuthProvider::new(repo);

        let result = provider.test_connection().await;
        assert!(matches!(
            result,
            Err(AuthProviderError::ConnectionFailed(_))
        ));
    }

    #[tokio::test]
    async fn test_authenticate_db_failure() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let repo = SqliteSpecRepository::new(pool);
        let provider = LocalAuthProvider::new(repo);

        let result = provider.authenticate("approved_user", "password123").await;
        assert!(matches!(result, Err(AuthProviderError::Internal(_))));
    }
}
