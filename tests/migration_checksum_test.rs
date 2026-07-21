use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

// `cfg(test)` is always true in this crate; the attribute marks the helper as
// test code for clippy's `allow-panic-in-tests`.
#[cfg(test)]
fn calculate_file_sha256(path: &Path) -> String {
    let content =
        fs::read(path).unwrap_or_else(|e| panic!("Failed to read file {:?}: {}", path, e));
    let mut hasher = Sha256::new();
    hasher.update(&content);
    let result = hasher.finalize();
    hex::encode(result)
}

#[test]
fn test_sqlite_migration_checksums() {
    let expected = [
        (
            "src/infrastructure/migrations/sqlite/20240430000000_initial_schema.sql",
            "e54701a681da8c557769b02fdd6fb51052b77e2f9b35c70e1a4790ff21e19c6b",
        ),
        (
            "src/infrastructure/migrations/sqlite/20240502000000_scope_shared_contracts.sql",
            "b2816ca4dc5399998f783e7f252f26ac28951944b0bd9da1b1321ffc1e996ba2",
        ),
        (
            "src/infrastructure/migrations/sqlite/20240503000000_audit_log.sql",
            "08875062722a955e55b8c74381210eef1fc4141ab0aa4f753d03108bac824ce1",
        ),
        (
            "src/infrastructure/migrations/sqlite/20240504000000_add_version_metadata.sql",
            "49a28a1165e829615ad34073448f006f3bec789edccd150daec06761b2a72c7f",
        ),
        (
            "src/infrastructure/migrations/sqlite/20240505000000_user_favorites.sql",
            "5f752fcc790dd0bb0495bef4cf0de9a78dd38f69bc25ca82881c5b579c11f702",
        ),
        (
            "src/infrastructure/migrations/sqlite/20240506000000_add_auth_mode_default.sql",
            "5bea1ded02d78034ab7349a245e131df0d29925ded409f3245819a7339b8aaa3",
        ),
    ];

    for (file_path, expected_hash) in expected {
        let path = Path::new(file_path);
        let actual_hash = calculate_file_sha256(path);
        assert_eq!(
            actual_hash, expected_hash,
            "Checksum mismatch for SQLite migration file: {}. If you intend to make schema changes, please create a new migration SQL file instead of modifying existing ones inline to prevent breaking updates for existing users.",
            file_path
        );
    }
}

#[test]
fn test_postgres_migration_checksums() {
    let expected = [
        (
            "src/infrastructure/migrations/postgres/20240430000000_initial_schema.sql",
            "8b94a116f3042cecd3b1aac7a78e4545144c0754019bb27bdc2f08f5efc08fa7",
        ),
        (
            "src/infrastructure/migrations/postgres/20240502000000_scope_shared_contracts.sql",
            "d2caa70eda6deedb916aa989a3ce054223e282f4bfed6abfcb39a9d9f490e777",
        ),
        (
            "src/infrastructure/migrations/postgres/20240503000000_audit_log.sql",
            "1338cd5bc68c3a01a0a9370143ae3bd7be6f1060670176a4526332f6bc5bd030",
        ),
        (
            "src/infrastructure/migrations/postgres/20240504000000_add_version_metadata.sql",
            "ebb448e9947e46d249b93f1d95ebc2333ebe98d89723959b51137041e3a433c2",
        ),
        (
            "src/infrastructure/migrations/postgres/20240505000000_user_favorites.sql",
            "d324aafa8cecc6dab1b48bee2ba51333d2012181b9efd5ce1e6491329781e80f",
        ),
        (
            "src/infrastructure/migrations/postgres/20240506000000_add_auth_mode_default.sql",
            "be85a40f175779ccba8dcad0b14a30c7af9ab5270bdcb78fd30249dd4f6634eb",
        ),
    ];

    for (file_path, expected_hash) in expected {
        let path = Path::new(file_path);
        let actual_hash = calculate_file_sha256(path);
        assert_eq!(
            actual_hash, expected_hash,
            "Checksum mismatch for PostgreSQL migration file: {}. If you intend to make schema changes, please create a new migration SQL file instead of modifying existing ones inline to prevent breaking updates for existing users.",
            file_path
        );
    }
}
