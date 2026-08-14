use sqlx::postgres::PgPoolOptions;
mod common;

#[tokio::test]
async fn test_postgres_migration_upgrade_1_0_1() {
    // 1. Start Postgres container
    let postgres_container = common::start_postgres().await;
    let host = postgres_container.get_host().await.unwrap();
    let port = postgres_container.get_host_port_ipv4(5432).await.unwrap();
    let db_url = format!("postgres://postgres:postgres@{}:{}/postgres", host, port);

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();

    // 1. Apply initial schema (1.0.0)
    let initial_schema = std::fs::read_to_string(
        "src/infrastructure/migrations/postgres/20240430000000_initial_schema.sql",
    )
    .unwrap();
    for statement in initial_schema.split(';') {
        let trimmed = statement.trim();
        if !trimmed.is_empty() {
            sqlx::query(trimmed)
                .execute(&pool)
                .await
                .unwrap_or_else(|_| panic!("Failed to execute statement: {}", trimmed));
        }
    }

    // 2. Seed some 1.0.0 data
    sqlx::query("INSERT INTO services (name) VALUES ('svc1')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO shared_contracts (branch_name, api_type, path, method, source_yaml, current_yaml) VALUES ('main', 'openapi', '/test', 'GET', 'v1', 'v1')")
        .execute(&pool).await.unwrap();

    // 3. Run migration
    let upgrade_schema = std::fs::read_to_string(
        "src/infrastructure/migrations/postgres/20240502000000_scope_shared_contracts.sql",
    )
    .unwrap();
    for statement in upgrade_schema.split(';') {
        let trimmed = statement.trim();
        if !trimmed.is_empty() {
            sqlx::query(trimmed)
                .execute(&pool)
                .await
                .unwrap_or_else(|_| panic!("Failed to execute statement: {}", trimmed));
        }
    }

    // 4. Verify new schema has branch_name AND service_id columns
    let row = sqlx::query("SELECT branch_name, service_id FROM shared_contracts LIMIT 1")
        .fetch_optional(&pool)
        .await;
    assert!(
        row.is_ok(),
        "Query for branch_name + service_id failed in Postgres: {:?}",
        row.err()
    );

    // 5. Verify branch_id does NOT exist (old v1.0.1 column)
    let err_row = sqlx::query("SELECT branch_id FROM shared_contracts LIMIT 1")
        .fetch_optional(&pool)
        .await;
    assert!(
        err_row.is_err(),
        "Query for branch_id succeeded in Postgres - old column still exists!"
    );
}
