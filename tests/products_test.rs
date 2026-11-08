//! Products (#76, ADR-0011). The repository round trip on SQLite; the
//! Postgres run of the same scenario lives in `postgres_test.rs`.

#[path = "common/product_membership.rs"]
mod product_membership;

use sanshain_service::infrastructure::sqlite_repository::SqliteSpecRepository;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn sqlite_round_trips_product_membership() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();
    product_membership::product_membership_round_trips(&repo).await;
}
