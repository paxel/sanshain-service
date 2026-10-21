//! Expired sessions and API tokens on SQLite; the Postgres run of the same
//! scenario lives in `postgres_test.rs`.

#[path = "common/credential_expiry.rs"]
mod credential_expiry;

use sanshain_service::infrastructure::sqlite_repository::SqliteSpecRepository;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn sqlite_refuses_and_cleans_expired_credentials() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();
    credential_expiry::expired_credentials_are_refused_and_cleaned(&repo).await;
}
