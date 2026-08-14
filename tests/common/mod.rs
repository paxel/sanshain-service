//! Shared helpers for the Postgres integration suites.

use testcontainers::ContainerAsync;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

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
pub async fn start_postgres() -> ContainerAsync<Postgres> {
    let mut last_err = String::new();
    for attempt in 1..=5 {
        match Postgres::default().start().await {
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
