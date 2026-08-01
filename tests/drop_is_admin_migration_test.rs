//! The upgrade that removes the administrator flag.
//!
//! This is the single highest-risk migration in the role work: it drops the
//! column every authorisation decision used to rest on. If the grant it writes
//! first is wrong, or runs in the wrong order, every administrator on a live
//! instance loses access on upgrade — recoverable only through
//! `SANSHAIN_ROOT_USERS`. So it is exercised against seeded data rather than
//! assumed correct because it parses.

use sqlx::Row;
use sqlx::sqlite::SqlitePool;

// `cfg(test)` is always true in this crate; the attribute marks the helper as
// test code so clippy's `allow-*-in-tests` exemptions apply to it.
#[cfg(test)]
async fn apply(pool: &SqlitePool, migration: &str) {
    let sql = std::fs::read_to_string(format!(
        "src/infrastructure/migrations/sqlite/{}.sql",
        migration
    ))
    .unwrap_or_else(|e| panic!("read {migration}: {e}"));
    // The files hold several statements; sqlx executes them as one script.
    sqlx::raw_sql(&sql)
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("apply {migration}: {e}"));
}

#[tokio::test]
async fn existing_administrators_keep_their_access_across_the_upgrade() {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("in-memory database");

    apply(&pool, "20240430000000_initial_schema").await;

    // Two administrators and two ordinary users, as a live instance would have.
    for (name, is_admin) in [
        ("root", 1),
        ("second-admin", 1),
        ("plain", 0),
        ("also-plain", 0),
    ] {
        sqlx::query(
            "INSERT INTO users (username, password_hash, is_admin, approved) VALUES (?, 'x', ?, 1)",
        )
        .bind(name)
        .bind(is_admin)
        .execute(&pool)
        .await
        .unwrap_or_else(|e| panic!("seed {name}: {e}"));
    }

    apply(&pool, "20260731000000_roles_and_groups").await;

    // One administrator already has the grant, so the migration must not trip
    // over the primary key by writing it twice.
    sqlx::query("INSERT INTO user_roles (user_id, role) SELECT id, 'admin' FROM users WHERE username = 'root'")
        .execute(&pool)
        .await
        .expect("pre-existing grant");

    apply(&pool, "20260731000004_drop_is_admin").await;

    let admins: Vec<String> = sqlx::query(
        "SELECT u.username FROM users u JOIN user_roles r ON r.user_id = u.id
          WHERE r.role = 'admin' ORDER BY u.username",
    )
    .fetch_all(&pool)
    .await
    .expect("read grants")
    .into_iter()
    .map(|row| row.get::<String, _>("username"))
    .collect();

    assert_eq!(
        admins,
        vec!["root".to_string(), "second-admin".to_string()],
        "every administrator must hold the admin role afterwards, and no one else"
    );

    // The column is gone, so nothing can read it by accident.
    assert!(
        sqlx::query("SELECT is_admin FROM users LIMIT 1")
            .fetch_optional(&pool)
            .await
            .is_err(),
        "the column must be dropped, not merely ignored"
    );

    // The rest of the table survived the rebuild-or-drop.
    let count: i64 = sqlx::query("SELECT COUNT(*) AS n FROM users")
        .fetch_one(&pool)
        .await
        .expect("count")
        .get("n");
    assert_eq!(count, 4, "dropping the column must not lose rows");
}

/// An instance with no administrators at all must not gain one.
#[tokio::test]
async fn the_upgrade_does_not_invent_an_administrator() {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("in-memory database");

    apply(&pool, "20240430000000_initial_schema").await;
    sqlx::query(
        "INSERT INTO users (username, password_hash, is_admin, approved) VALUES ('plain', 'x', 0, 1)",
    )
    .execute(&pool)
    .await
    .expect("seed");

    apply(&pool, "20260731000000_roles_and_groups").await;
    apply(&pool, "20260731000004_drop_is_admin").await;

    let count: i64 = sqlx::query("SELECT COUNT(*) AS n FROM user_roles")
        .fetch_one(&pool)
        .await
        .expect("count")
        .get("n");
    assert_eq!(count, 0, "no flag set means no grant written");
}
