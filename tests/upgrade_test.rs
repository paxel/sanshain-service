use sqlx::sqlite::SqlitePool;

#[tokio::test]
async fn test_sqlite_migration_upgrade_1_0_1() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();

    // 1. Apply initial schema (1.0.0)
    let initial_schema = std::fs::read_to_string(
        "src/infrastructure/migrations/sqlite/20240430000000_initial_schema.sql",
    )
    .unwrap();
    sqlx::query(&initial_schema).execute(&pool).await.unwrap();

    // 2. Seed some 1.0.0 data
    sqlx::query("INSERT INTO services (name) VALUES ('svc1')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO shared_contracts (branch_name, api_type, path, method, source_yaml, current_yaml) VALUES ('main', 'openapi', '/test', 'GET', 'v1', 'v1')")
        .execute(&pool).await.unwrap();

    // 3. Run migration
    let upgrade_schema = std::fs::read_to_string(
        "src/infrastructure/migrations/sqlite/20240502000000_scope_shared_contracts.sql",
    )
    .unwrap();
    sqlx::query(&upgrade_schema).execute(&pool).await.unwrap();

    // 4. Verify new schema has branch_name AND service_id columns
    let row = sqlx::query("SELECT branch_name, service_id FROM shared_contracts LIMIT 1")
        .fetch_optional(&pool)
        .await;
    assert!(
        row.is_ok(),
        "Query for branch_name + service_id failed: {:?}",
        row.err()
    );

    // 5. Verify branch_id does NOT exist (old v1.0.1 column)
    let err_row = sqlx::query("SELECT branch_id FROM shared_contracts LIMIT 1")
        .fetch_optional(&pool)
        .await;
    assert!(
        err_row.is_err(),
        "Query for branch_id succeeded - old column still exists!"
    );

    // 6. Verify the unique constraint includes service_id
    sqlx::query("INSERT INTO services (name) VALUES ('svc-a')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO services (name) VALUES ('svc-b')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO shared_contracts (branch_name, service_id, api_type, path, method, source_yaml, current_yaml) VALUES ('main', 2, 'openapi', '/ep', 'GET', 'y1', 'y1')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO shared_contracts (branch_name, service_id, api_type, path, method, source_yaml, current_yaml) VALUES ('main', 3, 'openapi', '/ep', 'GET', 'y2', 'y2')")
        .execute(&pool).await.unwrap();
    // Both inserts succeed — different service_id means no conflict
}
