use sha2::{Digest, Sha256};
use std::collections::HashSet;
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
    hex::encode(hasher.finalize())
}

/// An applied migration must never change: existing installations replay the
/// file they already ran, so an inline edit silently diverges their schema
/// from a fresh install's. Schema changes go in a *new* migration.
///
/// The completeness check matters as much as the hashes. This list covered
/// only the six original 2024 files for a long while, so every migration added
/// since — the whole 2.x schema — was unprotected while the test still passed.
/// A file on disk with no entry here is now a failure, not a silent gap.
#[cfg(test)]
fn verify_migrations(dir: &str, expected: &[(&str, &str)]) {
    for (name, expected_hash) in expected {
        let path = format!("{dir}/{name}");
        assert_eq!(
            calculate_file_sha256(Path::new(&path)),
            *expected_hash,
            "Checksum mismatch for migration {path}. An applied migration must not be edited \
             in place — create a new migration file instead, or existing installations end up \
             on a different schema than fresh ones."
        );
    }

    let listed: HashSet<&str> = expected.iter().map(|(name, _)| *name).collect();
    let mut unlisted: Vec<String> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("Failed to read {dir}: {e}"))
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .filter(|name| name.ends_with(".sql") && !listed.contains(name.as_str()))
        .collect();
    unlisted.sort();
    assert!(
        unlisted.is_empty(),
        "migration(s) in {dir} with no checksum entry: {unlisted:?} — add them here so they \
         are frozen too, otherwise this test passes while protecting nothing."
    );
}

const SQLITE_MIGRATIONS: &[(&str, &str)] = &[
    (
        "20240430000000_initial_schema.sql",
        "e54701a681da8c557769b02fdd6fb51052b77e2f9b35c70e1a4790ff21e19c6b",
    ),
    (
        "20240502000000_scope_shared_contracts.sql",
        "b2816ca4dc5399998f783e7f252f26ac28951944b0bd9da1b1321ffc1e996ba2",
    ),
    (
        "20240503000000_audit_log.sql",
        "08875062722a955e55b8c74381210eef1fc4141ab0aa4f753d03108bac824ce1",
    ),
    (
        "20240504000000_add_version_metadata.sql",
        "49a28a1165e829615ad34073448f006f3bec789edccd150daec06761b2a72c7f",
    ),
    (
        "20240505000000_user_favorites.sql",
        "5f752fcc790dd0bb0495bef4cf0de9a78dd38f69bc25ca82881c5b579c11f702",
    ),
    (
        "20240506000000_add_auth_mode_default.sql",
        "5bea1ded02d78034ab7349a245e131df0d29925ded409f3245819a7339b8aaa3",
    ),
    (
        "20240623000000_add_deprecated_to_endpoints.sql",
        "f3ff7d174e3a1abbd3af2aff0105a0abd4dcd6fa56cf3bc92b577977acdc3ad8",
    ),
    (
        "20240624000000_add_semver_to_spec_versions.sql",
        "0129758bf9704447e9f9bfce0f9defaa010cc4a626f1d9d0a3d81ad503c65d42",
    ),
    (
        "20240625000000_enhance_audit_logs.sql",
        "f7c5cf2b6faf59ed2ff203dffb16901e35a4d4a7e44d8e428d21a352da050720",
    ),
    (
        "20240626000000_fix_audit_types.sql",
        "b83f0f6049e44607a1617d4448bfba84a6c1b77cdd716626385dc5ea16c065c4",
    ),
    (
        "20240626000001_unmask_usernames.sql",
        "5a29e129fd299fae91fd07f1e7a746f50fd9c188520045c483216d44e1276209",
    ),
    (
        "20260704000000_drop_shared_contracts.sql",
        "f46fbcab56d32e2c671b0f9aa8e9ae35a60dbb1c1f6c1c0c98ba110a7c01becd",
    ),
    (
        "20260704000001_channel_message_contracts.sql",
        "4e7d53509ce3656ec057b00b16cbc3f82c44548f664d60a1dc135ebb2e5431eb",
    ),
    (
        "20260726000000_source_protected_branch.sql",
        "034ba2075f52efa435947bdefb8f8f2d3cd76aed4d07800e20283856f21cb0af",
    ),
    (
        "20260731000000_roles_and_groups.sql",
        "df93084f89e51b2fb302fad9d6ee6f733f4e5b7cb25fefe4149c37c0eac115c0",
    ),
    (
        "20260731000001_producer_maintainers.sql",
        "886d6c644cdd848085d07303ddee3d6031db44a3c9834a61c1cf35873828d89e",
    ),
    (
        "20260731000002_producer_onboarding.sql",
        "b2a7847ac4316582d3f1c35b92ff35b72d6626523b7e1ca749faf05e1e3c4533",
    ),
    (
        "20260731000003_pending_specs.sql",
        "4e48b1ff3e57627b2ad69e48147b477789d71473b3808dfead07464d7c934327",
    ),
    (
        "20260731000004_drop_is_admin.sql",
        "d0f47c68be25b7eaecccf0048931c1b606933118a07f5e4a50a8be5197594a0f",
    ),
    (
        "20260801000000_versions_replace_branches.sql",
        "1aee14953a862ad9f3b1c0d3e7b71f85100ae329f1013474a906eff83e9b0b23",
    ),
    (
        "20260807000000_trunk_provided_marker.sql",
        "2384090afd819036553dbbbd7147fa642093dec06f1bcba7311558d38ca6abcb",
    ),
    (
        "20260807000001_trunk_dependencies.sql",
        "f2be324bc76959831b42cf150f745f471fd9b33ffcb656726f88bb27e0837e78",
    ),
    (
        "20260807000002_sanshain_branches.sql",
        "0ae98f8a7bfc58aa68b8a1a885cdccd82b2cd6f8f9e26c75124ab07996c77460",
    ),
    (
        "20260807000003_branch_member_versions.sql",
        "6b2a5750a346ae2caafb83a66567af3a15ae9c06d7c9f44a49edd633dca14cf4",
    ),
    (
        "20260807000004_audit_stream.sql",
        "c09f2c3900312edbcc710173a203e8db31a03d2396cbbd542fccc9057831c821",
    ),
    (
        "20260808000000_unique_open_pin_rows.sql",
        "d15448d715996ef3bbfc466854ccaefdb9ad564ead750243a126a8d0eb2a4d6b",
    ),
];

const POSTGRES_MIGRATIONS: &[(&str, &str)] = &[
    (
        "20240430000000_initial_schema.sql",
        "8b94a116f3042cecd3b1aac7a78e4545144c0754019bb27bdc2f08f5efc08fa7",
    ),
    (
        "20240502000000_scope_shared_contracts.sql",
        "d2caa70eda6deedb916aa989a3ce054223e282f4bfed6abfcb39a9d9f490e777",
    ),
    (
        "20240503000000_audit_log.sql",
        "1338cd5bc68c3a01a0a9370143ae3bd7be6f1060670176a4526332f6bc5bd030",
    ),
    (
        "20240504000000_add_version_metadata.sql",
        "ebb448e9947e46d249b93f1d95ebc2333ebe98d89723959b51137041e3a433c2",
    ),
    (
        "20240505000000_user_favorites.sql",
        "d324aafa8cecc6dab1b48bee2ba51333d2012181b9efd5ce1e6491329781e80f",
    ),
    (
        "20240506000000_add_auth_mode_default.sql",
        "be85a40f175779ccba8dcad0b14a30c7af9ab5270bdcb78fd30249dd4f6634eb",
    ),
    (
        "20240623000000_add_deprecated_to_endpoints.sql",
        "25fa100f21c3da9ea6c879df6ac32d7d0be49c0fd326fbcc634edfca014aa9a2",
    ),
    (
        "20240624000000_add_semver_to_spec_versions.sql",
        "e28ee6374d589aaeed103f112055ccd3fc8bef3f07b929a29660fe7cc46ab5a1",
    ),
    (
        "20240625000000_enhance_audit_logs.sql",
        "b27e88bc76935098210bdd2a1eceeb162c94c44c1ce663f3ca333daf4a0e2f23",
    ),
    (
        "20240626000000_fix_audit_types.sql",
        "b83f0f6049e44607a1617d4448bfba84a6c1b77cdd716626385dc5ea16c065c4",
    ),
    (
        "20240626000001_unmask_usernames.sql",
        "5a29e129fd299fae91fd07f1e7a746f50fd9c188520045c483216d44e1276209",
    ),
    (
        "20260704000000_drop_shared_contracts.sql",
        "f46fbcab56d32e2c671b0f9aa8e9ae35a60dbb1c1f6c1c0c98ba110a7c01becd",
    ),
    (
        "20260704000001_channel_message_contracts.sql",
        "e123c90f80fae9242d968662a53482deb2ac90539ae66e0daea35d0277084957",
    ),
    (
        "20260726000000_source_protected_branch.sql",
        "034ba2075f52efa435947bdefb8f8f2d3cd76aed4d07800e20283856f21cb0af",
    ),
    (
        "20260731000000_roles_and_groups.sql",
        "029b2cef91f302a2fb0ec0d73ca2a64678d5630b160e845a1071469215b92fe9",
    ),
    (
        "20260731000001_producer_maintainers.sql",
        "c19327a9d1f1295c142c201ebd3ea744c82b10a1a2650194ee180de9cb079795",
    ),
    (
        "20260731000002_producer_onboarding.sql",
        "b2a7847ac4316582d3f1c35b92ff35b72d6626523b7e1ca749faf05e1e3c4533",
    ),
    (
        "20260731000003_pending_specs.sql",
        "d1cf303e13ef7078d816082ab2d2a3be52892361b29ca17e070772840a3a7f99",
    ),
    (
        "20260731000004_drop_is_admin.sql",
        "d0f47c68be25b7eaecccf0048931c1b606933118a07f5e4a50a8be5197594a0f",
    ),
    (
        "20260801000000_versions_replace_branches.sql",
        "e51c13c9691d573574cc9f97f17ab7efc8f7797518909302c1d5ec2b8528bf0c",
    ),
    (
        "20260807000000_trunk_provided_marker.sql",
        "2384090afd819036553dbbbd7147fa642093dec06f1bcba7311558d38ca6abcb",
    ),
    (
        "20260807000001_trunk_dependencies.sql",
        "d5186e1db98b55f9501067238560784d87c2d73390edf60b79c872c685eaf041",
    ),
    (
        "20260807000002_sanshain_branches.sql",
        "5137a7a9b85c1254ea78e5bb27e98088e44d03af5e9b0048f0d2e5aedb563c90",
    ),
    (
        "20260807000003_branch_member_versions.sql",
        "9a222bd22850be80da9d3932286756105ca754a6de25c2f7bfaa3996891155c2",
    ),
    (
        "20260807000004_audit_stream.sql",
        "c09f2c3900312edbcc710173a203e8db31a03d2396cbbd542fccc9057831c821",
    ),
    (
        "20260808000000_unique_open_pin_rows.sql",
        "d15448d715996ef3bbfc466854ccaefdb9ad564ead750243a126a8d0eb2a4d6b",
    ),
];

#[test]
fn test_sqlite_migration_checksums() {
    verify_migrations("src/infrastructure/migrations/sqlite", SQLITE_MIGRATIONS);
}

#[test]
fn test_postgres_migration_checksums() {
    verify_migrations(
        "src/infrastructure/migrations/postgres",
        POSTGRES_MIGRATIONS,
    );
}
