//! Postgres repository tests for message-level AsyncAPI channel contracts
//! (ai/improvements.md item #20). Mirrors the SQLite unit tests in
//! `src/infrastructure/sqlite_repository.rs`, run against a real Postgres via
//! testcontainers so the `= ANY($1)` cleanup path and `ON CONFLICT` upsert are
//! exercised on the actual backend.

use sanshain_service::domain::models::ChannelMessageContract;
use sanshain_service::domain::ports::SpecRepository;
use sanshain_service::infrastructure::postgres_repository::PostgresSpecRepository;
use sqlx::postgres::PgPoolOptions;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

fn contract(
    branch: &str,
    channel: &str,
    message: &str,
    owner: i64,
    payload: &str,
) -> ChannelMessageContract {
    ChannelMessageContract {
        branch_name: branch.to_string(),
        channel: channel.to_string(),
        message_name: message.to_string(),
        owner_service_id: owner,
        payload_yaml: payload.to_string(),
    }
}

#[tokio::test]
async fn postgres_channel_message_contract_lifecycle() {
    let container = Postgres::default().start().await.unwrap();
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let db_url = format!("postgres://postgres:postgres@{}:{}/postgres", host, port);

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();
    let repo = PostgresSpecRepository::new(pool);
    repo.run_migrations().await.unwrap();

    // Owners must reference real services (FK is enforced on Postgres).
    let a = repo.ensure_service("producer-a").await.unwrap();
    let b = repo.ensure_service("producer-b").await.unwrap();

    // Roundtrip.
    let c = contract("main", "orders", "OrderPlaced", a, "type: object");
    repo.upsert_channel_message_contract(&c).await.unwrap();
    assert_eq!(
        repo.get_channel_message_contract("main", "orders", "OrderPlaced")
            .await
            .unwrap(),
        Some(c)
    );

    // Missing key -> None.
    assert_eq!(
        repo.get_channel_message_contract("main", "orders", "Nope")
            .await
            .unwrap(),
        None
    );

    // Upsert on the same key replaces owner and payload, no duplicate row.
    repo.upsert_channel_message_contract(&contract("main", "orders", "OrderPlaced", b, "v2"))
        .await
        .unwrap();
    let got = repo
        .get_channel_message_contract("main", "orders", "OrderPlaced")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.owner_service_id, b);
    assert_eq!(got.payload_yaml, "v2");
    assert_eq!(
        repo.list_channel_message_contracts("main")
            .await
            .unwrap()
            .len(),
        1
    );

    // List is branch-scoped and sorted by (channel, message).
    repo.upsert_channel_message_contract(&contract("main", "audit", "Logged", a, "y"))
        .await
        .unwrap();
    repo.upsert_channel_message_contract(&contract("feature", "orders", "OrderPlaced", a, "y"))
        .await
        .unwrap();
    let main = repo.list_channel_message_contracts("main").await.unwrap();
    let keys: Vec<(String, String)> = main
        .iter()
        .map(|c| (c.channel.clone(), c.message_name.clone()))
        .collect();
    assert_eq!(
        keys,
        vec![
            ("audit".to_string(), "Logged".to_string()),
            ("orders".to_string(), "OrderPlaced".to_string()),
        ]
    );
    assert_eq!(
        repo.list_channel_message_contracts("feature")
            .await
            .unwrap()
            .len(),
        1
    );

    // Delete a single message.
    repo.delete_channel_message_contract("main", "audit", "Logged")
        .await
        .unwrap();
    assert!(
        repo.get_channel_message_contract("main", "audit", "Logged")
            .await
            .unwrap()
            .is_none()
    );

    // Orphan cleanup keeps live branches, removes dead ones.
    let removed = repo
        .delete_orphaned_channel_message_contracts(&["main".to_string()])
        .await
        .unwrap();
    assert_eq!(removed, 1); // the "feature" row
    assert!(
        repo.list_channel_message_contracts("feature")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        repo.list_channel_message_contracts("main")
            .await
            .unwrap()
            .len(),
        1
    );

    // Empty live set removes everything (the `= ANY('{}')` path).
    let removed = repo
        .delete_orphaned_channel_message_contracts(&[])
        .await
        .unwrap();
    assert_eq!(removed, 1);
    assert!(
        repo.list_channel_message_contracts("main")
            .await
            .unwrap()
            .is_empty()
    );
}
