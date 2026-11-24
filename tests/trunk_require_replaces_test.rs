//! san-1018 on the in-memory repository and on SQLite; the Postgres run of
//! the same scenario lives in `postgres_test.rs`.

#[path = "common/trunk_require_replaces.rs"]
mod trunk_require_replaces;

use sanshain_service::application::mock_repo::MockRepo;
use sanshain_service::infrastructure::sqlite_repository::SqliteSpecRepository;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn mock_trunk_requires_replace_the_dependencies_on_their_producer() {
    let repo = MockRepo::new();
    trunk_require_replaces::trunk_requires_replace_the_dependencies_on_their_producer(&repo).await;
}

#[tokio::test]
async fn sqlite_trunk_requires_replace_the_dependencies_on_their_producer() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();
    trunk_require_replaces::trunk_requires_replace_the_dependencies_on_their_producer(&repo).await;
}
