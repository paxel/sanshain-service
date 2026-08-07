//! Postgres repository tests for message-level AsyncAPI channel contracts
//! (ai/improvements.md item #20, reworked by ADR-0003). Contracts are keyed
//! globally by `(channel, message_name)` — Kafka topic names share one
//! namespace, so there is no branch (and no version) in the key. Mirrors the
//! SQLite unit tests in `src/infrastructure/sqlite_repository.rs`, run against
//! a real Postgres via testcontainers so the `ON CONFLICT` upsert is exercised
//! on the actual backend.

use sanshain_service::application::spec_service::{self, ProvideSpecParams};
use sanshain_service::domain::models::{ApiType, ChannelMessageContract, Stability};
use sanshain_service::domain::ports::SpecRepository;
use sanshain_service::infrastructure::postgres_repository::PostgresSpecRepository;
use sqlx::postgres::PgPoolOptions;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

fn contract(channel: &str, message: &str, owner: i64, payload: &str) -> ChannelMessageContract {
    ChannelMessageContract {
        channel: channel.to_string(),
        message_name: message.to_string(),
        owner_service_id: owner,
        payload_yaml: payload.to_string(),
    }
}

// `cfg(test)` is always true in this crate; the attribute marks the helper as
// test code so clippy's `allow-*-in-tests` exemptions apply to it.
#[cfg(test)]
async fn repo_and_container() -> (
    PostgresSpecRepository,
    testcontainers::ContainerAsync<Postgres>,
) {
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
    (repo, container)
}

#[tokio::test]
async fn postgres_channel_message_contract_lifecycle() {
    let (repo, _container) = repo_and_container().await;

    // Owners must reference real services (FK is enforced on Postgres).
    let a = repo.ensure_service("producer-a").await.unwrap();
    let b = repo.ensure_service("producer-b").await.unwrap();

    // Roundtrip on the global (channel, message_name) key.
    let c = contract("orders", "OrderPlaced", a, "type: object");
    repo.upsert_channel_message_contract(&c).await.unwrap();
    assert_eq!(
        repo.get_channel_message_contract("orders", "OrderPlaced")
            .await
            .unwrap(),
        Some(c)
    );

    // Missing key -> None.
    assert_eq!(
        repo.get_channel_message_contract("orders", "Nope")
            .await
            .unwrap(),
        None
    );

    // Upsert on the same key replaces owner and payload, no duplicate row.
    repo.upsert_channel_message_contract(&contract("orders", "OrderPlaced", b, "v2"))
        .await
        .unwrap();
    let got = repo
        .get_channel_message_contract("orders", "OrderPlaced")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.owner_service_id, b);
    assert_eq!(got.payload_yaml, "v2");
    assert_eq!(
        repo.list_channel_message_contracts().await.unwrap().len(),
        1
    );

    // List is global and sorted by (channel, message).
    repo.upsert_channel_message_contract(&contract("audit", "Logged", a, "y"))
        .await
        .unwrap();
    let all = repo.list_channel_message_contracts().await.unwrap();
    let keys: Vec<(String, String)> = all
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

    // Delete a single message.
    repo.delete_channel_message_contract("audit", "Logged")
        .await
        .unwrap();
    assert!(
        repo.get_channel_message_contract("audit", "Logged")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        repo.list_channel_message_contracts().await.unwrap().len(),
        1
    );
}

const ASYNCAPI_SPEC: &str = r#"asyncapi: '2.6.0'
info: { title: notifications, version: 1.0.0 }
channels:
  user.signup:
    publish:
      message:
        name: UserSignedUp
        payload:
          type: object
          properties:
            id: { type: string }
"#;

/// Contracts are registered on GA provides only: a snapshot claiming global
/// topic ownership would let one developer's WIP block another's release.
#[tokio::test]
async fn postgres_contracts_registered_on_ga_provides_only() {
    let (repo, _container) = repo_and_container().await;

    // Snapshot provide: no contract registered.
    spec_service::provide_spec(
        &repo,
        ProvideSpecParams {
            producername: "notify-svc",
            api_type: ApiType::AsyncApi,
            content: ASYNCAPI_SPEC,
            stability: Stability::Snapshot,
            dry_run: false,
            trunk: false,
            tag: None,
            caller: Some(sanshain_service::domain::permissions::Actor::test_releaser()),
            expected_prior_hash: None,
        },
    )
    .await
    .unwrap();
    assert!(
        repo.list_channel_message_contracts()
            .await
            .unwrap()
            .is_empty(),
        "a snapshot provide must not register a channel contract"
    );

    // GA provide of the same document registers the message contract, owned by
    // the providing service.
    let ga_spec = ASYNCAPI_SPEC.replace("version: 1.0.0", "version: 1.1.0");
    spec_service::provide_spec(
        &repo,
        ProvideSpecParams {
            producername: "notify-svc",
            api_type: ApiType::AsyncApi,
            content: &ga_spec,
            stability: Stability::Ga,
            dry_run: false,
            trunk: false,
            tag: None,
            caller: Some(sanshain_service::domain::permissions::Actor::test_releaser()),
            expected_prior_hash: None,
        },
    )
    .await
    .unwrap();
    let contracts = repo.list_channel_message_contracts().await.unwrap();
    assert_eq!(contracts.len(), 1);
    assert_eq!(contracts[0].channel, "user.signup");
    assert_eq!(contracts[0].message_name, "UserSignedUp");
    let owner = repo.find_service("notify-svc").await.unwrap().unwrap();
    assert_eq!(contracts[0].owner_service_id, owner);
}
