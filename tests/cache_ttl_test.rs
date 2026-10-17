//! Multi-replica safety net for the data caches (#49): two cached
//! repositories over one database stand in for two pods sharing PostgreSQL.
//! A write through one replica is invisible to the other until the time to
//! live elapses, visible after it, and visible on the writer at once.

use sanshain_service::domain::models::{ApiType, EndpointRecord, SemVer, Stability};
use sanshain_service::domain::ports::{SpecRepository, UpsertSpecVersion};
use sanshain_service::infrastructure::cached_repository::CachedSpecRepository;
use sanshain_service::infrastructure::database::DatabaseRepo;
use sanshain_service::infrastructure::sqlite_repository::SqliteSpecRepository;
use sqlx::sqlite::SqlitePoolOptions;
use std::time::Duration;

const TTL: Duration = Duration::from_millis(300);

/// Long enough for the TTL to have elapsed on every moka clock read.
async fn wait_past_ttl() {
    tokio::time::sleep(TTL * 2).await;
}

// `cfg(test)` is always true in this crate; the attribute marks the helpers
// as test code for clippy's `allow-unwrap-in-tests`.
/// Two cached repositories ("replicas") over one shared in-memory database.
/// The database is named per test so parallel tests never share one.
#[cfg(test)]
async fn two_replicas(db_name: &str) -> (CachedSpecRepository, CachedSpecRepository) {
    let url = format!("sqlite:file:{db_name}?mode=memory&cache=shared");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    let repo = SqliteSpecRepository::new(pool.clone());
    repo.run_migrations().await.unwrap();
    let writer = CachedSpecRepository::with_data_ttl(DatabaseRepo::Sqlite(repo), 256, TTL);
    let reader = CachedSpecRepository::with_data_ttl(
        DatabaseRepo::Sqlite(SqliteSpecRepository::new(pool)),
        256,
        TTL,
    );
    (writer, reader)
}

fn endpoint(path: &str, method: &str, yaml: &str) -> EndpointRecord {
    EndpointRecord {
        id: None,
        api_type: ApiType::OpenApi,
        path: path.to_string(),
        normalized_path: sanshain_service::openapi::normalize_path(path),
        method: method.to_string(),
        yaml_content: yaml.to_string(),
        deprecated: false,
    }
}

#[cfg(test)]
async fn provide(
    repo: &CachedSpecRepository,
    service_id: i64,
    content: &str,
    content_hash: &str,
    endpoints: Vec<EndpointRecord>,
) -> i64 {
    repo.upsert_spec_version(UpsertSpecVersion {
        outbox: Vec::new(),
        service_id,
        api_type: ApiType::OpenApi,
        version: "1.0.0".parse().unwrap(),
        stability: Stability::Snapshot,
        content,
        content_hash,
        provided_by: "tester",
        expected_prior_hash: None,
        now_iso: "2026-01-01T00:00:00Z",
        endpoints,
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn endpoint_resolution_catches_up_within_ttl() {
    let (writer, reader) = two_replicas("ttl_endpoint").await;
    let sid = writer.ensure_service("svc").await.unwrap();
    let vid = provide(
        &writer,
        sid,
        "spec-1",
        "hash-1",
        vec![endpoint("/users/{id}", "GET", "yaml-1")],
    )
    .await;
    let resolve = |repo: &CachedSpecRepository| {
        let repo = repo.clone();
        async move {
            repo.find_endpoint(vid, ApiType::OpenApi, "/users/{id}", "GET")
                .await
                .unwrap()
                .map(|(_, yaml, _)| yaml)
        }
    };
    assert_eq!(resolve(&reader).await.as_deref(), Some("yaml-1"));

    provide(
        &writer,
        sid,
        "spec-2",
        "hash-2",
        vec![endpoint("/users/{id}", "GET", "yaml-2")],
    )
    .await;
    assert_eq!(
        resolve(&writer).await.as_deref(),
        Some("yaml-2"),
        "the writing replica sees its own write immediately"
    );
    assert_eq!(
        resolve(&reader).await.as_deref(),
        Some("yaml-1"),
        "the other replica still serves the cached endpoint before the TTL"
    );

    wait_past_ttl().await;
    assert_eq!(
        resolve(&reader).await.as_deref(),
        Some("yaml-2"),
        "the other replica serves the new endpoint after the TTL"
    );
}

#[tokio::test]
async fn version_endpoints_catch_up_within_ttl() {
    let (writer, reader) = two_replicas("ttl_version_endpoints").await;
    let sid = writer.ensure_service("svc").await.unwrap();
    let vid = provide(
        &writer,
        sid,
        "spec-1",
        "hash-1",
        vec![endpoint("/users/{id}", "GET", "yaml-1")],
    )
    .await;
    assert_eq!(
        reader.get_endpoints_for_version(vid).await.unwrap().len(),
        1
    );

    provide(
        &writer,
        sid,
        "spec-2",
        "hash-2",
        vec![
            endpoint("/users/{id}", "GET", "yaml-2"),
            endpoint("/orders", "POST", "yaml-3"),
        ],
    )
    .await;
    assert_eq!(
        writer.get_endpoints_for_version(vid).await.unwrap().len(),
        2
    );
    assert_eq!(
        reader.get_endpoints_for_version(vid).await.unwrap().len(),
        1,
        "stale endpoint list before the TTL"
    );

    wait_past_ttl().await;
    assert_eq!(
        reader.get_endpoints_for_version(vid).await.unwrap().len(),
        2,
        "fresh endpoint list after the TTL"
    );
}

#[tokio::test]
async fn spec_content_catches_up_within_ttl() {
    let (writer, reader) = two_replicas("ttl_spec_content").await;
    let sid = writer.ensure_service("svc").await.unwrap();
    let vid = provide(&writer, sid, "spec-1", "hash-1", vec![]).await;
    assert_eq!(
        reader.get_spec_content(vid).await.unwrap().as_deref(),
        Some("spec-1")
    );

    provide(&writer, sid, "spec-2", "hash-2", vec![]).await;
    assert_eq!(
        writer.get_spec_content(vid).await.unwrap().as_deref(),
        Some("spec-2")
    );
    assert_eq!(
        reader.get_spec_content(vid).await.unwrap().as_deref(),
        Some("spec-1"),
        "stale Snapshot content before the TTL"
    );

    wait_past_ttl().await;
    assert_eq!(
        reader.get_spec_content(vid).await.unwrap().as_deref(),
        Some("spec-2"),
        "current Snapshot content after the TTL"
    );
}

#[tokio::test]
async fn report_catches_up_within_ttl() {
    let (writer, reader) = two_replicas("ttl_report").await;
    let sid = writer.ensure_service("svc").await.unwrap();
    provide(
        &writer,
        sid,
        "spec-1",
        "hash-1",
        vec![endpoint("/users/{id}", "GET", "yaml-1")],
    )
    .await;
    assert_eq!(reader.get_report().await.unwrap().unused_endpoints.len(), 1);

    provide(
        &writer,
        sid,
        "spec-2",
        "hash-2",
        vec![
            endpoint("/users/{id}", "GET", "yaml-2"),
            endpoint("/orders", "POST", "yaml-3"),
        ],
    )
    .await;
    assert_eq!(writer.get_report().await.unwrap().unused_endpoints.len(), 2);
    assert_eq!(
        reader.get_report().await.unwrap().unused_endpoints.len(),
        1,
        "stale report before the TTL"
    );

    wait_past_ttl().await;
    assert_eq!(
        reader.get_report().await.unwrap().unused_endpoints.len(),
        2,
        "fresh report after the TTL"
    );
}

#[tokio::test]
async fn producer_lists_catch_up_within_ttl() {
    let (writer, reader) = two_replicas("ttl_producers").await;
    writer.ensure_service("svc-a").await.unwrap();
    assert_eq!(reader.list_producers().await.unwrap(), vec!["svc-a"]);
    assert_eq!(reader.list_producers_detailed().await.unwrap().len(), 1);

    writer.ensure_service("svc-b").await.unwrap();
    assert_eq!(writer.list_producers().await.unwrap().len(), 2);
    assert_eq!(writer.list_producers_detailed().await.unwrap().len(), 2);
    assert_eq!(
        reader.list_producers().await.unwrap(),
        vec!["svc-a"],
        "stale Producer names before the TTL"
    );
    assert_eq!(
        reader.list_producers_detailed().await.unwrap().len(),
        1,
        "stale Producer summaries before the TTL"
    );

    wait_past_ttl().await;
    assert_eq!(
        reader.list_producers().await.unwrap(),
        vec!["svc-a", "svc-b"],
        "fresh Producer names after the TTL"
    );
    assert_eq!(
        reader.list_producers_detailed().await.unwrap().len(),
        2,
        "fresh Producer summaries after the TTL"
    );
}

#[tokio::test]
async fn version_metadata_catches_up_within_ttl() {
    let (writer, reader) = two_replicas("ttl_version_metadata").await;
    let sid = writer.ensure_service("svc").await.unwrap();
    let version: SemVer = "1.0.0".parse().unwrap();
    provide(&writer, sid, "spec-1", "hash-1", vec![]).await;
    let hash_via = |repo: &CachedSpecRepository| {
        let repo = repo.clone();
        async move {
            repo.find_spec_version(sid, ApiType::OpenApi, version)
                .await
                .unwrap()
                .map(|meta| meta.content_hash)
        }
    };
    assert_eq!(hash_via(&reader).await.as_deref(), Some("hash-1"));

    provide(&writer, sid, "spec-2", "hash-2", vec![]).await;
    assert_eq!(hash_via(&writer).await.as_deref(), Some("hash-2"));
    assert_eq!(
        hash_via(&reader).await.as_deref(),
        Some("hash-1"),
        "stale version metadata before the TTL"
    );

    wait_past_ttl().await;
    assert_eq!(
        hash_via(&reader).await.as_deref(),
        Some("hash-2"),
        "fresh version metadata after the TTL"
    );
}
