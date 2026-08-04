use sqlx::Row;
use sqlx::sqlite::SqlitePool;

const MIGRATIONS_DIR: &str = "src/infrastructure/migrations/sqlite";
const V2_MIGRATION: &str = "20260801000000_versions_replace_branches.sql";

/// Apply every SQLite migration file older than the 2.0 rework, in order —
/// the full 1.x-era schema a live instance would be on before upgrading.
// `cfg(test)` is always true in this crate; the attribute marks the helper as
// test code for clippy's `allow-unwrap-in-tests`.
#[cfg(test)]
async fn apply_pre_2_0_schema(pool: &SqlitePool) {
    let mut files: Vec<String> = std::fs::read_dir(MIGRATIONS_DIR)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .filter(|name| name.ends_with(".sql"))
        .collect();
    files.sort();
    for name in files {
        if name == V2_MIGRATION {
            continue;
        }
        assert!(
            name.as_str() < V2_MIGRATION,
            "a migration newer than the 2.0 rework appeared ({name}); teach this test about it"
        );
        let sql = std::fs::read_to_string(format!("{MIGRATIONS_DIR}/{name}")).unwrap();
        sqlx::raw_sql(&sql)
            .execute(pool)
            .await
            .unwrap_or_else(|e| panic!("migration {name} failed: {e}"));
    }
}

#[cfg(test)]
async fn table_names(pool: &SqlitePool) -> Vec<String> {
    sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.get::<String, _>(0))
        .collect()
}

/// The 2.0 migration seam (ADR-0003): branch-era spec data is dropped, the
/// version-line tables exist in their new shape, and users, settings and the
/// audit log survive with their data.
#[tokio::test]
async fn test_sqlite_migration_2_0_versions_replace_branches() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    apply_pre_2_0_schema(&pool).await;

    // Seed 1.x branch-era data across the tables the migration touches.
    sqlx::raw_sql(
        r#"
        INSERT INTO services (name, fallback_branch, onboarding) VALUES ('legacy-svc', 'main', 1);
        INSERT INTO branches (service_id, name) VALUES (1, 'main');
        INSERT INTO endpoints (branch_id, api_type, path, method, yaml_content)
            VALUES (1, 'openapi', '/x', 'GET', 'yaml');
        INSERT INTO clients (name) VALUES ('legacy-client');
        INSERT INTO dependencies (client_id, endpoint_id, api_type, requested_service_id,
                                  requested_branch_name, requested_path, requested_method)
            VALUES (1, 1, 'openapi', 1, 'main', '/x', 'GET');
        INSERT INTO pending_specs (service_id, branch, api_type, content, reason, submitted_by)
            VALUES (1, 'main', 'openapi', 'spec', 'breaking', 'ci');
        INSERT INTO users (username, password_hash, approved) VALUES ('alice', 'hash', 1);
        INSERT INTO audit_logs (timestamp, username, action, details, service, branch, action_type)
            VALUES ('2026-01-01T00:00:00Z', 'alice', 'PROVIDE_SPEC', 'branch-era entry',
                    'legacy-svc', 'main', 'WRITE');
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();

    // Run the 2.0 migration.
    let sql = std::fs::read_to_string(format!("{MIGRATIONS_DIR}/{V2_MIGRATION}")).unwrap();
    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();

    // The branch-era spec tables are gone…
    let tables = table_names(&pool).await;
    for dropped in [
        "branches",
        "protected_branches",
        "pending_specs",
        "service_spec_versions",
        "endpoint_versions",
        "endpoint_version_metadata",
    ] {
        assert!(
            !tables.contains(&dropped.to_string()),
            "table {dropped} must be dropped; present: {tables:?}"
        );
    }
    // …and the version-line tables exist in the new shape.
    for created in ["spec_versions", "endpoints", "dependencies"] {
        assert!(
            tables.contains(&created.to_string()),
            "table {created} must exist; present: {tables:?}"
        );
    }
    sqlx::query("SELECT service_id, api_type, major, minor, patch, stability FROM spec_versions")
        .fetch_all(&pool)
        .await
        .expect("spec_versions must have the version-line columns");
    sqlx::query("SELECT spec_version_id FROM endpoints")
        .fetch_all(&pool)
        .await
        .expect("endpoints must hang off a spec version now");
    assert!(
        sqlx::query("SELECT branch_id FROM endpoints")
            .fetch_all(&pool)
            .await
            .is_err(),
        "the branch-era endpoints shape must be gone"
    );
    sqlx::query("SELECT spec_version_id FROM dependencies")
        .fetch_all(&pool)
        .await
        .expect("dependencies must reference a spec version now");
    assert!(
        sqlx::query("SELECT requested_branch_name FROM dependencies")
            .fetch_all(&pool)
            .await
            .is_err(),
        "the branch-era dependencies shape must be gone"
    );
    // Old spec data is dropped, not migrated: the new tables start empty.
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM endpoints")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count.0, 0);

    // Services survive, but the branch-era columns do not.
    let name: (String,) = sqlx::query_as("SELECT name FROM services")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(name.0, "legacy-svc");
    for column in ["fallback_branch", "onboarding"] {
        assert!(
            sqlx::query(&format!("SELECT {column} FROM services"))
                .fetch_all(&pool)
                .await
                .is_err(),
            "services.{column} must be dropped"
        );
    }

    // Users survive with their data.
    let user: (String, bool) = sqlx::query_as("SELECT username, approved FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(user, ("alice".to_string(), true));

    // Settings: branch cleanup gives way to use-based snapshot expiry.
    let branch_age: Option<(String,)> =
        sqlx::query_as("SELECT value FROM settings WHERE key = 'branch_max_age_days'")
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert_eq!(branch_age, None, "branch_max_age_days must be deleted");
    let snapshot_age: (String,) =
        sqlx::query_as("SELECT value FROM settings WHERE key = 'snapshot_max_age_days'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(snapshot_age.0, "30");

    // The audit log survives: `branch` is renamed to `version`, data intact.
    let entry: (String, String, String) =
        sqlx::query_as("SELECT username, details, version FROM audit_logs")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        entry,
        (
            "alice".to_string(),
            "branch-era entry".to_string(),
            "main".to_string()
        )
    );
    assert!(
        sqlx::query("SELECT branch FROM audit_logs")
            .fetch_all(&pool)
            .await
            .is_err(),
        "audit_logs.branch must be renamed away"
    );
}

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
