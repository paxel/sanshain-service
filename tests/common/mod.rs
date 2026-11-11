//! Shared helpers for the Postgres integration suites.

use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

/// The running Postgres of a test; dropping it removes the container.
pub type PostgresContainer = ContainerAsync<GenericImage>;

/// The image the suites run against. CI pre-pulls the same tag
/// (`.github/workflows/ci.yml`, `quality.yml`).
const POSTGRES_IMAGE: (&str, &str) = ("postgres", "11-alpine");

/// Postgres with user, password and database `postgres`, and fsync off (a test
/// database needs no durability). The entrypoint logs "ready to accept
/// connections" once for its init server on stderr and once for the real one on
/// stdout; waiting for both skips the restart in between.
fn postgres_image() -> testcontainers::ContainerRequest<GenericImage> {
    GenericImage::new(POSTGRES_IMAGE.0, POSTGRES_IMAGE.1)
        .with_wait_for(WaitFor::message_on_stderr(
            "database system is ready to accept connections",
        ))
        .with_wait_for(WaitFor::message_on_stdout(
            "database system is ready to accept connections",
        ))
        .with_exposed_port(5432.tcp())
        .with_env_var("POSTGRES_DB", "postgres")
        .with_env_var("POSTGRES_USER", "postgres")
        .with_env_var("POSTGRES_PASSWORD", "postgres")
        .with_cmd(["-c", "fsync=off"])
}

/// Start a throwaway Postgres container, retrying transient testcontainers
/// failures.
///
/// When several `*_postgres_test` binaries run in parallel they can pull
/// `postgres:11-alpine` at the same moment; bollard occasionally aborts one of
/// the concurrent pull streams with "bytes remaining on stream". A pre-pull in
/// CI is not a guarantee — the retry loop that guards it can exhaust without
/// caching the image, and local runs pull on demand too. A few retries let the
/// first pull finish and cache the image so the rest find it present.
///
/// Deterministic: fixed attempt count and backoff, no randomness.
//
// `cfg(test)` is always true in this crate; the attribute marks the helper as
// test code so clippy's `allow-*-in-tests` exemptions (here, `panic`) apply.
#[cfg(test)]
pub async fn start_postgres() -> PostgresContainer {
    let mut last_err = String::new();
    for attempt in 1..=5 {
        match postgres_image().start().await {
            Ok(container) => return container,
            Err(err) => {
                last_err = err.to_string();
                eprintln!("postgres container start attempt {attempt}/5 failed: {last_err}");
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
        }
    }
    panic!("postgres container failed to start after 5 attempts: {last_err}");
}
