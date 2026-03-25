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
        let user = self.repo.find_user(username).await
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
        let _ = self.repo.user_count().await
            .map_err(|e| AuthProviderError::ConnectionFailed(format!("{:?}", e)))?;
        Ok(())
    }
}
